use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 检查点创建工具 (Create Checkpoint)
pub struct CreateCheckpointTool {
    pub workspace_root: PathBuf,
}

impl CreateCheckpointTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for CreateCheckpointTool {
    fn name(&self) -> &'static str {
        "create_checkpoint"
    }

    fn description(&self) -> &'static str {
        "Create a named Git checkpoint for the entire workspace. Use this before starting a risky refactoring or speculative bugfix to ensure a safe rollback point."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "A concise description of the current stable state (e.g. 'Before refactoring AuthController')."
                }
            },
            "required": ["description"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: create_checkpoint]
- Role: Safety Milestone (安全里程碑)
- Usage: Call `create_checkpoint` before starting speculative multi-file refactoring or major architectural changes.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Manual checkpoint");

        let cwd_str = self.workspace_root.to_string_lossy().to_string();
        let checkpoint = crate::services::git_service::create_checkpoint(&cwd_str, description)?;

        Ok(json!({
            "status": "success",
            "checkpoint_id": checkpoint.id,
            "commit_hash": checkpoint.commit_hash,
            "description": checkpoint.description,
            "timestamp": checkpoint.timestamp
        }))
    }
}

/// 假设树死锁回溯与检查点回滚工具 (Revert to Checkpoint)
pub struct RevertToCheckpointTool {
    pub workspace_root: PathBuf,
}

impl RevertToCheckpointTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for RevertToCheckpointTool {
    fn name(&self) -> &'static str {
        "revert_to_checkpoint"
    }

    fn description(&self) -> &'static str {
        "Rollback workspace files to a previously saved checkpoint. Use this when a speculative refactoring or bugfix attempt has failed (or hit a dead end) and you need a clean slate to try an alternative approach."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "checkpoint_id": {
                    "type": "string",
                    "description": "Optional checkpoint ID (e.g. 'cp-1234abcd') or commit hash prefix. If omitted, rolls back to the most recent checkpoint."
                },
                "reason": {
                    "type": "string",
                    "description": "Explanation of why this hypothesis failed and why rolling back is necessary (e.g. 'Strategy A causes cascading compiler errors; abandoning to try Strategy B')."
                }
            },
            "required": ["reason"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: revert_to_checkpoint]
- Role: Hypothesis Backtracking & Clean Rollback (假设推翻与安全回滚)
- Usage: When multiple attempts to fix a bug or refactor code lead to cascading errors, DO NOT continue digging into a dead end. Use `revert_to_checkpoint` to cleanly restore the workspace to a working state, then attempt an alternative strategy.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let checkpoint_id = args.get("checkpoint_id").and_then(|v| v.as_str());
        let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("Abandoning dead end hypothesis");

        let cwd_str = self.workspace_root.to_string_lossy().to_string();
        let message = crate::services::git_service::revert_to_checkpoint(&cwd_str, checkpoint_id)?;

        Ok(json!({
            "status": "success",
            "message": message,
            "reason": reason
        }))
    }
}
