use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::task_plan_revision::PlanRevision;
use crate::services::brain::task_progress::SharedTaskProgress;
use serde_json::Value;

pub struct ReviseTaskPlanTool(pub SharedTaskProgress);

impl ToolExecutor for ReviseTaskPlanTool {
    fn name(&self) -> &'static str {
        "revise_task_plan"
    }
    fn description(&self) -> &'static str {
        "Apply an additive delta to the current plan using its plan_revision. Append only new success criteria, constraints or questions; upsert only new/changed steps by id. Unmentioned requirements and unaffected evidence are preserved. Stale/cyclic/over-budget revisions change nothing. Changed steps and checks require fresh evidence. Prefer this over rewriting a plan after additional user instructions."
    }
    fn parameters_schema(&self) -> Value {
        PlanRevision::schema()
    }
    fn execute(&self, args: Value) -> Result<Value, String> {
        let patch = PlanRevision::from_value(args)?;
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revise(patch)
    }
}
