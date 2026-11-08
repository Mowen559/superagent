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

pub struct RetrySubagentTool {runtime:Arc<SubagentRuntime>}
impl RetrySubagentTool {pub fn new(runtime:Arc<SubagentRuntime>)->Self {Self{runtime}}}
impl ToolExecutor for RetrySubagentTool {
    fn name(&self)->&'static str {"retry_subagent"}
    fn description(&self)->&'static str {"Explicitly replace one owned failed/cancelled/timed-out, reaped isolated implementation worker once, preserving its exact original task/role/permissions. Wait for cancellation to settle first. This is not a new unrelated invoke. Old terminal history stays visible. Reviews and replacement chains cannot be retried this way; after replacement collect artifacts and rerun parent validation."}
    fn parameters_schema(&self)->Value {json!({"type":"object","additionalProperties":false,"required":["Subagent_ID"],"properties":{"Subagent_ID":{"type":"string"}}})}
    fn execute(&self,args:Value)->Result<Value,String> {
        if args.as_object().is_some_and(|a|a.keys().any(|k|k!="Subagent_ID")) {return Err("retry accepts only the owned predecessor ID; original task cannot be overridden".into());}
        let id=subagent_id(&args)?;let old=self.runtime.status(&id).ok_or("unknown predecessor")?;
        self.runtime.retry(&old.handle.parent_run_id,&id).map(|h|json!({"status":"accepted","handle":h,"replacesSubagentId":id,"businessVerified":false}))
    }
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
        "Start one of at most two asynchronous leaf workers. Use worktree for isolated code edits, then collect_subagent_changes to inspect/apply its artifacts. Collect results before final delivery; process success is not business verification."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Prompt": { "type": "string", "description": "A concrete, independently executable task." },
                "Role": { "type": "string", "description": "Short specialist role." },
                "SystemPrompt": { "type": "string", "description": "Optional isolated system instructions." },
                "Correlation_ID": { "type": "string", "description": "Optional idempotency key within this parent Run. Same key requires identical input; historical Runs do not block new work." },
                "WorkspaceMode": { "type": "string", "enum": ["inherit", "worktree"], "description": "inherit shares the bound parent workspace. worktree snapshots its CURRENT code, including uncommitted input, in an owned Git base; the parent need not be a Git repository. Requires the Git executable. Failure never falls back to sharing. Artifacts stay isolated until explicit fingerprinted collection." },
                "TimeoutSeconds": { "type": "integer", "minimum": 1, "maximum": 86400, "description":"Optional total worker wall-time for ALL model turns, edits and checks. Omit to use the 30-minute default unless the user explicitly gave a deadline. Do not use status-poll wait durations such as 30/60/120 seconds as the development task budget." }
            },
            "required": ["Prompt", "Role"]
        })
    }

    fn prompt_projection(&self)->Option<String> {
        Some("[Delegation coordination] Use the default worker timeout unless the user supplied a deadline; multi-turn model work can take minutes. WaitSeconds is only a status wait, not a task deadline. While independent workers execute, advance your own integration work instead of only polling. Results are delivered automatically; use bounded waits when a result is actually needed. Cancellation is not task completion. If an isolated implementation worker fails or is cancelled, wait until it is reaped, then use retry_subagent with its ID to create a single linked replacement of the same frozen task. A new unrelated invoke does not discharge the old failure. Collect successful artifacts and rerun parent validation before finishing; failed reviews remain blocking.".into())
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
        "Inspect owned subagents; optionally wait up to 30 seconds for one child to terminate. Collect child results before final delivery; process success alone is not business verification."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Subagent_ID": { "type": "string" },
                "WaitSeconds": { "type": "integer", "minimum": 0, "maximum": 30 },
                "Worker_ID": { "type": "string", "description": "Legacy alias for Subagent_ID." }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let wait = match args.get("WaitSeconds") {
            None => 0,
            Some(value) => value.as_u64().filter(|v| *v <= 30).ok_or("WaitSeconds must be an integer in 0..=30")?,
        };
        let id =
            optional_string(&args, "Subagent_ID").or_else(|| optional_string(&args, "Worker_ID"));
        if let Some(id) = id {
            let mut snapshot = self
                .runtime
                .status(&id)
                .ok_or_else(|| format!("unknown subagent: {id}"))?;
            if snapshot.handle.parent_session_id != self.parent_session_id {
                return Err("subagent is not owned by this parent session".to_string());
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(wait);
            while !snapshot.state.is_terminal() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
                snapshot = self.runtime.status(&id).ok_or("owned subagent no longer available")?;
            }
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        } else {
            if wait > 0 { return Err("Subagent_ID is required when waiting".into()); }
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

pub struct CollectSubagentChangesTool { runtime:Arc<SubagentRuntime>,parent:String }
impl CollectSubagentChangesTool { pub fn new(runtime:Arc<SubagentRuntime>,parent:impl Into<String>)->Self{Self{runtime,parent:parent.into()}} }
impl ToolExecutor for CollectSubagentChangesTool{
    fn name(&self)->&'static str{"collect_subagent_changes"}
    fn description(&self)->&'static str{"Inspect settled owned worktree changes, then explicitly apply their ExpectedDigest into this parent workspace. Conflicts and readonly/receipt guards are enforced; collection is not business verification."}
    fn parameters_schema(&self)->Value{json!({"type":"object","properties":{"Subagent_ID":{"type":"string"},"Action":{"type":"string","enum":["inspect","apply"]},"ExpectedDigest":{"type":"string"}},"required":["Subagent_ID","Action"]})}
    fn execute(&self,args:Value)->Result<Value,String>{
        let snapshot=self.runtime.status(&subagent_id(&args)?).ok_or("unknown owned subagent")?;
        if snapshot.handle.parent_session_id!=self.parent{return Err("subagent is not owned by this session".into());}
        self.runtime.collect_changes(&snapshot.handle.parent_run_id,&snapshot.handle.subagent_id,&required_string(&args,"Action")?,optional_string(&args,"ExpectedDigest").as_deref())
    }
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
