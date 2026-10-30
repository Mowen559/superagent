use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::task_progress::{ProgressUpdate, SharedTaskProgress};
use serde_json::{Value, json};

pub struct TaskProgressTool(pub SharedTaskProgress);

impl ToolExecutor for TaskProgressTool {
    fn name(&self) -> &'static str {
        "update_task_progress"
    }
    fn description(&self) -> &'static str {
        "Inspect task progress or cite successful tool receipt IDs from this Run to complete a subgoal or resolve a context question. Dependencies, evidence kinds, context sources and verification freshness are checked by the runtime. A model-written plan or Todo is not evidence. Question indices start at zero."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false, "properties": {
            "action": {"type": "string", "enum": ["status", "complete_step", "resolve_question"]},
            "step_id": {"type": "string", "maxLength": 32},
            "question_index": {"type": "integer", "minimum": 0, "maximum": crate::services::brain::task_understanding::MAX_QUESTIONS - 1},
            "receipt_ids": {"type": "array", "minItems": 1, "maxItems": 8, "items": {"type": "string", "maxLength": 128}}
        }, "required": ["action"]})
    }
    fn execute(&self, args: Value) -> Result<Value, String> {
        let update = serde_json::from_value::<ProgressUpdate>(args).map_err(|e| e.to_string())?;
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .update(update)
    }
}
