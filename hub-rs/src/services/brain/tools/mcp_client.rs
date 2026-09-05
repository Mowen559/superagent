use crate::services::brain::executor::ToolExecutor;
use lazy_static::lazy_static;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

lazy_static! {
    /// 全局 MCP 客户端单例字典 (ServerName -> Client)
    pub static ref GLOBAL_MCP_CLIENTS: Arc<Mutex<HashMap<String, McpClient>>> = Arc::new(Mutex::new(HashMap::new()));
}

/// 标准 MCP 客户端结构体
pub struct McpClient {
    stdin: Arc<Mutex<ChildStdin>>,
    request_counter: u64,
    responses: Arc<Mutex<HashMap<u64, Value>>>,
    tools_cache: Option<Value>,
    _child: Child,
}

impl McpClient {
    /// 启动 MCP 服务器进程并建立客户端会话
    pub fn start(command: &str, args: &[&str]) -> Result<Self, String> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn MCP server process ({}): {}", command, e))?;

        let stdin = child.stdin.take().ok_or("Failed to open ChildStdin")?;
        let stdout = child.stdout.take().ok_or("Failed to open ChildStdout")?;
        let stderr = child.stderr.take().ok_or("Failed to open ChildStderr")?;

        let responses = Arc::new(Mutex::new(HashMap::new()));
        let responses_clone = responses.clone();

        // Background thread to parse JSON-RPC over stdout
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(l) = line {
                    if let Ok(val) = serde_json::from_str::<Value>(&l) {
                        if let Some(id) = val.get("id").and_then(|v| v.as_u64()) {
                            if let Ok(mut map) = responses_clone.lock() {
                                map.insert(id, val);
                            }
                        }
                    }
                }
            }
        });

        // Background thread for stderr logging
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(l) = line {
                    eprintln!("[MCP Server STDERR] {}", l);
                }
            }
        });

        let mut client = Self {
            stdin: Arc::new(Mutex::new(stdin)),
            request_counter: 1,
            responses,
            tools_cache: None,
            _child: child,
        };

        // Initialize MCP
        let init_resp = client.send_request_and_wait(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "SuperAgent-MCP-Client",
                    "version": "1.0.0"
                }
            }),
            10,
        )?;

        // Send initialized notification
        client.send_notification("notifications/initialized", json!({}))?;

        // Pre-fetch tools
        if let Ok(tools) = client.send_request_and_wait("tools/list", json!({}), 10) {
            client.tools_cache = Some(tools);
        }

        Ok(client)
    }

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
                        return Err(format!("MCP RPC Error: {:?}", err_val));
                    }
                    if let Some(result) = resp.get("result") {
                        return Ok(result.clone());
                    }
                    return Ok(resp);
                }
            }
        }

        Err(format!(
            "MCP Request {} (id={}) timed out after {} seconds",
            method, id, timeout_secs
        ))
    }
}

/// Standard Model Context Protocol (MCP) Bridge Tool
pub struct McpClientTool;

impl ToolExecutor for McpClientTool {
    fn name(&self) -> &'static str {
        "mcp_client"
    }

    fn description(&self) -> &'static str {
        "Bridge tool for interacting with external Model Context Protocol (MCP) servers (e.g. SQLite, GitHub, PostgreSQL).
Actions:
- 'start': Start an MCP server. Requires ServerName, Command, and Args.
- 'list_tools': List available tools on a specific MCP server. Requires ServerName.
- 'call_tool': Call a tool on a specific MCP server. Requires ServerName, ToolName, and ToolArgs."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": { "type": "string", "enum": ["start", "list_tools", "call_tool"] },
                "ServerName": { "type": "string", "description": "Unique identifier for this MCP server instance." },
                "Command": { "type": "string", "description": "Command to start the MCP server (e.g. 'npx'). Required for 'start'." },
                "Args": { "type": "array", "items": { "type": "string" }, "description": "Arguments for the command. Required for 'start'." },
                "ToolName": { "type": "string", "description": "Name of the MCP tool to call. Required for 'call_tool'." },
                "ToolArgs": { "type": "object", "description": "Arguments to pass to the MCP tool. Required for 'call_tool'." }
            },
            "required": ["Action", "ServerName"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("Action")
            .and_then(|v| v.as_str())
            .ok_or("Missing Action")?;
        let server_name = args
            .get("ServerName")
            .and_then(|v| v.as_str())
            .ok_or("Missing ServerName")?;

        match action {
            "start" => {
                let mut clients = GLOBAL_MCP_CLIENTS.lock().map_err(|e| e.to_string())?;
                if clients.contains_key(server_name) {
                    return Ok(json!({ "status": "already_running" }));
                }

                let command = args
                    .get("Command")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing Command")?;
                let cmd_args_val = args
                    .get("Args")
                    .and_then(|v| v.as_array())
                    .ok_or("Missing Args")?;
                let mut cmd_args = Vec::new();
                for a in cmd_args_val {
                    if let Some(s) = a.as_str() {
                        cmd_args.push(s);
                    }
                }

                let client = McpClient::start(command, &cmd_args)?;
                clients.insert(server_name.to_string(), client);

                Ok(json!({ "status": "started", "server_name": server_name }))
            }
            "list_tools" => {
                let mut clients = GLOBAL_MCP_CLIENTS.lock().map_err(|e| e.to_string())?;
                let client = clients
                    .get_mut(server_name)
                    .ok_or("MCP Server not started")?;

                let tools = client.send_request_and_wait("tools/list", json!({}), 10)?;
                client.tools_cache = Some(tools.clone());
                Ok(tools)
            }
            "call_tool" => {
                let tool_name = args
                    .get("ToolName")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing ToolName")?;
                let tool_args = args.get("ToolArgs").cloned().unwrap_or(json!({}));

                let mut clients = GLOBAL_MCP_CLIENTS.lock().map_err(|e| e.to_string())?;
                let client = clients
                    .get_mut(server_name)
                    .ok_or("MCP Server not started")?;

                let req_params = json!({
                    "name": tool_name,
                    "arguments": tool_args
                });

                let result = client.send_request_and_wait("tools/call", req_params, 120)?;
                Ok(result)
            }
            _ => Err(format!("Unknown Action: {}", action)),
        }
    }
}
