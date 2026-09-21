//! Model-facing adapters for the durable subagent lifecycle runtime.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::subagent_runtime::{SubagentLaunchRequest, SubagentRuntime};

pub struct InvokeSubagentTool {
    runtime: Arc<SubagentRuntime>,
    parent_session_id: String,
}

impl InvokeSubagentTool {
    pub fn new(runtime: Arc<SubagentRuntime>, parent_session_id: impl Into<String>) -> Self {
        Self {
            runtime,
            parent_session_id: parent_session_id.into(),
        }
    }
}

impl ToolExecutor for InvokeSubagentTool {
    fn name(&self) -> &'static str {
        "invoke_subagent"
    }

    fn description(&self) -> &'static str {
        "Start a bounded asynchronous subagent. Completion is automatically delivered into the parent run; use the returned Subagent_ID for supervision."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Prompt": { "type": "string", "description": "A concrete, independently executable task." },
                "Role": { "type": "string", "description": "Short specialist role." },
                "SystemPrompt": { "type": "string", "description": "Optional isolated system instructions." },
                "Correlation_ID": { "type": "string", "description": "Optional idempotency key within this parent session." },
                "WorkspaceMode": { "type": "string", "enum": ["inherit", "worktree"], "description": "Optional workspace isolation: 'inherit' (default) shares parent workspace; 'worktree' spawns subagent in an isolated ephemeral Git worktree to eliminate concurrent file edit race conditions." },
                "TimeoutSeconds": { "type": "integer", "minimum": 1, "maximum": 86400 }
            },
            "required": ["Prompt", "Role"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let role = required_string(&args, "Role")?;
        let prompt = required_string(&args, "Prompt")?;
        let mut request = SubagentLaunchRequest::new(&self.parent_session_id, role, prompt);
        request.parent_depth = parent_depth();
        request.system_prompt = optional_string(&args, "SystemPrompt");
        request.correlation_id = optional_string(&args, "Correlation_ID");
        request.workspace_mode = optional_string(&args, "WorkspaceMode");
        if let Some(seconds) = args.get("TimeoutSeconds").and_then(Value::as_u64) {
            request.timeout = Duration::from_secs(seconds);
        }
        let handle = self.runtime.launch(request)?;
        Ok(json!({
            "status": "accepted",
            "subagent_id": handle.subagent_id,
            "parent_session_id": handle.parent_session_id,
            "correlation_id": handle.correlation_id,
            "depth": handle.depth,
            "message": "The result will be injected into the parent run automatically. Poll only when intermediate supervision is necessary."
        }))
    }
}

pub struct CheckWorkerStatusTool {
    runtime: Arc<SubagentRuntime>,
    parent_session_id: String,
}

impl CheckWorkerStatusTool {
    pub fn new(runtime: Arc<SubagentRuntime>, parent_session_id: impl Into<String>) -> Self {
        Self {
            runtime,
            parent_session_id: parent_session_id.into(),
        }
    }
}

impl ToolExecutor for CheckWorkerStatusTool {
    fn name(&self) -> &'static str {
        "check_worker_status"
    }

    fn description(&self) -> &'static str {
        "Inspect one owned subagent, or list all subagents owned by the current parent session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Subagent_ID": { "type": "string" },
                "Worker_ID": { "type": "string", "description": "Legacy alias for Subagent_ID." }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let id =
            optional_string(&args, "Subagent_ID").or_else(|| optional_string(&args, "Worker_ID"));
        if let Some(id) = id {
            let snapshot = self
                .runtime
                .status(&id)
                .ok_or_else(|| format!("unknown subagent: {id}"))?;
            if snapshot.handle.parent_session_id != self.parent_session_id {
                return Err("subagent is not owned by this parent session".to_string());
            }
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        } else {
            Ok(json!({ "subagents": self.runtime.list(Some(&self.parent_session_id)) }))
        }
    }
}

pub struct InterveneWorkerTool {
    runtime: Arc<SubagentRuntime>,
}

impl InterveneWorkerTool {
    pub fn new(runtime: Arc<SubagentRuntime>) -> Self {
        Self { runtime }
    }
}

impl ToolExecutor for InterveneWorkerTool {
    fn name(&self) -> &'static str {
        "intervene_worker"
    }

    fn description(&self) -> &'static str {
        "Steer a running subagent at its next safe model boundary; this does not terminate it."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Subagent_ID": { "type": "string" },
                "Worker_ID": { "type": "string", "description": "Legacy alias for Subagent_ID." },
                "Instruction": { "type": "string" }
            },
            "required": ["Instruction"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let id = subagent_id(&args)?;
        let instruction = required_string(&args, "Instruction")?;
        self.runtime.intervene(&id, &instruction)?;
        Ok(json!({ "status": "queued", "subagent_id": id }))
    }
}

pub struct CancelSubagentTool {
    runtime: Arc<SubagentRuntime>,
}

impl CancelSubagentTool {
    pub fn new(runtime: Arc<SubagentRuntime>) -> Self {
        Self { runtime }
    }
}

impl ToolExecutor for CancelSubagentTool {
    fn name(&self) -> &'static str {
        "cancel_subagent"
    }

    fn description(&self) -> &'static str {
        "Request cancellation of a running subagent and settle its process before final delivery."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Subagent_ID": { "type": "string" },
                "Worker_ID": { "type": "string", "description": "Legacy alias for Subagent_ID." }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let id = subagent_id(&args)?;
        let requested = self.runtime.cancel(&id)?;
        Ok(
            json!({ "status": if requested { "cancel_requested" } else { "already_terminal" }, "subagent_id": id }),
        )
    }
}

fn parent_depth() -> u8 {
    std::env::var("HUB_DELEGATION_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn subagent_id(args: &Value) -> Result<String, String> {
    optional_string(args, "Subagent_ID")
        .or_else(|| optional_string(args, "Worker_ID"))
        .ok_or_else(|| "Subagent_ID is required".to_string())
}

fn required_string(args: &Value, name: &str) -> Result<String, String> {
    optional_string(args, name).ok_or_else(|| format!("{name} is required"))
}

fn optional_string(args: &Value, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
