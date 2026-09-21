use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::interaction_runtime::{
    InteractionRuntime, DEFAULT_TIMEOUT_SECONDS, MAX_CHOICES, MAX_TIMEOUT_SECONDS,
};

/// Model-facing Adapter for the correlated user-interaction runtime.
pub struct AskUserTool {
    runtime: Arc<InteractionRuntime>,
}

impl AskUserTool {
    pub fn new(runtime: Arc<InteractionRuntime>) -> Self {
        Self { runtime }
    }
}

impl ToolExecutor for AskUserTool {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn description(&self) -> &'static str {
        "Request correlated user input when a material choice cannot be resolved from available evidence. Supports optional choices and a finite timeout."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Question": { "type": "string" },
                "Choices": {
                    "type": "array",
                    "maxItems": MAX_CHOICES,
                    "items": { "type": "string" }
                },
                "TimeoutSeconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECONDS,
                    "default": DEFAULT_TIMEOUT_SECONDS
                }
            },
            "required": ["Question"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let question = args
            .get("Question")
            .and_then(Value::as_str)
            .ok_or_else(|| "Question is required".to_string())?;
        let choices = args
            .get("Choices")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let timeout = args
            .get("TimeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        let receipt = self
            .runtime
            .ask(question, choices, Duration::from_secs(timeout))?;
        serde_json::to_value(receipt).map_err(|error| error.to_string())
    }
}
