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

        let file_path = self.workspace_root.join(".superagent_todo.md");

        match action.as_str() {
            "read" => {
                let current = fs::read_to_string(&file_path)
                    .unwrap_or_else(|_| "No todo list found. Start by writing one.".to_string());
                Ok(json!({ "status": "success", "content": current, "file_path":file_path }))
            }
            "write" => {
                fs::write(&file_path, content)
                    .map_err(|e| format!("Failed to write todo list: {}", e))?;
                let has_unfinished = content.lines().any(|l| {
                    let t = l.trim();
                    t.starts_with("- [ ]") || t.starts_with("* [ ]") || t.contains("[in_progress]") || t.contains("[pending]")
                });
                let message = if !has_unfinished && !content.trim().is_empty() {
                    "Checklist notes are checked; this is not execution evidence. If plan_task exists, complete its subgoals with actual receipt_ids via update_task_progress. Conclude only when the required artifact and verification evidence are satisfied. Do not rewrite these notes again."
                } else {
                    "Todo list updated."
                };
                Ok(json!({ "status": "success", "message": message, "file_path":file_path }))
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

                Ok(json!({ "status": "success", "message": "Content appended to todo list.", "file_path":file_path }))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checklist_reads_and_writes_are_bound_to_the_selected_workspace() {
        let root=tempfile::tempdir().unwrap();let other=tempfile::tempdir().unwrap();
        let name=".superagent_todo.md";
        fs::write(root.path().join(name),"selected workspace").unwrap();
        fs::write(other.path().join(name),"other workspace").unwrap();
        let launcher=std::env::current_dir().unwrap().join(name);
        let before=fs::read(&launcher).ok();
        let tool=ManageTaskTool::new(root.path().to_path_buf());
        assert_eq!(tool.execute(json!({"Action":"read"})).unwrap()["content"],"selected workspace");
        tool.execute(json!({"Action":"write","Content":"- [ ] scoped change"})).unwrap();
        tool.execute(json!({"Action":"append","Content":"- [x] verified scope"})).unwrap();
        assert!(fs::read_to_string(root.path().join(name)).unwrap().contains("verified scope"));
        assert_eq!(fs::read_to_string(other.path().join(name)).unwrap(),"other workspace");
        assert_eq!(fs::read(&launcher).ok(),before,"launcher checklist must not change");
    }
}
