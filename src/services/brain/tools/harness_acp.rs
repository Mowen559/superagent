//! DeepSeek Harness ACP (Agent Client Protocol) 跨进程桥接客户端与工具
//! 用于在 Rust 中拉起并控制 Node.js/TS 编写的 DeepSeek Harness ACP 服务器

use crate::domains::agent_cognitive::executor::ToolExecutor;
use lazy_static::lazy_static;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

lazy_static! {
    /// 全局 DeepSeek Harness ACP 客户端单例
    pub static ref GLOBAL_HARNESS_CLIENT: Arc<Mutex<Option<HarnessAcpClient>>> = Arc::new(Mutex::new(None));
}

/// 跨进程 ACP 客户端结构体
pub struct HarnessAcpClient {
    stdin: Arc<Mutex<ChildStdin>>,
    request_counter: u64,
    session_id: String,
    agent_info: Value,
    responses: Arc<Mutex<HashMap<u64, Value>>>,
    updates: Arc<Mutex<Vec<Value>>>,
    _child: Child,
}

impl HarnessAcpClient {
    /// 启动 Harness ACP 进程并建立 ACP 客户端会话
    pub fn start(harness_dir_override: Option<&str>) -> Result<Self, String> {
        let harness_dir = harness_dir_override.unwrap_or(r"D:\DeepSeek\deepseek-harness");
        let harness_path = Path::new(harness_dir);

        if !harness_path.exists() {
            return Err(format!("Harness directory not found at: {}", harness_dir));
        }

        let script_rel_path = r"packages\examples\acp-demo\src\bin.ts";
        let config_rel_path = r"examples\acp-agent\cordis.yml";

        let api_key =
            std::env::var("DEEPSEEK_API_KEY").unwrap_or_else(|_| "sk-dummy-test-key".to_string());

        let mut child = Command::new("node")
            .args(&[
                "--import",
                "tsx",
                script_rel_path,
                "--config",
                config_rel_path,
            ])
            .current_dir(harness_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env("DEEPSEEK_API_KEY", api_key)
            .env("DSH_PERMISSION_MODE", "danger-full-access")
            .spawn()
            .map_err(|e| format!("Failed to spawn Harness ACP node process: {}", e))?;

        let stdin = child.stdin.take().ok_or("Failed to open ChildStdin")?;
        let stdout = child.stdout.take().ok_or("Failed to open ChildStdout")?;

        let stdin_arc = Arc::new(Mutex::new(stdin));
        let responses = Arc::new(Mutex::new(HashMap::new()));
        let updates = Arc::new(Mutex::new(Vec::new()));

        let stdin_clone_for_reader = stdin_arc.clone();
        let responses_clone = responses.clone();
        let updates_clone = updates.clone();

        // 启动后台线程并发监听 stdout 的 JSON-RPC 消息
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line_buf = String::new();

            loop {
                line_buf.clear();
                match reader.read_line(&mut line_buf) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let trimmed = line_buf.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                            // 1. 处理 Agent 主动向 Client 发起请求 (如 session/request_permission)
                            if val.get("method").is_some() && val.get("id").is_some() {
                                let method = val["method"].as_str().unwrap_or("");
                                let req_id = &val["id"];

                                if method == "session/request_permission" {
                                    let resp_payload = json!({
                                        "jsonrpc": "2.0",
                                        "id": req_id,
                                        "result": {
                                            "outcome": {
                                                "kind": "allow_once"
                                            }
                                        }
                                    });
                                    if let Ok(mut writer) = stdin_clone_for_reader.lock() {
                                        let _ = writeln!(writer, "{}", resp_payload.to_string());
                                        let _ = writer.flush();
                                    }
                                } else {
                                    // 给其它请求提供默认 Success 响应
                                    let resp_payload = json!({
                                        "jsonrpc": "2.0",
                                        "id": req_id,
                                        "result": {}
                                    });
                                    if let Ok(mut writer) = stdin_clone_for_reader.lock() {
                                        let _ = writeln!(writer, "{}", resp_payload.to_string());
                                        let _ = writer.flush();
                                    }
                                }
                            }
                            // 2. 处理 Notification (如 session/update)
                            else if val.get("method").is_some() {
                                if let Ok(mut up_vec) = updates_clone.lock() {
                                    up_vec.push(val);
                                }
                            }
                            // 3. 处理 Response (只有 id 和 result/error)
                            else if let Some(id_val) = val.get("id") {
                                let numeric_id = id_val.as_u64().or_else(|| {
                                    id_val.as_str().and_then(|s| s.parse::<u64>().ok())
                                });
                                if let Some(id) = numeric_id {
                                    if let Ok(mut map) = responses_clone.lock() {
                                        map.insert(id, val);
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut client = Self {
            stdin: stdin_arc,
            request_counter: 1,
            session_id: String::new(),
            agent_info: json!({}),
            responses,
            updates,
            _child: child,
        };

        // 执行 ACP initialize
        let init_resp = client.send_request_and_wait(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {}
            }),
            10,
        )?;

        let agent_info = init_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| json!({}));
        client.agent_info = agent_info;

        // 执行 session/new
        let session_resp = client.send_request_and_wait(
            "session/new",
            json!({
                "cwd": harness_dir,
                "mcpServers": []
            }),
            10,
        )?;

        let session_id = session_resp
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(|s| s.as_str())
            .ok_or_else(|| {
                format!(
                    "Failed to obtain sessionId from response: {:?}",
                    session_resp
                )
            })?
            .to_string();

        client.session_id = session_id;

        Ok(client)
    }

    /// 发送 JSON-RPC 通知 (Notification)
    pub fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let msg = payload.to_string();
        let mut writer = self.stdin.lock().map_err(|e| e.to_string())?;
        writeln!(writer, "{}", msg).map_err(|e| format!("Write error: {}", e))?;
        writer.flush().map_err(|e| format!("Flush error: {}", e))?;
        Ok(())
    }

    /// 发送 JSON-RPC 请求并阻塞等待指定超时秒数
    pub fn send_request_and_wait(
        &mut self,
        method: &str,
        params: Value,
        timeout_secs: u64,
    ) -> Result<Value, String> {
        let id = self.request_counter;
        self.request_counter += 1;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        {
            let mut writer = self.stdin.lock().map_err(|e| e.to_string())?;
            writeln!(writer, "{}", payload.to_string())
                .map_err(|e| format!("Write error: {}", e))?;
            writer.flush().map_err(|e| format!("Flush error: {}", e))?;
        }

        let loops = timeout_secs * 20; // 50ms interval
        for _ in 0..loops {
            thread::sleep(Duration::from_millis(50));
            if let Ok(mut map) = self.responses.lock() {
                if let Some(resp) = map.remove(&id) {
                    if let Some(err_val) = resp.get("error") {
                        return Err(format!("ACP RPC Error: {:?}", err_val));
                    }
                    return Ok(resp);
                }
            }
        }

        Err(format!(
            "ACP Request '{}' (id={}) timed out after {} seconds",
            method, id, timeout_secs
        ))
    }

    /// 获取当前的 Session ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 获取 Agent 信息
    pub fn agent_info(&self) -> &Value {
        &self.agent_info
    }

    /// 清空并返回这段时间内接收到的 Updates 列表
    pub fn drain_updates(&self) -> Vec<Value> {
        if let Ok(mut up) = self.updates.lock() {
            std::mem::take(&mut *up)
        } else {
            Vec::new()
        }
    }
}

/// Harness ACP 桥接工具
pub struct HarnessAcpClientTool;

impl ToolExecutor for HarnessAcpClientTool {
    fn name(&self) -> &'static str {
        "harness_acp"
    }

    fn description(&self) -> &'static str {
        "Bridge tool for interacting with the DeepSeek Harness ACP (Agent Client Protocol) server across process boundaries.
Actions:
- 'start': Boot or verify the Harness ACP server, initialize session.
- 'prompt': Route a turn prompt to the active Harness session and wait for response/updates.
- 'status': Inspect current connection status, active session ID, and agent capability metadata.
- 'cancel': Send a cancellation signal for the currently running session prompt turn."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": {
                    "type": "string",
                    "enum": ["start", "prompt", "status", "cancel"],
                    "description": "Action to perform."
                },
                "Prompt": {
                    "type": "string",
                    "description": "Prompt content to send to Harness ACP server (Required for 'prompt' Action)."
                },
                "HarnessPath": {
                    "type": "string",
                    "description": "Optional custom path to the deepseek-harness directory (defaults to D:\\DeepSeek\\deepseek-harness)."
                }
            },
            "required": ["Action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("Action")
            .and_then(|v| v.as_str())
            .ok_or("Missing Action")?;
        let harness_path = args.get("HarnessPath").and_then(|v| v.as_str());

        match action {
            "start" => {
                let mut client_lock = GLOBAL_HARNESS_CLIENT.lock().map_err(|e| e.to_string())?;
                if client_lock.is_none() {
                    let client = HarnessAcpClient::start(harness_path)?;
                    *client_lock = Some(client);
                }
                let client = client_lock.as_ref().unwrap();
                Ok(json!({
                    "success": true,
                    "status": "connected",
                    "sessionId": client.session_id(),
                    "agentInfo": client.agent_info()
                }))
            }
            "status" => {
                let client_lock = GLOBAL_HARNESS_CLIENT.lock().map_err(|e| e.to_string())?;
                if let Some(client) = client_lock.as_ref() {
                    Ok(json!({
                        "connected": true,
                        "sessionId": client.session_id(),
                        "agentInfo": client.agent_info()
                    }))
                } else {
                    Ok(json!({
                        "connected": false,
                        "message": "Harness ACP process is not running. Call 'start' action first."
                    }))
                }
            }
            "prompt" => {
                let prompt_text = args
                    .get("Prompt")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing Prompt parameter for 'prompt' action")?;

                // Ensure client is started
                {
                    let mut client_lock =
                        GLOBAL_HARNESS_CLIENT.lock().map_err(|e| e.to_string())?;
                    if client_lock.is_none() {
                        let client = HarnessAcpClient::start(harness_path)?;
                        *client_lock = Some(client);
                    }
                }

                let mut client_lock = GLOBAL_HARNESS_CLIENT.lock().map_err(|e| e.to_string())?;
                let client = client_lock
                    .as_mut()
                    .ok_or("Failed to access Harness ACP client")?;

                let session_id = client.session_id().to_string();

                let prompt_params = json!({
                    "sessionId": session_id,
                    "prompt": [
                        {
                            "type": "text",
                            "text": prompt_text
                        }
                    ]
                });

                // Send prompt request and wait for stopReason (timeout 120s)
                let resp = client.send_request_and_wait("session/prompt", prompt_params, 120)?;
                let updates = client.drain_updates();

                Ok(json!({
                    "success": true,
                    "response": resp.get("result"),
                    "updates_count": updates.len(),
                    "updates": updates
                }))
            }
            "cancel" => {
                let mut client_lock = GLOBAL_HARNESS_CLIENT.lock().map_err(|e| e.to_string())?;
                let client = client_lock
                    .as_mut()
                    .ok_or("Harness ACP process is not running.")?;
                let session_id = client.session_id().to_string();

                client.send_notification("session/cancel", json!({ "sessionId": session_id }))?;

                Ok(json!({
                    "success": true,
                    "message": format!("Cancellation signal sent for session {}", session_id)
                }))
            }
            _ => Err(format!("Unknown Action: {}", action)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_acp_client_schema() {
        let tool = HarnessAcpClientTool;
        assert_eq!(tool.name(), "harness_acp");
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
        assert!(schema["properties"].get("Action").is_some());
    }

    #[test]
    fn test_harness_acp_status_unstarted() {
        let tool = HarnessAcpClientTool;
        let res = tool.execute(json!({ "Action": "status" })).unwrap();
        assert_eq!(res["connected"], false);
    }
}

impl Drop for HarnessAcpClient {
    fn drop(&mut self) {
        let _ = self._child.kill();
        let _ = self._child.wait();
    }
}
