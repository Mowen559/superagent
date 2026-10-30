use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::task_understanding::TaskContract;
use serde_json::{Value, json};

/// Pure planning capability: no I/O, model calls, or authority changes.
pub struct PlanTaskTool;

impl ToolExecutor for PlanTaskTool {
    fn name(&self) -> &'static str {
        "plan_task"
    }

    fn description(&self) -> &'static str {
        "For complex requests, record or replace your understanding after inspecting available context: intended outcome, observable success criteria, constraints, up to 12 concise ordered subgoals with acceptance conditions and dependencies, and remaining context gaps. Workspace/external gaps should be investigated with tools; ask the user only about unresolved material choices. This plan does not prove execution or completion."
    }

    fn parameters_schema(&self) -> Value {
        TaskContract::schema()
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let contract = TaskContract::from_value(args)?;
        Ok(json!({"status": "planned", "task_contract": contract}))
    }
}
