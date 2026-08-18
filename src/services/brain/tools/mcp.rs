//! MCP 代理工具 (MCP Proxy Tool)
//! 允许大脑 B (Superagent) 调用外部 MCP Server (如 codebase-memory-mcp)

use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::io::{Write, Read};

/// MCP 代理调用工具
pub struct McpProxyTool;

impl ToolExecutor for McpProxyTool {
    fn name(&self) -> &'static str {
        "mcp_proxy"
    }

    fn description(&self) -> &'static str {
        "Call an external MCP server tool (e.g. codebase-memory-mcp) using JSON-RPC."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server_cmd": {
                    "type": "string",
                    "description": "The command to run the MCP server (e.g. 'node build/index.js')"
                },
                "method": {
                    "type": "string",
                    "description": "The MCP method to call (e.g. 'tools/call')"
                },
                "params": {
                    "type": "object",
                    "description": "The JSON-RPC params object for the method call"
                }
            },
            "required": ["server_cmd", "method", "params"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let server_cmd = args.get("server_cmd").and_then(|v| v.as_str()).unwrap_or("");
        let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let params = args.get("params").cloned().unwrap_or(json!({}));

        if server_cmd.is_empty() || method.is_empty() {
            return Err("Missing 'server_cmd' or 'method'".into());
        }

        eprintln!("[Brain B] Calling MCP server {} method {}", server_cmd, method);

        // 这里仅作为一个简化的单次调用的封装（真实 MCP 环境一般是长连接）
        // 组装 JSON-RPC 2.0 Payload
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        let mut child = Command::new("cmd")
            .arg("/c")
            .arg(server_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn MCP server: {}", e))?;

        if let Some(mut stdin) = child.stdin.take() {
            let req_str = serde_json::to_string(&payload).unwrap() + "\n";
            stdin.write_all(req_str.as_bytes()).map_err(|e| e.to_string())?;
        }

        let mut stdout_str = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            // 设置超时读取或仅读取第一行 (由于 MCP 是 newline delimited JSON)
            // 这里为了简单起见读完所有
            stdout.read_to_string(&mut stdout_str).unwrap_or(0);
        }

        let _ = child.kill(); // 结束进程

        Ok(json!({
            "status": "executed",
            "response_raw": stdout_str
        }))
    }
}
