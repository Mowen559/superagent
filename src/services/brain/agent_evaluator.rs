use serde::{Deserialize, Serialize};
use crate::services::brain::agent_tracing::{AgentSpan, SpanKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCriteria {
    pub max_allowed_tool_calls: Option<usize>,
    pub required_tool_sequence: Vec<String>,
    pub forbidden_tools: Vec<String>,
    pub max_execution_time_ms: Option<i64>,
}

impl Default for EvalCriteria {
    fn default() -> Self {
        Self {
            max_allowed_tool_calls: Some(50),
            required_tool_sequence: Vec::new(),
            forbidden_tools: Vec::new(),
            max_execution_time_ms: Some(300_000), // 5 min
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalOutcome {
    Passed,
    Failed { reason: String },
}

pub struct AgentEvaluator;

impl AgentEvaluator {
    pub fn evaluate(spans: &[AgentSpan], criteria: &EvalCriteria) -> EvalOutcome {
        let tool_spans: Vec<&AgentSpan> = spans.iter().filter(|s| s.kind == SpanKind::ToolCall).collect();

        // 1. Max allowed tool calls assertion
        if let Some(max_calls) = criteria.max_allowed_tool_calls {
            if tool_spans.len() > max_calls {
                return EvalOutcome::Failed {
                    reason: format!("Exceeded max allowed tool calls: {} > {}", tool_spans.len(), max_calls),
                };
            }
        }

        // 2. Forbidden tools assertion
        for span in &tool_spans {
            if let Some(tool_name) = span.attributes.get("tool") {
                if criteria.forbidden_tools.contains(tool_name) {
                    return EvalOutcome::Failed {
                        reason: format!("Forbidden tool '{}' was invoked", tool_name),
                    };
                }
            }
        }

        // 3. Required tool sequence assertion
        if !criteria.required_tool_sequence.is_empty() {
            let invoked_tools: Vec<String> = tool_spans
                .iter()
                .filter_map(|s| s.attributes.get("tool").cloned())
                .collect();

            let mut seq_idx = 0;
            for tool in &invoked_tools {
                if tool == &criteria.required_tool_sequence[seq_idx] {
                    seq_idx += 1;
                    if seq_idx >= criteria.required_tool_sequence.len() {
                        break;
                    }
                }
            }

            if seq_idx < criteria.required_tool_sequence.len() {
                return EvalOutcome::Failed {
                    reason: format!(
                        "Required tool sequence not satisfied. Expected sequence: {:?}",
                        criteria.required_tool_sequence
                    ),
                };
            }
        }

        // 4. Execution duration assertion
        if let Some(max_time) = criteria.max_execution_time_ms {
            let start = spans.iter().map(|s| s.start_time_ms).min().unwrap_or(0);
            let end = spans.iter().filter_map(|s| s.end_time_ms).max().unwrap_or(start);
            let duration = end - start;

            if duration > max_time {
                return EvalOutcome::Failed {
                    reason: format!("Exceeded max execution time: {}ms > {}ms", duration, max_time),
                };
            }
        }

        EvalOutcome::Passed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::brain::agent_tracing::AgentTracingRecorder;

    #[test]
    fn test_evaluator_assertions() {
        let tracer = AgentTracingRecorder::new();
        let s1 = tracer.start_span("Call 1", SpanKind::ToolCall, None);
        tracer.set_attribute(&s1, "tool", "read_file");
        tracer.end_span(&s1, "OK");

        let s2 = tracer.start_span("Call 2", SpanKind::ToolCall, None);
        tracer.set_attribute(&s2, "tool", "run_command");
        tracer.end_span(&s2, "OK");

        let spans = tracer.export_spans();

        let criteria = EvalCriteria {
            max_allowed_tool_calls: Some(10),
            required_tool_sequence: vec!["read_file".to_string(), "run_command".to_string()],
            forbidden_tools: vec!["rm_rf".to_string()],
            max_execution_time_ms: Some(10000),
        };

        let outcome = AgentEvaluator::evaluate(&spans, &criteria);
        assert_eq!(outcome, EvalOutcome::Passed);
    }
}
