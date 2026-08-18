//! 子智能体委派工具 (Subagent Tool)
//! 允许大模型 (Brain B) 孵化另一个 Brain B (子智能体) 来执行并行的复杂任务。

use crate::services::brain::executor::ToolExecutor;
use crate::services::brain::ipc::{IpcClient, IpcMessage};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::process::Command;

pub struct InvokeSubagentTool;

impl ToolExecutor for InvokeSubagentTool {
    fn name(&self) -> &'static str {
        "invoke_subagent"
    }

    fn description(&self) -> &'static str {
        "Invoke a subagent to handle a complex sub-task in a separate context. Use this to parallelize work or prevent cluttering your current context with long research tasks."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Prompt": {
                    "type": "string",
                    "description": "A clear, actionable task description for the subagent."
                },
                "Role": {
                    "type": "string",
                    "description": "A short role description for the subagent, e.g., 'Codebase Researcher'."
                }
            },
            "required": ["Prompt", "Role"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let prompt = args.get("Prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let role = args.get("Role").and_then(|v| v.as_str()).unwrap_or("Subagent").to_string();
        
        let sub_prompt = format!("You are a subagent with the role: {}.\nYour task is: {}", role, prompt);
        
        // 阻塞式的在当前的 tokio worker 线程中运行异步的子进程逻辑
        let handle = tokio::runtime::Handle::current();
        let result = handle.block_on(async move {
            let mut child = Command::new("cargo")
                .arg("run")
                .arg("--bin")
                .arg("superagent")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| format!("Failed to spawn subagent process: {}", e))?;

            let mut ipc_client = IpcClient::new(&mut child);

            // Send Prompt
            ipc_client.send_message(IpcMessage::Prompt(sub_prompt)).await
                .map_err(|e| format!("Failed to send prompt to subagent: {}", e))?;

            // Wait for Result or Error
            loop {
                match ipc_client.read_message().await {
                    Ok(msg) => match msg {
                        IpcMessage::Result(res) => {
                            let _ = child.kill().await;
                            return Ok(res);
                        }
                        IpcMessage::Error(err) => {
                            let _ = child.kill().await;
                            return Err(format!("Subagent failed: {}", err));
                        }
                        // Ignore other messages like AskUser or ReloadRequest from subagents (or handle them appropriately)
                        // A true implementation might forward AskUser to Brain A!
                        _ => {}
                    },
                    Err(e) => {
                        let _ = child.kill().await;
                        return Err(format!("Subagent IPC error: {}", e));
                    }
                }
            }
        });

        match result {
            Ok(output) => Ok(json!({ "status": "success", "result": output })),
            Err(e) => Err(e),
        }
    }
}
