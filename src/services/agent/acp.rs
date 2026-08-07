//! ACP (Agent Client Protocol) driver.
//!
//! The ACP family (hermes / kimi / kiro / qoder / traecli / grok) speaks
//! JSON-RPC 2.0 over stdio as a bidirectional long-lived connection:
//! `initialize` -> `session/new` | `session/load` (resume) -> `session/set_model`
//! -> `session/prompt`, while concurrently handling agent->client notifications
//! (`session/update` streaming chunks, `session/request_permission` which must
//! be auto-answered allow_once/reject_once) and matching pending request ids.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::backend::{BackendError, BackendResult};
use super::proc::agent_command;
use super::types::{AgentMessage, AgentResult, AgentSession, ExecOptions, TokenUsage};

/// Per-backend knobs for the ACP driver.
pub struct AcpSpec {
    /// CLI command name (e.g. "hermes")
    pub command: String,
    /// Builds the argument vector after the command name.
    pub build_args: Box<dyn Fn(&ExecOptions, Option<&std::path::Path>) -> Vec<String> + Send + Sync>,
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
}

pub async fn run(spec: AcpSpec, prompt: String, opts: ExecOptions, mut cancel_rx: tokio::sync::watch::Receiver<bool>) -> BackendResult<AgentSession> {
    let mut _mcp_guard = None;
    let mcp_path = if crate::services::agent::mcp::has_managed_mcp_config(opts.mcp_config.as_ref()) {
        let guard = crate::services::agent::mcp::McpConfigTempFile::new(opts.mcp_config.as_ref().unwrap())
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
        
    if let Some(dir) = opts.cwd.as_deref().filter(|p| std::path::Path::new(p).is_dir()) {
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

    let mut stdin = child.stdin.take().ok_or_else(|| BackendError::Launch("No stdin".to_string()))?;
    let stdout = child.stdout.take().ok_or_else(|| BackendError::Launch("No stdout".to_string()))?;
    let stderr = child.stderr.take().ok_or_else(|| BackendError::Launch("No stderr".to_string()))?;

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
            if line.is_empty() { continue; }
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                if let Some(id) = value.get("id").and_then(|id| id.as_i64()) {
                    let mut st = state_clone.lock().await;
                    if let Some(pending) = st.pending.remove(&id) {
                        if let Some(err) = value.get("error") {
                            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error").to_string();
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
                                if let Some(status) = params.get("status").and_then(|s| s.as_str()) {
                                    let sid = state_clone.lock().await.session_id.clone();
                                    if status == "running" || status == "processing" {
                                        let _ = msg_tx_clone.send(AgentMessage::Status {
                                            status: "running".to_string(),
                                            session_id: sid,
                                        }).await;
                                    }
                                }
                                if let Some(chunk) = params.get("chunk") {
                                    if let Some(kind) = chunk.get("type").and_then(|k| k.as_str()) {
                                        if kind == "text" {
                                            if let Some(text) = chunk.get("text").and_then(|t| t.as_str()) {
                                                if !text.is_empty() {
                                                    tracker_clone.lock().await.observe_text(text);
                                                    let _ = msg_tx_clone.send(AgentMessage::Text { content: text.to_string() }).await;
                                                }
                                            }
                                        } else if kind == "tool_use" {
                                            tracker_clone.lock().await.observe_tool_use();
                                            let tool = chunk.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            let call_id = chunk.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            let input = chunk.get("input").cloned();
                                            let _ = msg_tx_clone.send(AgentMessage::ToolUse { tool, call_id, input }).await;
                                        }
                                    }
                                }
                            }
                        }
                        "session/request_permission" => {
                            // Auto-approve
                            if let Some(params) = value.get("params") {
                                if let Some(req_id) = params.get("requestId").and_then(|r| r.as_str()) {
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
            let _ = request("initialize", json!({
                "protocolVersion": 1,
                "clientInfo": {
                    "name": "hub-rs",
                    "version": "0.1.0",
                },
                "clientCapabilities": {}
            })).await.map_err(|e| format!("initialize failed: {e}"))?;

            let cwd = opts.cwd.clone().unwrap_or_else(|| ".".to_string());
            let mcp_servers = json!([]); // TODO: Map opts.mcp_config
            let session_id = if let Some(ref rid) = resume {
                let res = request("session/resume", json!({
                    "cwd": cwd,
                    "sessionId": rid,
                    "mcpServers": mcp_servers,
                })).await.map_err(|e| format!("session/resume failed: {e}"))?;
                res.get("sessionId").and_then(|s| s.as_str()).unwrap_or(rid).to_string()
            } else {
                let params = if let Some(m) = &opts.model {
                    json!({ "cwd": cwd, "mcpServers": mcp_servers, "modelId": m })
                } else {
                    json!({ "cwd": cwd, "mcpServers": mcp_servers })
                };
                let res = request("session/new", params).await.map_err(|e| format!("session/new failed: {e}"))?;
                res.get("sessionId").and_then(|s| s.as_str()).unwrap_or_default().to_string()
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
            let prompt_res = request("session/prompt", prompt_params).await.map_err(|e| format!("session/prompt failed: {e}"))?;
            
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
                        st.result_is_error = true;
                        st.final_error = "cancelled".to_string();
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
                        st.result_is_error = true;
                        st.final_error = "cancelled".to_string();
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
        
        let (final_status, output, error) = if st.timed_out {
            ("timeout".to_string(), String::new(), "run timed out".to_string())
        } else if st.result_is_error {
            ("failed".to_string(), deliverable, st.final_error.clone())
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
use crate::services::agent::models::Model;

pub struct AcpDiscoveryProvider {
    pub default_bin: String,
    pub client_name: String,
    pub extra_env: Vec<(&'static str, &'static str)>,
    pub acp_args: Vec<String>,
    pub select_auth_method: Option<Box<dyn Fn(Value, &[(String, String)]) -> Result<String, String> + Send + Sync>>,
}

pub async fn discover_acp_models(
    executable_path: Option<&str>,
    provider: AcpDiscoveryProvider,
) -> Result<Vec<Model>, String> {
    let bin = executable_path.unwrap_or(&provider.default_bin);
    let mut cmd = agent_command(bin);
    cmd.args(&provider.acp_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut envs: Vec<(String, String)> = std::env::vars().collect();
    for (k, v) in provider.extra_env {
        cmd.env(k, v);
        envs.push((k.to_string(), v.to_string()));
    }

    let mut child = cmd.spawn().map_err(|e| format!("spawn: {}", e))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut scanner = BufReader::new(stdout).lines();

    let mut next_id = 1;

    let init_msg = json!({
        "jsonrpc": "2.0",
        "id": next_id,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientInfo": { "name": provider.client_name, "version": "0.1.0" },
            "clientCapabilities": {}
        }
    });
    next_id += 1;

    async fn request_acp(
        stdin: &mut tokio::process::ChildStdin,
        scanner: &mut tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
        msg: Value,
        expected_id: i64,
    ) -> Result<Value, String> {
        let mut data = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        data.push(b'\n');
        stdin.write_all(&data).await.map_err(|e| e.to_string())?;

        while let Ok(Some(line)) = scanner.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(env) = serde_json::from_str::<Value>(trimmed) {
                if env.get("id").and_then(|i| i.as_i64()) == Some(expected_id) {
                    if let Some(err) = env.get("error") {
                        if !err.is_null() {
                            return Err(err.to_string());
                        }
                    }
                    if let Some(res) = env.get("result") {
                        return Ok(res.clone());
                    }
                    return Err("No result".to_string());
                }
            }
        }
        Err("EOF".to_string())
    }

    let init_result = request_acp(&mut stdin, &mut scanner, init_msg, next_id - 1).await?;

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("tempdir: {}", e))?;

    if let Some(select) = provider.select_auth_method {
        let method_id = select(init_result, &envs)?;
        let auth_msg = json!({
            "jsonrpc": "2.0",
            "id": next_id,
            "method": "authenticate",
            "params": {
                "methodId": method_id,
                "_meta": { "headless": true }
            }
        });
        next_id += 1;
        request_acp(&mut stdin, &mut scanner, auth_msg, next_id - 1).await?;
    }

    let session_msg = json!({
        "jsonrpc": "2.0",
        "id": next_id,
        "method": "session/new",
        "params": {
            "cwd": tmp_dir.path().to_string_lossy(),
            "mcpServers": []
        }
    });
    let session_result = request_acp(&mut stdin, &mut scanner, session_msg, next_id).await?;

    let _ = child.kill().await;

    Ok(parse_acp_session_new_models(&session_result))
}

fn parse_acp_session_new_models(raw: &Value) -> Vec<Model> {
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let extract_current = |v: &Value| -> String {
        v.get("currentModelId").and_then(|s| s.as_str())
         .or_else(|| v.get("current_model_id").and_then(|s| s.as_str()))
         .unwrap_or("")
         .trim()
         .to_string()
    };

    let extract_models = |available: &Value, current: &str, out: &mut Vec<Model>, seen: &mut std::collections::HashSet<String>| {
        if let Some(arr) = available.as_array() {
            for m in arr {
                let id = m.get("modelId").and_then(|s| s.as_str())
                          .or_else(|| m.get("model_id").and_then(|s| s.as_str()))
                          .unwrap_or("").trim().to_string();
                if id.is_empty() || seen.contains(&id) {
                    continue;
                }
                seen.insert(id.clone());
                
                let name = m.get("name").and_then(|s| s.as_str()).unwrap_or("").trim().to_string();
                let label = if name.is_empty() || name.eq_ignore_ascii_case("unknown") { id.clone() } else { name };
                
                let provider = id.find(':').map(|idx| id[..idx].to_string());

                out.push(Model {
                    id: id.clone(),
                    label,
                    provider,
                    default: Some(id == current),
                    service_tiers: vec![],
                    thinking: None,
                });
            }
        }
    };

    if let Some(models_obj) = raw.get("models") {
        let current = extract_current(models_obj);
        if let Some(available) = models_obj.get("availableModels").or_else(|| models_obj.get("available_models")) {
            extract_models(available, &current, &mut models, &mut seen);
        }
    }
    
    if models.is_empty() {
        if let Some(opts) = raw.get("configOptions").or_else(|| raw.get("config_options")) {
            if let Some(arr) = opts.as_array() {
                for opt in arr {
                    let id = opt.get("id").and_then(|s| s.as_str()).unwrap_or("").trim();
                    let category = opt.get("category").and_then(|s| s.as_str()).unwrap_or("").trim();
                    if !id.eq_ignore_ascii_case("model") && !category.eq_ignore_ascii_case("model") {
                        continue;
                    }
                    let current = extract_current(opt);
                    if let Some(options) = opt.get("options") {
                        if let Some(o_arr) = options.as_array() {
                            for choice in o_arr {
                                let val = choice.get("value").and_then(|s| s.as_str()).unwrap_or("").trim().to_string();
                                if val.is_empty() || seen.contains(&val) {
                                    continue;
                                }
                                seen.insert(val.clone());
                                let name = choice.get("name").and_then(|s| s.as_str()).unwrap_or("").trim().to_string();
                                let label = if name.is_empty() || name.eq_ignore_ascii_case("unknown") { val.clone() } else { name };
                                
                                let provider = val.find(':').map(|idx| val[..idx].to_string());
                                
                                models.push(Model {
                                    id: val.clone(),
                                    label,
                                    provider,
                                    default: Some(val == current),
                                    service_tiers: vec![],
                                    thinking: None,
                                });
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    models
}
