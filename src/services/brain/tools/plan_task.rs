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
        "For complex requests, record your understanding after inspecting available context: intended outcome, observable success criteria, constraints, concise ordered subgoals with acceptance and dependencies, and remaining context gaps. For a simple single artifact, act and verify directly without a plan. Do not add ceremony steps for writing a Todo or sending the final answer. Complete planned subgoals using update_task_progress with actual receipt_ids. This plan does not prove execution or completion."
    }

    fn parameters_schema(&self) -> Value {
        TaskContract::schema()
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let contract = TaskContract::from_value(args)?;
        Ok(json!({"status": "planned", "task_contract": contract}))
    }
}
