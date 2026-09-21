//! LSP 大模型调度工具 (LSP Tool)
//! 让大模型可以使用语言服务器查询定义、悬停文档和引用

use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::{json, Value};
use std::path::Path;

/// LSP 语义分析工具
pub struct LspTool;

impl ToolExecutor for LspTool {
    fn name(&self) -> &'static str {
        "lsp"
    }

    fn description(&self) -> &'static str {
        "Query a Language Server (like rust-analyzer) for code intelligence.
Actions:
- 'start': Start the LSP server (e.g., Command='rust-analyzer', RootUri='file:///path/to/project').
- 'hover': Get documentation for a symbol (File, Line, Character).
- 'definition': Go to definition of a symbol (File, Line, Character).
- 'references': Find all references of a symbol (File, Line, Character).

NOTE: Line and Character are 0-indexed. You MUST use ReadFileTool to read the file first to find the exact Line and Character of the symbol before calling hover/definition/references."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": {
                    "type": "string",
                    "enum": ["start", "hover", "definition", "references"],
                },
                "Command": {
                    "type": "string",
                    "description": "Command to start LSP (e.g. 'rust-analyzer'). Required for 'start'."
                },
                "RootUri": {
                    "type": "string",
                    "description": "Project root URI (e.g. 'file:///D:/project'). Required for 'start'."
                },
                "File": {
                    "type": "string",
                    "description": "Absolute file path. Required for queries."
                },
                "Line": {
                    "type": "integer",
                    "description": "0-indexed line number. Required for queries."
                },
                "Character": {
                    "type": "integer",
                    "description": "0-indexed character offset. Required for queries."
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

        if action == "start" {
            let cmd = args
                .get("Command")
                .and_then(|v| v.as_str())
                .unwrap_or("rust-analyzer");
            let root_uri = args.get("RootUri").and_then(|v| v.as_str()).unwrap_or("");
            if root_uri.is_empty() {
                return Err("Missing RootUri for start".to_string());
            }

            eprintln!("[Brain B] Starting LSP Server '{}' at '{}'", cmd, root_uri);
            let client =
                crate::services::brain::tools::lsp_client::LspClient::start(cmd, &[], root_uri)?;

            if let Ok(mut global) =
                crate::services::brain::tools::lsp_client::GLOBAL_LSP_CLIENT.lock()
            {
                *global = Some(client);
            }
            return Ok(json!({ "success": true, "message": "LSP Server started." }));
        }

        // For queries, we need an active client
        let mut client_lock = crate::services::brain::tools::lsp_client::GLOBAL_LSP_CLIENT
            .lock()
            .unwrap();
        if client_lock.is_none() {
            return Err("LSP Server is not started. Please call 'start' Action first.".to_string());
        }
        let client = client_lock.as_mut().unwrap();

        let file_path = args
            .get("File")
            .and_then(|v| v.as_str())
            .ok_or("Missing File")?;
        let line = args
            .get("Line")
            .and_then(|v| v.as_u64())
            .ok_or("Missing Line")?;
        let character = args
            .get("Character")
            .and_then(|v| v.as_u64())
            .ok_or("Missing Character")?;

        let uri = if file_path.starts_with("file://") {
            file_path.to_string()
        } else {
            // Convert absolute path to URI
            let path = Path::new(file_path);
            format!("file:///{}", path.display().to_string().replace("\\", "/"))
        };

        let position_params = json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": line,
                "character": character
            }
        });

        match action {
            "hover" => {
                let resp = client.send_request_and_wait("textDocument/hover", position_params)?;
                Ok(resp)
            }
            "definition" => {
                let resp =
                    client.send_request_and_wait("textDocument/definition", position_params)?;
                Ok(resp)
            }
            "references" => {
                let mut params = position_params;
                params["context"] = json!({ "includeDeclaration": true });
                let resp = client.send_request_and_wait("textDocument/references", params)?;
                Ok(resp)
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }
}
