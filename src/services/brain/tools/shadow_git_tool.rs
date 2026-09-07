use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::path::Path;

pub struct ShadowGitTool;

impl ToolExecutor for ShadowGitTool {
    fn name(&self) -> &'static str {
        "shadow_git"
    }

    fn description(&self) -> &'static str {
        "Manage automatic backup snapshots of files.
Actions:
- 'list': List all available snapshots in the .shadow_git directory.
- 'restore': Restore a specific file from a snapshot. Requires SnapshotName."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": { "type": "string", "enum": ["list", "restore"] },
                "SnapshotName": { "type": "string", "description": "The name of the snapshot file to restore (e.g. 20260822_123456_main.rs)" },
                "TargetFile": { "type": "string", "description": "The absolute or relative path to restore the snapshot into." }
            },
            "required": ["Action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("Action")
            .and_then(|v| v.as_str())
            .ok_or("Missing Action")?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let shadow_dir = cwd.join(".shadow_git");

        if action == "list" {
            if !shadow_dir.exists() {
                return Ok(json!({ "snapshots": [] }));
            }
            let mut files = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&shadow_dir) {
                for entry in entries.flatten() {
                    files.push(entry.file_name().to_string_lossy().to_string());
                }
            }
            return Ok(json!({ "snapshots": files }));
        }

        if action == "restore" {
            let snap_name = args
                .get("SnapshotName")
                .and_then(|v| v.as_str())
                .ok_or("Missing SnapshotName")?;
            let target_file = args
                .get("TargetFile")
                .and_then(|v| v.as_str())
                .ok_or("Missing TargetFile")?;

            let snap_path = shadow_dir.join(snap_name);
            if !snap_path.exists() {
                return Err(format!("Snapshot {} not found", snap_name));
            }

            let target_path = Path::new(target_file);
            std::fs::copy(&snap_path, target_path).map_err(|e| format!("Restore failed: {}", e))?;

            return Ok(
                json!({ "success": true, "message": format!("Restored {} into {}", snap_name, target_file) }),
            );
        }

        Err(format!("Unknown action: {}", action))
    }
}
