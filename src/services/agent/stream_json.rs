//! Shared driver for the **stream-json** protocol family
//! (claude / cursor / qwen / codebuddy).
//!
//! These CLIs all speak an Anthropic-style newline-delimited JSON stream: each
//! line is an object with a top-level `type` in {system, assistant, user,
//! result, ...}. The prompt is fed on **stdin** (kept open for the run so the
//! CLI can issue control requests), and the terminal `result` line carries the
//! final status + usage. Per-CLI differences are limited to argv construction
//! and whether the prompt rides on stdin vs. a positional arg — captured by
//! [`StreamJsonSpec`]; the line-by-line parsing below is identical for all four.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use super::backend::{BackendError, BackendResult};
use super::proc::agent_command;
use super::types::{AgentMessage, AgentResult, AgentSession, ExecOptions, TokenUsage};

/// How the prompt is delivered to a stream-json CLI.
#[derive(Clone, Copy, PartialEq)]
pub enum PromptDelivery {
    /// Written to stdin and stdin is closed (claude, codebuddy, qwen args
    /// carry the prompt; cursor relies on stdin-to-EOF).
    Stdin,
}

/// Per-backend knobs for the stream-json driver. `build_args` produces the argv
/// after the command name; the prompt handling is decided by `delivery`.
pub struct StreamJsonSpec {
    /// CLI command name (also the argv0 the shell resolves).
    pub command: String,
    /// Whether the prompt is embedded in argv (built by `build_args`) or must
    /// be piped through stdin because the CLI takes it as a positional.
    pub prompt_on_stdin: bool,
    /// Builds the argument vector (excluding the command name and, when
    /// `prompt_on_stdin` is false, excluding the prompt itself).
    pub build_args:
        Box<dyn Fn(&str, &ExecOptions, Option<&std::path::Path>) -> Vec<String> + Send + Sync>,

    /// Optional environment variables to set for the CLI command.
    pub envs: Option<Box<dyn Fn(&ExecOptions) -> HashMap<String, String> + Send + Sync>>,
}

/// Run a stream-json CLI to completion, translating its stream into unified
/// [`AgentMessage`]s and a final [`AgentResult`].
pub async fn run(
    spec: StreamJsonSpec,
    prompt: String,
    opts: ExecOptions,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    _control_rx: Option<
        tokio::sync::mpsc::UnboundedReceiver<crate::domains::project_management::task_manager::TaskControl>,
    >,
) -> BackendResult<AgentSession> {
    let mut _mcp_guard = None;
    let mcp_path = if crate::services::agent::mcp::has_managed_mcp_config(opts.mcp_config.as_ref())
    {
        let guard =
            crate::services::agent::mcp::McpConfigTempFile::new(opts.mcp_config.as_ref().unwrap())
                .map_err(|e| BackendError::Launch(format!("mcp config: {e}")))?;
        let path = guard.path().to_path_buf();
        _mcp_guard = Some(guard);
        Some(path)
    } else {
        None
    };

    let args = (spec.build_args)(&prompt, &opts, mcp_path.as_deref());

    let mut cmd = agent_command(&spec.command);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(envs_fn) = &spec.envs {
        cmd.envs((envs_fn)(&opts));
    }

    if let Some(dir) = opts
        .cwd
        .as_deref()
        .filter(|p| std::path::Path::new(p).is_dir())
    {
        cmd.current_dir(dir);
    }
    if let Some(env) = &opts.env {
        cmd.envs(env);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| BackendError::Launch(format!("{}: {e}", spec.command)))?;

    // Feed the prompt via stdin. Some CLIs (cursor) require it as the prompt
    // source; others already have it in argv but still expect stdin closed.
    if let Some(mut stdin) = child.stdin.take() {
        if spec.prompt_on_stdin {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        }
        let _ = stdin.shutdown().await;
    }

    let (msg_tx, msg_rx) = mpsc::channel::<AgentMessage>(256);
    let (res_tx, res_rx) = oneshot::channel::<AgentResult>();

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let timeout = opts.timeout;
    let resume = opts.resume_session_id.clone();

    tokio::spawn(async move {
        let _guard_moved = _mcp_guard;
        let started = Instant::now();

        // Drain stderr into a bounded tail for error reporting.
        let stderr_tail = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
        if let Some(stderr) = stderr {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut guard = tail.lock().await;
                    guard.push_str(&line);
                    guard.push('\n');
                    if guard.len() > 4096 {
                        let cut = guard.len() - 4096;
                        *guard = guard[cut..].to_string();
                    }
                }
            });
        }

        let mut state = StreamState::default();

        if let Some(stdout) = stdout {
            let mut lines = BufReader::new(stdout).lines();
            let scan = async {
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                        handle_event(&event, &msg_tx, &mut state).await;
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
                                state.timed_out = true;
                                let _ = child.kill().await;
                            }
                        }
                        _ = cancel_fut => {
                            state.cancelled = true;
                            let _ = child.kill().await;
                        }
                    }
                }
                None => {
                    tokio::select! {
                        _ = scan => {}
                        _ = cancel_fut => {
                            state.cancelled = true;
                            let _ = child.kill().await;
                        }
                    }
                }
            }
        }

        let status = child.wait().await;
        let duration_ms = started.elapsed().as_millis() as i64;
        let tail = stderr_tail.lock().await.trim().to_string();

        let result = state.finalize(status, duration_ms, &resume, &tail);
        drop(msg_tx);
        let _ = res_tx.send(result);
    });

    Ok(AgentSession {
        messages: msg_rx,
        result: res_rx,
    })
}

/// Mutable accumulation while scanning the stream-json output.
#[derive(Default)]
struct StreamState {
    session_id: String,
    model: String,
    last_assistant_text: String,
    final_result_text: String,
    saw_result: bool,
    result_is_error: bool,
    timed_out: bool,
    cancelled: bool,
    usage: HashMap<String, TokenUsage>,
}

impl StreamState {
    fn finalize(
        &self,
        status: std::io::Result<std::process::ExitStatus>,
        duration_ms: i64,
        resume: &Option<String>,
        stderr_tail: &str,
    ) -> AgentResult {
        let session_id = if self.session_id.is_empty() {
            resume.clone().unwrap_or_default()
        } else {
            self.session_id.clone()
        };

        let (status_str, output, error) = if self.cancelled {
            (
                "cancelled".to_string(),
                String::new(),
                "run cancelled by user".to_string(),
            )
        } else if self.timed_out {
            (
                "timeout".to_string(),
                String::new(),
                "run timed out".to_string(),
            )
        } else if self.result_is_error {
            (
                "failed".to_string(),
                String::new(),
                if self.final_result_text.is_empty() {
                    stderr_tail.to_string()
                } else {
                    self.final_result_text.clone()
                },
            )
        } else {
            let exit_ok = matches!(&status, Ok(s) if s.success());
            let out = if !self.final_result_text.is_empty() {
                self.final_result_text.clone()
            } else {
                self.last_assistant_text.clone()
            };
            if exit_ok || self.saw_result {
                ("completed".to_string(), out, String::new())
            } else {
                let err = if stderr_tail.is_empty() {
                    "agent exited without a result".to_string()
                } else {
                    stderr_tail.to_string()
                };
                ("failed".to_string(), out, err)
            }
        };

        AgentResult {
            status: status_str,
            output,
            error,
            duration_ms,
            session_id,
            usage: self.usage.clone(),
        }
    }
}

/// Dispatch one stream-json line by its top-level `type`.
async fn handle_event(
    event: &serde_json::Value,
    tx: &mpsc::Sender<AgentMessage>,
    state: &mut StreamState,
) {
    if let Some(sid) = event.get("session_id").and_then(|v| v.as_str()) {
        if !sid.is_empty() {
            state.session_id = sid.to_string();
        }
    }
    if let Some(model) = event.get("model").and_then(|v| v.as_str()) {
        if !model.is_empty() {
            state.model = model.to_string();
        }
    }

    match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "system" => {
            let _ = tx
                .send(AgentMessage::Status {
                    status: "running".to_string(),
                    session_id: state.session_id.clone(),
                })
                .await;
        }
        "assistant" => {
            handle_assistant(event.get("message"), tx, state).await;
        }
        "user" => {
            handle_user(event.get("message"), tx).await;
        }
        "result" => {
            state.saw_result = true;
            let subtype = event.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            state.result_is_error = event
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || subtype == "error"
                || subtype == "failed";
            if let Some(text) = event.get("result").and_then(|v| v.as_str()) {
                state.final_result_text = text.to_string();
            } else if state.result_is_error {
                state.final_result_text = error_text(event);
            }
            accumulate_usage(event.get("usage"), &state.model, &mut state.usage);
        }
        "error" => {
            state.saw_result = true;
            state.result_is_error = true;
            state.final_result_text = error_text(event);
        }
        _ => {}
    }
}

async fn handle_assistant(
    message: Option<&serde_json::Value>,
    tx: &mpsc::Sender<AgentMessage>,
    state: &mut StreamState,
) {
    let Some(message) = message else { return };
    if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
        if !model.is_empty() {
            state.model = model.to_string();
        }
    }
    accumulate_usage(message.get("usage"), &state.model, &mut state.usage);

    let mut turn_text = String::new();
    let mut tools = 0usize;
    if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
        for block in content {
            match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "thinking" => {
                    if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            let _ = tx
                                .send(AgentMessage::Thinking {
                                    content: t.to_string(),
                                })
                                .await;
                        }
                    }
                }
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            turn_text.push_str(t);
                            let _ = tx
                                .send(AgentMessage::Text {
                                    content: t.to_string(),
                                })
                                .await;
                        }
                    }
                }
                "tool_use" => {
                    tools += 1;
                    let _ = tx
                        .send(AgentMessage::ToolUse {
                            tool: block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            call_id: block
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            input: block.get("input").cloned(),
                        })
                        .await;
                }
                _ => {}
            }
        }
    }
    if tools == 0 && !turn_text.is_empty() {
        state.last_assistant_text = turn_text;
    } else if tools > 0 {
        state.last_assistant_text.clear();
    }
}

async fn handle_user(message: Option<&serde_json::Value>, tx: &mpsc::Sender<AgentMessage>) {
    let Some(message) = message else { return };
    if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
        for block in content {
            if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                let output = match block.get("content") {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                let _ = tx
                    .send(AgentMessage::ToolResult {
                        call_id: block
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        tool: String::new(),
                        output,
                    })
                    .await;
            }
        }
    }
}

/// Merge an Anthropic-style `usage` object into the per-model accounting.
fn accumulate_usage(
    usage: Option<&serde_json::Value>,
    model: &str,
    into: &mut HashMap<String, TokenUsage>,
) {
    let Some(usage) = usage else { return };
    if model.is_empty() {
        return;
    }
    let get = |k: &str| usage.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let input = get("input_tokens");
    let output = get("output_tokens");
    let cache_read = get("cache_read_input_tokens");
    let cache_write = get("cache_creation_input_tokens");
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
        return;
    }
    let entry = into.entry(model.to_string()).or_default();
    entry.input_tokens += input;
    entry.output_tokens += output;
    entry.cache_read_tokens += cache_read;
    entry.cache_write_tokens += cache_write;
}

/// Best-effort extraction of a human-readable error from a result/error line.
fn error_text(event: &serde_json::Value) -> String {
    if let Some(r) = event.get("result").and_then(|v| v.as_str()) {
        if !r.is_empty() {
            return r.to_string();
        }
    }
    if let Some(m) = event
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|v| v.as_str())
    {
        return m.to_string();
    }
    if let Some(e) = event.get("error") {
        if !e.is_null() {
            return e.to_string();
        }
    }
    "agent returned an error event without details".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_assistant_text_tool_and_result() {
        let (tx, mut rx) = mpsc::channel::<AgentMessage>(64);
        let mut state = StreamState::default();
        let lines = [
            r#"{"type":"system","session_id":"sess-1"}"#,
            r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"Bash","input":{"cmd":"ls"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"files"}]}}"#,
            r#"{"type":"result","result":"done","usage":{"input_tokens":10,"output_tokens":5}}"#,
        ];
        for l in lines {
            let ev: serde_json::Value = serde_json::from_str(l).unwrap();
            handle_event(&ev, &tx, &mut state).await;
        }
        drop(tx);

        let mut kinds = Vec::new();
        while let Some(m) = rx.recv().await {
            kinds.push(match m {
                AgentMessage::Status { .. } => "status",
                AgentMessage::Text { .. } => "text",
                AgentMessage::ToolUse { .. } => "tool-use",
                AgentMessage::ToolResult { .. } => "tool-result",
                _ => "other",
            });
        }
        assert_eq!(kinds, vec!["status", "text", "tool-use", "tool-result"]);
        assert_eq!(state.session_id, "sess-1");
        assert!(state.saw_result);
        assert_eq!(state.final_result_text, "done");
        assert_eq!(state.usage.get("m").map(|u| u.input_tokens), Some(10));
    }

    #[tokio::test]
    async fn result_error_marks_failure() {
        let (tx, _rx) = mpsc::channel::<AgentMessage>(64);
        let mut state = StreamState::default();
        let ev: serde_json::Value =
            serde_json::from_str(r#"{"type":"result","is_error":true,"error":{"message":"boom"}}"#)
                .unwrap();
        handle_event(&ev, &tx, &mut state).await;
        assert!(state.result_is_error);
        assert_eq!(state.final_result_text, "boom");
    }
}
