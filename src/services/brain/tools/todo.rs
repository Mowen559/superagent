//! 任务追踪工具 (Todo Management)
//! 允许大模型显式地写入和读取任务进度。

use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::{json, Value};
use std::fs;

pub struct ManageTaskTool {
    pub workspace_root: std::path::PathBuf,
}
impl ManageTaskTool {
    pub fn new(w: std::path::PathBuf) -> Self {
        Self { workspace_root: w }
    }
}

impl ToolExecutor for ManageTaskTool {
    fn name(&self) -> &'static str {
        "manage_task"
    }

    fn description(&self) -> &'static str {
        "Manage your execution checklist (todo.md). Use this tool to write a plan, mark tasks as complete, or review current progress. Actions: 'read', 'write', 'append'."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": {
                    "type": "string",
                    "enum": ["read", "write", "append"],
                    "description": "The action to perform on the todo list."
                },
                "Content": {
                    "type": "string",
                    "description": "The content to write or append. Leave empty if reading."
                }
            },
            "required": ["Action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("Action")
            .or_else(|| args.get("action"))
            .or_else(|| args.get("operation"))
            .and_then(|v| v.as_str())
            .unwrap_or("read")
            .to_ascii_lowercase();

        let content = args
            .get("Content")
            .or_else(|| args.get("content"))
            .or_else(|| args.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let file_path = std::env::current_dir()
            .unwrap_or_default()
            .join(".superagent_todo.md");

        match action.as_str() {
            "read" => {
                let current = fs::read_to_string(&file_path)
                    .unwrap_or_else(|_| "No todo list found. Start by writing one.".to_string());
                Ok(json!({ "status": "success", "content": current }))
            }
            "write" => {
                fs::write(&file_path, content)
                    .map_err(|e| format!("Failed to write todo list: {}", e))?;
                let has_unfinished = content.lines().any(|l| {
                    let t = l.trim();
                    t.starts_with("- [ ]") || t.starts_with("* [ ]") || t.contains("[in_progress]") || t.contains("[pending]")
                });
                let message = if !has_unfinished && !content.trim().is_empty() {
                    "All tasks marked as completed. You may now conclude the turn and present the final summary to the user. Do NOT call manage_task again."
                } else {
                    "Todo list updated."
                };
                Ok(json!({ "status": "success", "message": message }))
            }
            "append" => {
                use std::io::Write;
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&file_path)
                    .map_err(|e| format!("Failed to open todo list: {}", e))?;

                writeln!(file, "{}", content)
                    .map_err(|e| format!("Failed to append to todo list: {}", e))?;

                Ok(json!({ "status": "success", "message": "Content appended to todo list." }))
            }
            "clear" | "reset" => {
                if file_path.exists() {
                    let _ = fs::remove_file(&file_path);
                }
                Ok(json!({ "status": "success", "message": "Todo list cleared. Checklist is now empty." }))
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }
}
