use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::interaction_runtime::{
    InteractionChoice, InteractionRuntime, DEFAULT_TIMEOUT_SECONDS, MAX_CHOICES,
    MAX_TIMEOUT_SECONDS,
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

/// 兼容两种选项形态：纯字符串，或 ZCode AskUserQuestion 式结构化对象
/// {label, description?, recommended?}（推荐方案会被前端高亮标记）。
fn parse_choices(raw: Option<&Vec<Value>>) -> (Vec<String>, Vec<InteractionChoice>) {
    let Some(values) = raw else {
        return (Vec::new(), Vec::new());
    };
    let mut labels = Vec::new();
    let mut details = Vec::new();
    for value in values {
        match value {
            Value::String(s) if !s.trim().is_empty() => labels.push(s.trim().to_string()),
            Value::Object(obj) => {
                let Some(label) = obj.get("label").and_then(Value::as_str).map(str::trim) else {
                    continue;
                };
                if label.is_empty() {
                    continue;
                }
                let description = obj
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let recommended = obj
                    .get("recommended")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                details.push(InteractionChoice {
                    label: label.to_string(),
                    description,
                    recommended,
                });
                labels.push(label.to_string());
            }
            _ => {}
        }
    }
    (labels, details)
}

impl ToolExecutor for AskUserTool {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn description(&self) -> &'static str {
        "Request correlated user input when a material choice cannot be resolved from available evidence. Choices may be plain strings or structured objects {label, description, recommended} following the dual-track inquiry protocol (state verified facts, list option trade-offs, mark your recommendation)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Question": { "type": "string" },
                "Choices": {
                    "type": "array",
                    "maxItems": MAX_CHOICES,
                    "items": {
                        "oneOf": [
                            { "type": "string" },
                            {
                                "type": "object",
                                "properties": {
                                    "label": { "type": "string" },
                                    "description": { "type": "string" },
                                    "recommended": { "type": "boolean" }
                                },
                                "required": ["label"]
                            }
                        ]
                    }
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
        let (labels, details) = parse_choices(args.get("Choices").and_then(Value::as_array));
        let timeout = args
            .get("TimeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        let receipt =
            self.runtime
                .ask_with_details(question, labels, details, Duration::from_secs(timeout))?;
        serde_json::to_value(receipt).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_structured_choices() {
        let raw = vec![
            json!("Plain option"),
            json!({ "label": "Option A", "description": "lower risk", "recommended": true }),
            json!({ "label": "" }),
            json!({ "description": "missing label" }),
        ];
        let (labels, details) = parse_choices(Some(&raw));
        assert_eq!(labels, vec!["Plain option", "Option A"]);
        assert_eq!(details.len(), 1);
        assert!(details[0].recommended);
        assert_eq!(details[0].description.as_deref(), Some("lower risk"));
    }
}
