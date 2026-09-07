//! Codex app-server backend implementation.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};

use super::backend::{AgentBackend, BackendError, BackendResult};
use super::proc::agent_command;
use super::types::{AgentMessage, AgentResult, AgentSession, ExecOptions};

pub struct AppServerBackend;

impl AppServerBackend {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AgentBackend for AppServerBackend {
    async fn execute(
        &self,
        prompt: String,
        opts: ExecOptions,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
        _control_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::services::task_manager::TaskControl>,
        >,
    ) -> BackendResult<AgentSession> {
        let run_timeout = opts.timeout;
        let mut cmd = agent_command("codex");
        cmd.arg("app-server").arg("--listen").arg("stdio://");

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

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| BackendError::Launch(format!("failed to spawn app-server: {}", e)))?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Start stderr reader
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let tail_clone = stderr_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let mut guard = tail_clone.lock().await;
                        guard.push_str(&line);
                        guard.push('\n');
                        if guard.len() > 8192 {
                            let drain = guard.len() - 8192;
                            guard.drain(..drain);
                        }
                    }
                    _ => break,
                }
            }
        });

        // Setup JSON-RPC channels
        let (req_tx, mut req_rx) = mpsc::channel::<Value>(32);
        let responses = Arc::new(Mutex::new(HashMap::<
            u64,
            tokio::sync::oneshot::Sender<Value>,
        >::new()));
        let responses_clone = responses.clone();

        // Notifications channel (from daemon to our turn loop)
        let (notify_tx, mut notify_rx) = mpsc::channel::<Value>(128);

        let reader_handle = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Ok(value) = serde_json::from_str::<Value>(line) {
                            if let Some(id_val) = value.get("id") {
                                if let Some(id) = id_val.as_u64() {
                                    let mut map = responses_clone.lock().await;
                                    if let Some(tx) = map.remove(&id) {
                                        let _ = tx.send(value);
                                    }
                                }
                            } else if value.get("method").is_some() {
                                let _ = notify_tx.send(value).await;
                            }
                        }
                    }
                    _ => break,
                }
            }
        });

        let writer_handle = tokio::spawn(async move {
            while let Some(req) = req_rx.recv().await {
                let mut serialized = req.to_string();
                serialized.push('\n');
                if stdin.write_all(serialized.as_bytes()).await.is_err() {
                    break;
                }
            }
        });

        let mut state = AppServerState {
            req_tx,
            responses,
            next_id: 1,
            thread_id: String::new(),
        };

        // Handshake
        let init_res = state
            .call(
                "initialize",
                json!({
                    "clientInfo": { "name": "hub-rs", "version": "0.1.0" },
                    "capabilities": { "experimentalApi": true }
                }),
            )
            .await?;
        if init_res.get("error").is_some() {
            return Err(BackendError::Launch(format!(
                "initialize failed: {:?}",
                init_res
            )));
        }

        let mut start_params = json!({
            "cwd": opts.cwd,
            "persistExtendedHistory": true,
        });
        if let Some(m) = opts.model {
            start_params
                .as_object_mut()
                .unwrap()
                .insert("model".to_string(), Value::String(m));
        }

        let start_res = state.call("thread/start", start_params).await?;
        if let Some(err) = start_res.get("error") {
            return Err(BackendError::Launch(format!(
                "thread/start failed: {:?}",
                err
            )));
        }
        let thread_id = start_res
            .get("result")
            .and_then(|r| r.get("threadId"))
            .and_then(|id| id.as_str())
            .ok_or_else(|| {
                BackendError::Launch("no threadId in thread/start response".to_string())
            })?
            .to_string();

        state.thread_id = thread_id.clone();

        // Start turn
        let turn_res = state
            .call(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{"type": "text", "text": prompt}]
                }),
            )
            .await?;
        if let Some(err) = turn_res.get("error") {
            return Err(BackendError::Launch(format!(
                "turn/start failed: {:?}",
                err
            )));
        }

        let (msg_tx, msg_rx) = mpsc::channel(128);
        let (res_tx, res_rx) = tokio::sync::oneshot::channel();

        // Turn read loop
        let thread_id_clone = thread_id.clone();

        tokio::spawn(async move {
            let mut final_result = AgentResult {
                session_id: thread_id_clone.clone(),
                ..Default::default()
            };

            let turn_loop = async {
                while let Some(notify) = notify_rx.recv().await {
                    let method = notify.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    let params = notify.get("params");

                    if method == "turn/started" {
                        let _ = msg_tx
                            .send(AgentMessage::Status {
                                status: "running".to_string(),
                                session_id: thread_id_clone.clone(),
                            })
                            .await;
                    } else if method == "turn/completed" {
                        final_result.status = "completed".to_string();
                        if let Some(p) = params {
                            if let Some(st) = p
                                .get("turn")
                                .and_then(|t| t.get("status"))
                                .and_then(|s| s.as_str())
                            {
                                if st == "failed" {
                                    final_result.status = "failed".to_string();
                                    final_result.error = p
                                        .get("turn")
                                        .and_then(|t| t.get("error"))
                                        .and_then(|e| e.get("message"))
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("codex turn failed")
                                        .to_string();
                                } else if st == "cancelled"
                                    || st == "canceled"
                                    || st == "aborted"
                                    || st == "interrupted"
                                {
                                    final_result.status = "cancelled".to_string();
                                    final_result.error = "run cancelled by user".to_string();
                                }
                            }
                        }
                        break;
                    } else if method == "error" {
                        let will_retry = params
                            .and_then(|p| p.get("willRetry"))
                            .and_then(|w| w.as_bool())
                            .unwrap_or(false);
                        if !will_retry {
                            let msg = params
                                .and_then(|p| p.get("error"))
                                .and_then(|e| e.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or("codex error")
                                .to_string();
                            let _ = msg_tx
                                .send(AgentMessage::Error {
                                    content: msg.clone(),
                                })
                                .await;
                            final_result.status = "failed".to_string();
                            final_result.error = msg;
                            break;
                        }
                    } else if method == "thread/status/changed" {
                        if let Some(p) = params {
                            if let Some(st_val) = p.get("status") {
                                if st_val.get("type").and_then(|t| t.as_str()) == Some("idle") {
                                    break;
                                }
                            }
                        }
                    } else if method == "item/started" {
                        if let Some(p) = params {
                            if let Some(item) = p.get("item") {
                                let item_type =
                                    item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                let item_id = item
                                    .get("id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if item_type == "commandExecution" {
                                    let cmd = item
                                        .get("command")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let mut input = serde_json::Map::new();
                                    input.insert("command".to_string(), Value::String(cmd));
                                    let _ = msg_tx
                                        .send(AgentMessage::ToolUse {
                                            tool: "exec_command".to_string(),
                                            call_id: item_id,
                                            input: Some(Value::Object(input)),
                                        })
                                        .await;
                                } else if item_type == "fileChange" {
                                    let _ = msg_tx
                                        .send(AgentMessage::ToolUse {
                                            tool: "patch_apply".to_string(),
                                            call_id: item_id,
                                            input: None,
                                        })
                                        .await;
                                }
                            }
                        }
                    } else if method == "item/completed" {
                        if let Some(p) = params {
                            if let Some(item) = p.get("item") {
                                let item_type =
                                    item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                let item_id = item
                                    .get("id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if item_type == "commandExecution" {
                                    let output = item
                                        .get("aggregatedOutput")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let _ = msg_tx
                                        .send(AgentMessage::ToolResult {
                                            tool: "exec_command".to_string(),
                                            call_id: item_id,
                                            output,
                                        })
                                        .await;
                                } else if item_type == "fileChange" {
                                    let _ = msg_tx
                                        .send(AgentMessage::ToolResult {
                                            tool: "patch_apply".to_string(),
                                            call_id: item_id,
                                            output: String::new(),
                                        })
                                        .await;
                                } else if item_type == "agentMessage" {
                                    let text = item
                                        .get("text")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if !text.is_empty() {
                                        let _ = msg_tx
                                            .send(AgentMessage::Text {
                                                content: text.clone(),
                                            })
                                            .await;
                                    }
                                    if item.get("phase").and_then(|p| p.as_str())
                                        == Some("final_answer")
                                    {
                                        final_result.output = text;
                                    }
                                }
                            }
                        }
                    }
                }
            };

            let cancel_fut = async {
                let _ = cancel_rx.wait_for(|&c| c).await;
            };

            if let Some(timeout) = run_timeout {
                tokio::select! {
                    result = tokio::time::timeout(timeout, turn_loop) => {
                        if result.is_err() {
                            final_result.status = "timeout".to_string();
                            final_result.error = "run timed out".to_string();
                        }
                    }
                    _ = cancel_fut => {
                        final_result.status = "cancelled".to_string();
                        final_result.error = "run cancelled by user".to_string();
                    }
                }
            } else {
                tokio::select! {
                    _ = turn_loop => {}
                    _ = cancel_fut => {
                        final_result.status = "cancelled".to_string();
                        final_result.error = "run cancelled by user".to_string();
                    }
                }
            }

            if final_result.status.is_empty() {
                final_result.status = "completed".to_string();
            }
            let _ = res_tx.send(final_result);

            // Clean up when turn is done
            let _ = child.kill().await;
            reader_handle.abort();
            writer_handle.abort();
        });

        Ok(AgentSession {
            messages: msg_rx,
            result: res_rx,
        })
    }

    async fn sync_config(
        &self,
        config: &super::types::SyncConfigPayload,
        _executable_path: Option<&str>,
    ) -> BackendResult<()> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".codex");
        crate::services::agent::sync_utils::sync_codex(config, &global_dir)
    }
}

struct AppServerState {
    req_tx: mpsc::Sender<Value>,
    responses: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>,
    next_id: u64,
    pub thread_id: String,
}

impl AppServerState {
    async fn call(&mut self, method: &str, params: Value) -> BackendResult<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.responses.lock().await.insert(id, tx);

        self.req_tx
            .send(req)
            .await
            .map_err(|_| BackendError::Launch("failed to send RPC request".to_string()))?;

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(_)) => Err(BackendError::Launch(
                "RPC response channel closed".to_string(),
            )),
            Err(_) => Err(BackendError::Launch("RPC request timed out".to_string())),
        }
    }
}
