//! 交互工具 (Interaction Tools)
//! 允许大模型在推演过程中主动打断，向用户提问并获取答案。

use crate::services::brain::executor::ToolExecutor;
use crate::services::brain::ipc::{IpcMessage, send_to_hub};
use serde_json::{json, Value};
use std::io::{self, BufRead};

/// 人工介入提问工具
pub struct AskUserTool;

impl ToolExecutor for AskUserTool {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn description(&self) -> &'static str {
        "Ask the user a question and wait for their response. Use this when you are unsure, need permission for a destructive action, or require information you cannot find."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Question": {
                    "type": "string",
                    "description": "The exact question to display to the user."
                }
            },
            "required": ["Question"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let question = args
            .get("Question")
            .and_then(|v| v.as_str())
            .unwrap_or("Waiting for user input...")
            .to_string();

        // 1. 发送提问给 Hub
        send_to_hub(IpcMessage::AskUser { question });

        // 2. 阻塞读取，直到 Hub 返回 UserResponse
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let mut buffer = String::new();
        
        loop {
            buffer.clear();
            match handle.read_line(&mut buffer) {
                Ok(0) => return Err("EOF while waiting for user response".to_string()),
                Ok(_) => {
                    if let Ok(msg) = serde_json::from_str::<IpcMessage>(&buffer) {
                        if let IpcMessage::UserResponse(answer) = msg {
                            return Ok(json!({
                                "status": "success",
                                "response": answer
                            }));
                        } else {
                            // 虽然理应收到 UserResponse，但如果收到别的消息，记录错误或忽略
                            eprintln!("[Brain B] Warning: Expected UserResponse, got {:?}", msg);
                        }
                    } else {
                        eprintln!("[Brain B] Warning: Failed to parse IPC message: {}", buffer);
                    }
                }
                Err(e) => return Err(format!("Failed to read stdin: {}", e)),
            }
        }
    }
}
