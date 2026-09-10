use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanKind {
    Thought,
    Action,
    ToolCall,
    Observation,
    Reflection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpan {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: SpanKind,
    pub start_time_ms: i64,
    pub end_time_ms: Option<i64>,
    pub attributes: HashMap<String, String>,
    pub status: String,
}

#[derive(Clone, Default)]
pub struct AgentTracingRecorder {
    spans: Arc<RwLock<Vec<AgentSpan>>>,
}

impl AgentTracingRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_span(&self, name: impl Into<String>, kind: SpanKind, parent_span_id: Option<String>) -> String {
        let span_id = format!("span-{}", Uuid::new_v4());
        let span = AgentSpan {
            span_id: span_id.clone(),
            parent_span_id,
            name: name.into(),
            kind,
            start_time_ms: chrono::Utc::now().timestamp_millis(),
            end_time_ms: None,
            attributes: HashMap::new(),
            status: "IN_PROGRESS".to_string(),
        };

        let mut lock = self.spans.write().unwrap();
        lock.push(span);
        span_id
    }

    pub fn set_attribute(&self, span_id: &str, key: impl Into<String>, value: impl Into<String>) {
        let mut lock = self.spans.write().unwrap();
        if let Some(span) = lock.iter_mut().find(|s| s.span_id == span_id) {
            span.attributes.insert(key.into(), value.into());
        }
    }

    pub fn end_span(&self, span_id: &str, status: impl Into<String>) {
        let mut lock = self.spans.write().unwrap();
        if let Some(span) = lock.iter_mut().find(|s| s.span_id == span_id) {
            span.end_time_ms = Some(chrono::Utc::now().timestamp_millis());
            span.status = status.into();
        }
    }

    pub fn export_spans(&self) -> Vec<AgentSpan> {
        let lock = self.spans.read().unwrap();
        lock.clone()
    }

    pub fn to_otel_json(&self) -> String {
        let spans = self.export_spans();
        serde_json::to_string_pretty(&spans).unwrap_or_else(|_| "[]".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_span_lifecycle() {
        let tracer = AgentTracingRecorder::new();
        let span_id = tracer.start_span("ToolCall: run_command", SpanKind::ToolCall, None);
        tracer.set_attribute(&span_id, "tool", "run_command");
        tracer.end_span(&span_id, "OK");

        let spans = tracer.export_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].status, "OK");
        assert_eq!(spans[0].attributes.get("tool").unwrap(), "run_command");
    }
}
