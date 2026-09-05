use crate::services::brain::events::SessionEvent;
use serde_json::json;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPairingReport {
    pub repaired_missing_results: usize,
    pub removed_orphan_results: usize,
}

impl ToolPairingReport {
    pub fn changed(self) -> bool {
        self.repaired_missing_results > 0 || self.removed_orphan_results > 0
    }
}

pub fn repair_tool_pairs(events: &mut Vec<SessionEvent>) -> ToolPairingReport {
    let mut pending = HashSet::new();
    let mut repaired = Vec::with_capacity(events.len());
    let mut removed_orphan_results = 0usize;

    for event in events.drain(..) {
        match &event {
            SessionEvent::ToolCall { id, .. } => {
                pending.insert(id.clone());
                repaired.push(event);
            }
            SessionEvent::ToolResult { id, .. } if pending.remove(id) => repaired.push(event),
            SessionEvent::ToolResult { .. } => removed_orphan_results += 1,
            _ => repaired.push(event),
        }
    }

    let repaired_missing_results = pending.len();
    for id in pending {
        repaired.push(SessionEvent::ToolResult {
            id,
            result: json!({
                "error": "tool execution was interrupted before a result was recorded",
                "kind": "interrupted",
                "recovered": true
            }),
        });
    }
    if removed_orphan_results > 0 {
        repaired.push(SessionEvent::SystemWarning(format!(
            "Removed {} orphan tool result event(s) during session recovery.",
            removed_orphan_results
        )));
    }
    *events = repaired;

    ToolPairingReport {
        repaired_missing_results,
        removed_orphan_results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_missing_result_and_removes_orphan_result() {
        let mut events = vec![
            SessionEvent::ToolCall {
                id: "missing".into(),
                name: "shell".into(),
                arguments: json!({}),
            },
            SessionEvent::ToolResult {
                id: "orphan".into(),
                result: json!({ "value": true }),
            },
        ];

        let report = repair_tool_pairs(&mut events);
        assert_eq!(report.repaired_missing_results, 1);
        assert_eq!(report.removed_orphan_results, 1);
        assert!(events.iter().any(|event| matches!(
            event,
            SessionEvent::ToolResult { id, result }
                if id == "missing" && result["kind"] == "interrupted"
        )));
    }

    #[test]
    fn leaves_valid_pairs_unchanged() {
        let mut events = vec![
            SessionEvent::ToolCall {
                id: "paired".into(),
                name: "read".into(),
                arguments: json!({}),
            },
            SessionEvent::ToolResult {
                id: "paired".into(),
                result: json!({ "ok": true }),
            },
        ];
        assert!(!repair_tool_pairs(&mut events).changed());
        assert_eq!(events.len(), 2);
    }
}
