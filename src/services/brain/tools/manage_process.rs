//! 管理后台进程工具 (Manage Process)

use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};

/// 后台进程管理工具 (Manage Process Tool)
pub struct ManageProcessTool;

impl ToolExecutor for ManageProcessTool {
    fn name(&self) -> &'static str {
        "manage_process"
    }

    fn description(&self) -> &'static str {
        "Manage background terminal processes. Use this to list, read logs, or kill processes spawned with RunInBackground=true."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": {
                    "type": "string",
                    "enum": ["list", "logs", "kill"],
                    "description": "The action to perform: 'list' (all background processes), 'logs' (get stdout/stderr), 'kill' (terminate process)."
                },
                "ProcessId": {
                    "type": "string",
                    "description": "The target process ID (required for 'logs' and 'kill')."
                }
            },
            "required": ["Action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("Action")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'Action' argument")?;

        match action {
            "list" => {
                let processes = crate::services::brain::tools::process_manager::list_processes();
                Ok(json!({ "processes": processes }))
            }
            "logs" => {
                let process_id = args.get("ProcessId").and_then(|v| v.as_str()).ok_or("Missing ProcessId")?;
                if let Some(logs) = crate::services::brain::tools::process_manager::get_process_logs(process_id) {
                    Ok(json!({ "process_id": process_id, "logs": logs }))
                } else {
                    Err(format!("Process {} not found.", process_id))
                }
            }
            "kill" => {
                let process_id = args.get("ProcessId").and_then(|v| v.as_str()).ok_or("Missing ProcessId")?;
                let success = crate::services::brain::tools::process_manager::kill_process(process_id);
                if success {
                    Ok(json!({ "success": true, "message": format!("Process {} killed.", process_id) }))
                } else {
                    Err(format!("Process {} not found or already dead.", process_id))
                }
            }
            _ => Err(format!("Unknown Action: {}", action)),
        }
    }
}
