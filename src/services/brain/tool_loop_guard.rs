use crate::services::brain::tool_call_identity::fingerprint;
use serde_json::Value;
use std::collections::VecDeque;

const DEFAULT_MAX_CONSECUTIVE: usize = 3;
const DEFAULT_MAX_IN_WINDOW: usize = 5;
const DEFAULT_WINDOW_SIZE: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepetitionBlock {
    pub fingerprint: String,
    pub consecutive_count: usize,
    pub window_count: usize,
}

/// Session-local circuit breaker for exact repeated tool calls. It stores only
/// fingerprints, keeping arguments and credentials out of diagnostics.
#[derive(Debug)]
pub struct ToolLoopGuard {
    recent: VecDeque<String>,
    max_consecutive: usize,
    max_in_window: usize,
    window_size: usize,
}

impl Default for ToolLoopGuard {
    fn default() -> Self {
        Self {
            recent: VecDeque::new(),
            max_consecutive: DEFAULT_MAX_CONSECUTIVE,
            max_in_window: DEFAULT_MAX_IN_WINDOW,
            window_size: DEFAULT_WINDOW_SIZE,
        }
    }
}

impl ToolLoopGuard {
    pub fn reset(&mut self) {
        self.recent.clear();
    }

    #[cfg(test)]
    pub fn new(max_consecutive: usize, max_in_window: usize, window_size: usize) -> Self {
        Self {
            recent: VecDeque::new(),
            max_consecutive,
            max_in_window,
            window_size,
        }
    }

    pub fn record(&mut self, tool_name: &str, arguments: &Value) -> Result<(), RepetitionBlock> {
        let key = fingerprint(tool_name, arguments);
        let consecutive_count = self
            .recent
            .iter()
            .rev()
            .take_while(|previous| previous.as_str() == key)
            .count()
            + 1;
        let window_count = self
            .recent
            .iter()
            .filter(|previous| **previous == key)
            .count()
            + 1;

        if consecutive_count > self.max_consecutive || window_count > self.max_in_window {
            return Err(RepetitionBlock {
                fingerprint: key,
                consecutive_count,
                window_count,
            });
        }

        self.recent.push_back(key);
        while self.recent.len() > self.window_size {
            self.recent.pop_front();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blocks_the_fourth_exact_consecutive_call() {
        let mut guard = ToolLoopGuard::new(3, 10, 12);
        for _ in 0..3 {
            assert!(guard.record("view_file", &json!({ "path": "a" })).is_ok());
        }
        assert!(guard.record("view_file", &json!({ "path": "a" })).is_err());
    }

    #[test]
    fn changed_arguments_break_the_consecutive_sequence() {
        let mut guard = ToolLoopGuard::new(2, 10, 12);
        assert!(guard.record("view_file", &json!({ "path": "a" })).is_ok());
        assert!(guard.record("view_file", &json!({ "path": "a" })).is_ok());
        assert!(guard.record("view_file", &json!({ "path": "b" })).is_ok());
        assert!(guard.record("view_file", &json!({ "path": "a" })).is_ok());
    }

    #[test]
    fn blocks_recurrence_inside_a_sliding_window() {
        let mut guard = ToolLoopGuard::new(10, 3, 8);
        for index in 0..3 {
            assert!(guard.record("fixture", &json!({ "same": true })).is_ok());
            assert!(guard.record("fixture", &json!({ "other": index })).is_ok());
        }
        assert!(guard.record("fixture", &json!({ "same": true })).is_err());
    }

    #[test]
    fn a_new_run_resets_the_guard() {
        let mut guard = ToolLoopGuard::new(1, 1, 4);
        assert!(guard.record("fixture", &json!({})).is_ok());
        assert!(guard.record("fixture", &json!({})).is_err());
        guard.reset();
        assert!(guard.record("fixture", &json!({})).is_ok());
    }
}
