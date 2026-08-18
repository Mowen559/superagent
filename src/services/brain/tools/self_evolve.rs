//! 自举重载工具 (Self-Evolve Tool)
//! 当大模型修改了自身代码后，调用此工具向守护进程 (Brain A) 发出重新编译并拉起新版本的请求。

use crate::services::brain::executor::ToolExecutor;
use crate::services::brain::ipc::{IpcMessage, send_to_hub};
use serde_json::{json, Value};

/// 进程重载工具 (Request Reload)
pub struct RequestReloadTool;

impl ToolExecutor for RequestReloadTool {
    fn name(&self) -> &'static str {
        "request_reload"
    }

    fn description(&self) -> &'static str {
        "Request the supervisor (Brain A) to terminate the current agent (Brain B), recompile the project using 'cargo build', and restart the agent. Call this ONLY after modifying the agent's source code."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Reason": {
                    "type": "string",
                    "description": "The reason for the reload (e.g. 'Updated fs.rs to add delete capability')."
                }
            },
            "required": ["Reason"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let reason = args
            .get("Reason")
            .and_then(|v| v.as_str())
            .unwrap_or("No reason provided");

        eprintln!("[Brain B] Self-evolution triggered. Reason: {}", reason);
        
        // 发送重载信令给大本营 (Brain A)
        send_to_hub(IpcMessage::ReloadRequest);

        // 返回一段成功发起重载的信息，由于整个进程很快会被 Brain A 杀掉，这个返回值主要用于最后一次记忆记录
        Ok(json!({
            "status": "Reload requested",
            "message": "Supervisor is recompiling and restarting the agent..."
        }))
    }
}
