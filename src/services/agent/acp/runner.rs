use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::services::agent::backend::{BackendError, BackendResult};
use crate::services::agent::proc::agent_command;
use crate::services::agent::types::{AgentMessage, AgentResult, AgentSession, ExecOptions, TokenUsage};

/// Per-backend knobs for the ACP driver.
pub struct AcpSpec {
    /// CLI command name (e.g. "hermes")
    pub command: String,
    /// Builds the argument vector after the command name.
    pub build_args:
        Box<dyn Fn(&ExecOptions, Option<&std::path::Path>) -> Vec<String> + Send + Sync>,
}
#[derive(Default)]
struct AcpDeliverableTracker {
    full: String,
    deliverable: String,
    last_text_block: String,
}

impl AcpDeliverableTracker {
    fn observe_text(&mut self, text: &str) {
        self.full.push_str(text);
        self.deliverable.push_str(text);
    }
    fn observe_tool_use(&mut self) {
        let block = self.deliverable.trim();
        if !block.is_empty() {
            self.last_text_block = block.to_string();
        }
        self.deliverable.clear();
    }
    fn result(&self) -> (String, String) {
        let mut deliv = self.deliverable.trim().to_string();
        if deliv.is_empty() {
            deliv = self.last_text_block.clone();
        }
        (deliv, self.full.clone())
    }
}

struct PendingRpc {
    tx: oneshot::Sender<Result<Value, String>>,
}

struct AcpClientState {
    next_id: i64,
    pending: HashMap<i64, PendingRpc>,
    session_id: String,
    model: String,
    usage: TokenUsage,
    result_is_error: bool,
    final_error: String,
    timed_out: bool,
    cancelled: bool,
}

fn forced_terminal(state: &AcpClientState) -> Option<(&'static str, String)> {
    if state.cancelled {
        Some(("cancelled", "run cancelled by user".to_string()))
    } else if state.timed_out {
        Some(("timeout", "run timed out".to_string()))
    } else if state.result_is_error {
        Some(("failed", state.final_error.clone()))
    } else {
        None
    }
}

pub async fn run(
    spec: AcpSpec,
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

    let args = (spec.build_args)(&opts, mcp_path.as_deref());
    let mut cmd = agent_command(&spec.command);
    cmd.args(&args)
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
        cmd.envs(env);
    }
    // YOLO mode to avoid blocking on shell prompts.
    cmd.env("HERMES_YOLO_MODE", "1");

    let mut child = cmd
        .spawn()
        .map_err(|e| BackendError::Launch(format!("{}: {e}", spec.command)))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| BackendError::Launch("No stdin".to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BackendError::Launch("No stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| BackendError::Launch("No stderr".to_string()))?;

    let (msg_tx, msg_rx) = mpsc::channel::<AgentMessage>(256);
    let (res_tx, res_rx) = oneshot::channel::<AgentResult>();
    let (rpc_tx, mut rpc_rx) = mpsc::channel::<Value>(256); // To send JSON-RPC requests to stdin loop

    let state = Arc::new(Mutex::new(AcpClientState {
        next_id: 1,
        pending: HashMap::new(),
        session_id: String::new(),
        model: opts.model.clone().unwrap_or_default(),
        usage: TokenUsage::default(),
        result_is_error: false,
        final_error: String::new(),
        timed_out: false,
        cancelled: false,
    }));

    let tracker = Arc::new(Mutex::new(AcpDeliverableTracker::default()));

    let stderr_tail = Arc::new(Mutex::new(String::new()));
    let tail_clone = stderr_tail.clone();

    // Stderr reader
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut guard = tail_clone.lock().await;
            guard.push_str(&line);
            guard.push('\n');
            if guard.len() > 4096 {
                let cut = guard.len() - 4096;
                *guard = guard[cut..].to_string();
            }
        }
    });

    // Stdin writer loop
    tokio::spawn(async move {
        while let Some(msg) = rpc_rx.recv().await {
            let mut data = serde_json::to_vec(&msg).unwrap_or_default();
            data.push(b'\n');
            if stdin.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // Helper to send a request and wait for a response
    let request = {
        let state = state.clone();
        let rpc_tx = rpc_tx.clone();
        move |method: &str, params: Value| {
            let state = state.clone();
            let rpc_tx = rpc_tx.clone();
            let method = method.to_string();
            async move {
                let (tx, rx) = oneshot::channel();
                let id = {
                    let mut st = state.lock().await;
                    let id = st.next_id;
                    st.next_id += 1;
                    st.pending.insert(id, PendingRpc { tx });
                    id
                };

                let req = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": params,
                });

                if rpc_tx.send(req).await.is_err() {
                    return Err("Failed to send request".to_string());
                }

                match rx.await {
                    Ok(Ok(val)) => Ok(val),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err("Request cancelled or failed".to_string()),
                }
            }
        }
    };

    let resume = opts.resume_session_id.clone();

    // Stdout reader and JSON-RPC dispatcher
    let state_clone = state.clone();
    let msg_tx_clone = msg_tx.clone();
    let rpc_tx_clone = rpc_tx.clone();
    let tracker_clone = tracker.clone();

    let reader_handle = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                if let Some(id) = value.get("id").and_then(|id| id.as_i64()) {
                    let mut st = state_clone.lock().await;
                    if let Some(pending) = st.pending.remove(&id) {
                        if let Some(err) = value.get("error") {
                            let msg = err
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("Unknown error")
                                .to_string();
                            let _ = pending.tx.send(Err(msg));
                        } else if let Some(res) = value.get("result") {
                            let _ = pending.tx.send(Ok(res.clone()));
                        } else {
                            let _ = pending.tx.send(Ok(Value::Null));
                        }
                    }
                } else if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
                    match method {
                        "session/update" => {
                            if let Some(params) = value.get("params") {
                                if let Some(status) = params.get("status").and_then(|s| s.as_str())
                                {
                                    let sid = state_clone.lock().await.session_id.clone();
                                    if status == "running" || status == "processing" {
                                        let _ = msg_tx_clone
                                            .send(AgentMessage::Status {
                                                status: "running".to_string(),
                                                session_id: sid,
                                            })
                                            .await;
                                    }
                                }
                                if let Some(chunk) = params.get("chunk") {
                                    if let Some(kind) = chunk.get("type").and_then(|k| k.as_str()) {
                                        if kind == "text" {
                                            if let Some(text) =
                                                chunk.get("text").and_then(|t| t.as_str())
                                            {
                                                if !text.is_empty() {
                                                    tracker_clone.lock().await.observe_text(text);
                                                    let _ = msg_tx_clone
                                                        .send(AgentMessage::Text {
                                                            content: text.to_string(),
                                                        })
                                                        .await;
                                                }
                                            }
                                        } else if kind == "tool_use" {
                                            tracker_clone.lock().await.observe_tool_use();
                                            let tool = chunk
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let call_id = chunk
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let input = chunk.get("input").cloned();
                                            let _ = msg_tx_clone
                                                .send(AgentMessage::ToolUse {
                                                    tool,
                                                    call_id,
                                                    input,
                                                })
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                        "session/request_permission" => {
                            // Auto-approve
                            if let Some(params) = value.get("params") {
                                if let Some(req_id) =
                                    params.get("requestId").and_then(|r| r.as_str())
                                {
                                    let sid = state_clone.lock().await.session_id.clone();
                                    let req = json!({
                                        "jsonrpc": "2.0",
                                        "method": "session/grant_permission",
                                        "params": {
                                            "sessionId": sid,
                                            "requestId": req_id,
                                            "action": "allow_once"
                                        }
                                    });
                                    let _ = rpc_tx_clone.send(req).await;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    let timeout = opts.timeout;
    let started = Instant::now();
    let prompt_req = prompt.clone();

    tokio::spawn(async move {
        let _guard_moved = _mcp_guard;
        // Run the ACP state machine
        let run_session = async {
            // 1. initialize
            let _ = request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientInfo": {
                        "name": "hub-rs",
                        "version": "0.1.0",
                    },
                    "clientCapabilities": {}
                }),
            )
            .await
            .map_err(|e| format!("initialize failed: {e}"))?;

            let cwd = opts.cwd.clone().unwrap_or_else(|| ".".to_string());
            let mcp_servers = match &opts.mcp_config {
                Some(serde_json::Value::Array(arr)) => serde_json::Value::Array(arr.clone()),
                Some(serde_json::Value::Object(map)) => {
                    if let Some(servers) = map.get("mcpServers") {
                        servers.clone()
                    } else {
                        serde_json::Value::Object(map.clone())
                    }
                }
                _ => json!([]),
            };
            let session_id = if let Some(ref rid) = resume {
                let res = request(
                    "session/resume",
                    json!({
                        "cwd": cwd,
                        "sessionId": rid,
                        "mcpServers": mcp_servers,
                    }),
                )
                .await
                .map_err(|e| format!("session/resume failed: {e}"))?;
                res.get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or(rid)
                    .to_string()
            } else {
                let params = if let Some(m) = &opts.model {
                    json!({ "cwd": cwd, "mcpServers": mcp_servers, "modelId": m })
                } else {
                    json!({ "cwd": cwd, "mcpServers": mcp_servers })
                };
                let res = request("session/new", params)
                    .await
                    .map_err(|e| format!("session/new failed: {e}"))?;
                res.get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string()
            };

            if session_id.is_empty() {
                return Err("Failed to get session ID".to_string());
            }

            {
                let mut st = state.lock().await;
                st.session_id = session_id.clone();
            }

            // session/prompt
            let prompt_params = json!({
                "sessionId": session_id,
                "prompt": [
                    { "type": "text", "text": prompt_req }
                ]
            });
            let prompt_res = request("session/prompt", prompt_params)
                .await
                .map_err(|e| format!("session/prompt failed: {e}"))?;

            // Extract usage
            if let Some(usage) = prompt_res.get("usage") {
                let mut st = state.lock().await;
                let get_i64 = |k| usage.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
                st.usage.input_tokens += get_i64("inputTokens");
                st.usage.output_tokens += get_i64("outputTokens");
            }

            // Wait a short moment for final updates to arrive
            tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

            Ok::<(), String>(())
        };

        let cancel_fut = async {
            let _ = cancel_rx.wait_for(|&c| c).await;
        };

        match timeout {
            Some(dur) => {
                tokio::select! {
                    res = tokio::time::timeout(dur, run_session) => {
                        if let Err(_) = res {
                            let mut st = state.lock().await;
                            st.timed_out = true;
                            st.final_error = "timeout".to_string();
                            let _ = child.kill().await;
                        } else if let Ok(Err(e)) = res {
                            let mut st = state.lock().await;
                            st.result_is_error = true;
                            st.final_error = e;
                        }
                    }
                    _ = cancel_fut => {
                        let mut st = state.lock().await;
                        st.cancelled = true;
                        let _ = child.kill().await;
                    }
                }
            }
            None => {
                tokio::select! {
                    res = run_session => {
                        if let Err(e) = res {
                            let mut st = state.lock().await;
                            st.result_is_error = true;
                            st.final_error = e;
                        }
                    }
                    _ = cancel_fut => {
                        let mut st = state.lock().await;
                        st.cancelled = true;
                        let _ = child.kill().await;
                    }
                }
            }
        }

        // Signal stdin close
        drop(rpc_tx);

        let status = child.wait().await;
        let _ = reader_handle.await;

        let st = state.lock().await;
        let (deliverable, _full) = tracker.lock().await.result();

        let tail = stderr_tail.lock().await.trim().to_string();

        let (final_status, output, error) = if let Some((status, error)) = forced_terminal(&st) {
            let output = if status == "failed" {
                deliverable
            } else {
                String::new()
            };
            (status.to_string(), output, error)
        } else {
            let exit_ok = matches!(&status, Ok(s) if s.success());
            if exit_ok {
                ("completed".to_string(), deliverable, String::new())
            } else {
                let err = if tail.is_empty() {
                    "agent exited with error".to_string()
                } else {
                    tail.clone()
                };
                ("failed".to_string(), deliverable, err)
            }
        };

        let mut usage_map = HashMap::new();
        if st.usage.input_tokens > 0 || st.usage.output_tokens > 0 {
            usage_map.insert(st.model.clone(), st.usage.clone());
        }

        let result = AgentResult {
            status: final_status,
            output,
            error,
            duration_ms: started.elapsed().as_millis() as i64,
            session_id: st.session_id.clone(),
            usage: usage_map,
        };

        drop(msg_tx);
        let _ = res_tx.send(result);
    });

    Ok(AgentSession {
        messages: msg_rx,
        result: res_rx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AcpClientState {
        AcpClientState {
            next_id: 1,
            pending: HashMap::new(),
            session_id: String::new(),
            model: String::new(),
            usage: TokenUsage::default(),
            result_is_error: false,
            final_error: String::new(),
            timed_out: false,
            cancelled: false,
        }
    }

    #[test]
    fn cancellation_has_distinct_terminal_status_and_wins_races() {
        let mut state = state();
        state.cancelled = true;
        state.timed_out = true;
        state.result_is_error = true;
        assert_eq!(forced_terminal(&state).unwrap().0, "cancelled");
    }

    #[test]
    fn timeout_and_backend_failure_remain_distinct() {
        let mut state = state();
        state.timed_out = true;
        assert_eq!(forced_terminal(&state).unwrap().0, "timeout");

        state.timed_out = false;
        state.result_is_error = true;
        state.final_error = "rpc failed".to_string();
        assert_eq!(
            forced_terminal(&state),
            Some(("failed", "rpc failed".to_string()))
        );
    }
}
