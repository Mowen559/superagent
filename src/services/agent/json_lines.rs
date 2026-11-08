//! Shared driver for the **json** protocol family
//! (copilot / opencode / deveco / openclaw / pi / antigravity).
//!
//! Unlike stream-json these CLIs take the prompt on **argv** (no long-lived
//! stdin control channel) and each emits its own newline-delimited event
//! envelope — copilot uses `{type:"dotted.name", data:{...}}`, opencode uses
//! `{type, part:{...}}`, etc. The scanning loop, subprocess lifecycle, timeout,
//! stderr tail, and terminal-status derivation are identical; the per-CLI event
//! semantics live in a [`JsonSpec::map_event`] closure that each backend file
//! supplies. This keeps the driver protocol-shaped while backends stay thin.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use super::backend::{BackendError, BackendResult};
use super::proc::agent_command;
use super::types::{AgentMessage, AgentResult, AgentSession, ExecOptions, TokenUsage};

/// Mutable accumulation for a json-family run, updated by the per-backend
/// event mapper and read when deriving the final result.
#[derive(Default)]
pub struct JsonState {
    pub subagent_session: Option<String>,
    pub session_id: String,
    pub active_model: String,
    /// Latest COMPLETE assistant turn (the deliverable).
    pub output: String,
    /// Buffered streaming turn; only wins if the process died mid-turn.
    pub pending_delta: String,
    /// Set to "failed"/"timeout"/... by the mapper to force a terminal state.
    pub forced_status: Option<String>,
    pub forced_error: String,
    /// Only adapters with an explicit final marker set this. Descendants may
    /// otherwise keep the stdout pipe open after the worker has finished.
    pub terminal_received: bool,
    pub usage: HashMap<String, TokenUsage>,
}

impl JsonState {
    /// The deliverable: the partial turn if the process died streaming it,
    /// else the last complete turn (mirrors multica copilot `finalOutput`).
    pub fn final_output(&self) -> String {
        if !self.pending_delta.is_empty() {
            self.pending_delta.clone()
        } else {
            self.output.clone()
        }
    }
}

/// How a raw stdout line is handled when it does NOT parse as JSON. Most json
/// CLIs emit strict JSONL, but antigravity's `-p` print mode streams plain
/// text lines — those flow through `map_raw` instead of being dropped.
pub type RawLineMapper = Box<dyn Fn(&str, &mut JsonState) -> Vec<AgentMessage> + Send + Sync>;

/// Per-backend knobs for the json driver.
pub struct JsonSpec {
    /// CLI command name.
    pub command: String,
    /// Full argv after the command name, including the prompt where the CLI
    /// takes it positionally / as a flag value.
    pub args: Vec<String>,
    /// Maps one parsed event line into zero or more unified messages, mutating
    /// terminal state as needed. Called once per non-empty stdout line.
    pub map_event:
        Box<dyn Fn(&serde_json::Value, &mut JsonState) -> Vec<AgentMessage> + Send + Sync>,
    /// Optional handler for non-JSON lines (plain-text stdout CLIs). When
    /// `None`, non-JSON lines are ignored.
    pub map_raw: Option<RawLineMapper>,
}

/// Run a json-family CLI to completion.
pub async fn run(
    spec: JsonSpec,
    opts: ExecOptions,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    mut control_rx: Option<
        tokio::sync::mpsc::UnboundedReceiver<crate::domains::project_management::task_manager::TaskControl>,
    >,
) -> BackendResult<AgentSession> {
    let mut cmd = agent_command(&spec.command);
    cmd.args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = opts
        .cwd
        .as_deref()
        .filter(|p| std::path::Path::new(p).is_dir())
    {
        cmd.current_dir(dir);
    }
    if let Some(env) = &opts.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| BackendError::Launch(format!("{}: {e}", spec.command)))?;
    let process_tree = match super::proc::RunProcessTree::attach(&child) {
        Ok(tree) => tree,
        Err(error) => { let _ = child.kill().await; return Err(BackendError::Launch(format!("cannot contain worker process tree: {error}"))); }
    };

    if let (Some(mut rx), Some(mut stdin)) = (control_rx, child.stdin.take()) {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Some(control) = rx.recv().await {
                // For superagent IPC, we serialize it as UserResponse or Intervene.
                let msg = match control {
                    crate::domains::project_management::task_manager::TaskControl::Subagent(control) => {
                        serde_json::json!({"type":"SubagentControl","payload":control})
                    }
                    crate::domains::project_management::task_manager::TaskControl::UserResponse {
                        request_id,
                        response,
                    } => {
                        serde_json::json!({
                            "type": "UserResponse",
                            "payload": {
                                "request_id": request_id,
                                "response": response
                            }
                        })
                    }
                    crate::domains::project_management::task_manager::TaskControl::Intervene { instruction } => {
                        serde_json::json!({
                            "type": "Intervene",
                            "payload": instruction
                        })
                    }
                };
                let mut s = msg.to_string();
                s.push_str("\n");

                if stdin.write_all(s.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }

    let (msg_tx, msg_rx) = mpsc::channel::<AgentMessage>(256);
    let (res_tx, res_rx) = oneshot::channel::<AgentResult>();

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let timeout = opts.timeout;
    let map_event = spec.map_event;
    let map_raw = spec.map_raw;

    tokio::spawn(async move {
        let started = Instant::now();

        let stderr_tail = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
        if let Some(stderr) = stderr {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut guard = tail.lock().await;
                    guard.push_str(&line);

                    if guard.len() > 4096 {
                        let mut cut = guard.len() - 4096;
                        while cut < guard.len() && !guard.is_char_boundary(cut) {
                            cut += 1;
                        }
                        *guard = guard[cut..].to_string();
                    }
                }
            });
        }

        let mut state = JsonState::default();
        let mut timed_out = false;
        let mut cancelled = false;

        if let Some(stdout) = stdout {
            let mut lines = BufReader::new(stdout).lines();
            let scan = async {
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                        for m in map_event(&event, &mut state) {
                            let _ = msg_tx.send(m).await;
                        }
                        if state.terminal_received { break; }
                    } else if let Some(map_raw) = &map_raw {
                        for m in map_raw(line, &mut state) {
                            let _ = msg_tx.send(m).await;
                        }
                    }
                }
            };
            let cancel_fut = async {
                let _ = cancel_rx.wait_for(|&c| c).await;
            };
            match timeout {
                Some(dur) => {
                    tokio::select! {
                        res = tokio::time::timeout(dur, scan) => {
                            if res.is_err() {
                                timed_out = true;
                                process_tree.terminate();
                                let _ = child.kill().await;
                            }
                        }
                        _ = cancel_fut => {
                            cancelled = true;
                            process_tree.terminate();
                            let _ = child.kill().await;
                        }
                    }
                }
                None => {
                    tokio::select! {
                        _ = scan => {}
                        _ = cancel_fut => {
                            cancelled = true;
                            process_tree.terminate();
                            let _ = child.kill().await;
                        }
                    }
                }
            }
        }

        if state.terminal_received {
            process_tree.terminate();
            let _ = child.kill().await;
        }
        let status = child.wait().await;
        let duration_ms = started.elapsed().as_millis() as i64;
        let tail = stderr_tail.lock().await.trim().to_string();

        let mut result = finalize(&state, status, timed_out, cancelled, duration_ms, &tail);
        if let Some(session)=&state.subagent_session {
            // The process tree has been terminated/reaped before touching its
            // lease. Only the child ledger determines reconciled child states.
            match crate::services::brain::subagent_journal::after_parent_exit(session,&result.status){
                Ok(frames)=>for frame in frames{let _=msg_tx.send(AgentMessage::Log{level:"subagent".into(),content:frame.to_string()}).await;},
                Err(error)=>{
                    let _=msg_tx.send(AgentMessage::Log{level:"error".into(),content:format!("subagent reconciliation failed: {error}")}).await;
                    if result.status=="completed"{result.status="failed".into();result.error=format!("child lifecycle reconciliation failed: {error}");}
                }
            }
        }
        drop(msg_tx);
        let _ = res_tx.send(result);
    });

    Ok(AgentSession {
        messages: msg_rx,
        result: res_rx,
    })
}

fn finalize(
    state: &JsonState,
    status: std::io::Result<std::process::ExitStatus>,
    timed_out: bool,
    cancelled: bool,
    duration_ms: i64,
    stderr_tail: &str,
) -> AgentResult {
    let (status_str, error) = if cancelled {
        ("cancelled".to_string(), "run cancelled by user".to_string())
    } else if timed_out {
        ("timeout".to_string(), "run timed out".to_string())
    } else if let Some(forced) = &state.forced_status {
        (forced.clone(), state.forced_error.clone())
    } else {
        let exit_ok = matches!(&status, Ok(s) if s.success());
        if exit_ok {
            ("completed".to_string(), String::new())
        } else {
            let err = if stderr_tail.is_empty() {
                format!("agent exited with {:?}", status.map(|s| s.code()))
            } else {
                stderr_tail.to_string()
            };
            ("failed".to_string(), err)
        }
    };

    let mut error = error;
    if status_str == "failed"
        && !stderr_tail.is_empty()
        && !error.contains(stderr_tail)
        && error.is_empty()
    {
        error = stderr_tail.to_string();
    }

    AgentResult {
        status: status_str,
        output: state.final_output(),
        error,
        duration_ms,
        session_id: state.session_id.clone(),
        usage: state.usage.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[tokio::test]
    async fn actual_driver_cancel_timeout_and_exit_reap_descendants() {
        use windows_sys::Win32::{Foundation::{CloseHandle, WAIT_OBJECT_0}, System::Threading::{OpenProcess, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject}};
        let python = std::process::Command::new("python").args(["-X", "utf8", "-c", "import sys;print(sys.executable)"]).output().unwrap();
        let python = String::from_utf8(python.stdout).unwrap().trim().to_string();
        for mode in ["cancel", "timeout", "normal", "terminal-marker"] {
            let inherited = if mode == "terminal-marker" { "None" } else { "subprocess.DEVNULL" };
            let ending = if mode == "normal" { "" } else if mode == "terminal-marker" { "print(json.dumps({'done':True}),flush=True)\ntime.sleep(30)" } else { "time.sleep(30)" };
            let script = format!("import subprocess,sys,time,json\ntime.sleep(0.2)\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'],stdin=subprocess.DEVNULL,stdout={inherited},stderr=subprocess.DEVNULL)\nprint(json.dumps({{'pid':p.pid}}),flush=True)\n{ending}");
            let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
            let mut session = run(JsonSpec { command: python.clone(), args: vec!["-u".into(), "-c".into(), script],
                map_event: Box::new(|value, state| {
                    if value["done"] == true { state.terminal_received = true; state.forced_status = Some("completed".into()); vec![] }
                    else { vec![AgentMessage::Text { content: value["pid"].to_string() }] }
                }), map_raw: None },
                ExecOptions { timeout: Some(std::time::Duration::from_secs(1)), ..Default::default() }, cancel_rx, None).await.unwrap();
            let pid: u32 = match session.messages.recv().await.unwrap() { AgentMessage::Text { content } => content.parse().unwrap(), _ => panic!("missing process fixture event") };
            let descendant = unsafe { OpenProcess(SYNCHRONIZATION_SYNCHRONIZE, 0, pid) };
            if mode == "cancel" { cancel_tx.send(true).unwrap(); }
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), session.result).await.unwrap().unwrap();
            assert_eq!(outcome.status, if mode == "cancel" { "cancelled" } else if mode == "timeout" { "timeout" } else { "completed" }, "{mode}");
            if !descendant.is_null() {
                let exited = unsafe { WaitForSingleObject(descendant, 200) };
                unsafe { CloseHandle(descendant); }
                assert_eq!(exited, WAIT_OBJECT_0, "{mode}: driver left a descendant alive");
            } else { assert!(matches!(mode, "normal" | "terminal-marker"), "fixture descendant was never observable: {mode}"); }
        }
    }

    // Exercises the shared state/finalize with a copilot-shaped mapper inline,
    // proving the driver contract without spawning a process.
    fn copilot_map(evt: &serde_json::Value, st: &mut JsonState) -> Vec<AgentMessage> {
        let mut out = Vec::new();
        match evt.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "session.start" => {
                if let Some(sid) = evt.pointer("/data/sessionId").and_then(|v| v.as_str()) {
                    st.session_id = sid.to_string();
                }
            }
            "assistant.message" => {
                if let Some(c) = evt.pointer("/data/content").and_then(|v| v.as_str()) {
                    st.output = c.to_string();
                    st.pending_delta.clear();
                    out.push(AgentMessage::Text {
                        content: c.to_string(),
                    });
                }
            }
            "result" => {
                if evt.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0) != 0 {
                    st.forced_status = Some("failed".to_string());
                }
            }
            _ => {}
        }
        out
    }

    #[test]
    fn final_output_prefers_pending_when_present() {
        let mut st = JsonState::default();
        st.output = "old".to_string();
        st.pending_delta = "streaming".to_string();
        assert_eq!(st.final_output(), "streaming");
        st.pending_delta.clear();
        assert_eq!(st.final_output(), "old");
    }

    #[test]
    fn mapper_captures_session_and_output() {
        let mut st = JsonState::default();
        for line in [
            r#"{"type":"session.start","data":{"sessionId":"s1"}}"#,
            r#"{"type":"assistant.message","data":{"content":"answer"}}"#,
        ] {
            let ev: serde_json::Value = serde_json::from_str(line).unwrap();
            copilot_map(&ev, &mut st);
        }
        assert_eq!(st.session_id, "s1");
        assert_eq!(st.final_output(), "answer");
        assert!(st.forced_status.is_none());
    }
}
