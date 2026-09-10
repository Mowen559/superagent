use std::collections::VecDeque;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StuckReason {
    ConsecutiveIdenticalFailure(String),
    NoProgressLoop(String),
    HighErrorFrequency(usize),
}

impl std::fmt::Display for StuckReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsecutiveIdenticalFailure(tool) => write!(f, "Consecutive identical tool failure for '{}'", tool),
            Self::NoProgressLoop(tool) => write!(f, "No-progress execution loop for '{}'", tool),
            Self::HighErrorFrequency(count) => write!(f, "High error frequency ({}/5 recent calls failed)", count),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StuckState {
    Normal,
    StuckDetected {
        reason: StuckReason,
        suggested_action: String,
    },
}

#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub args_hash: String,
    pub success: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub struct StuckDetector {
    max_history: usize,
    call_history: VecDeque<ToolCallRecord>,
    identical_threshold: usize,
}

impl Default for StuckDetector {
    fn default() -> Self {
        Self::new(10, 3)
    }
}

impl StuckDetector {
    pub fn new(max_history: usize, identical_threshold: usize) -> Self {
        Self {
            max_history,
            call_history: VecDeque::new(),
            identical_threshold,
        }
    }

    pub fn record_call(&mut self, tool_name: &str, args_json: &str, success: bool) -> StuckState {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(args_json.as_bytes());
        let hash_result = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();

        let record = ToolCallRecord {
            tool_name: tool_name.to_string(),
            args_hash: hash_result,
            success,
            timestamp: chrono::Utc::now().timestamp(),
        };

        self.call_history.push_back(record);
        if self.call_history.len() > self.max_history {
            self.call_history.pop_front();
        }

        self.evaluate()
    }

    pub fn evaluate(&self) -> StuckState {
        let len = self.call_history.len();
        if len < self.identical_threshold {
            return StuckState::Normal;
        }

        // 1. Check for consecutive identical failures
        let recent = self.call_history.iter().rev().take(self.identical_threshold).collect::<Vec<_>>();
        if recent.len() >= self.identical_threshold {
            let first = &recent[0];
            let all_identical_failures = recent.iter().all(|r| {
                !r.success && r.tool_name == first.tool_name && r.args_hash == first.args_hash
            });

            if all_identical_failures {
                return StuckState::StuckDetected {
                    reason: StuckReason::ConsecutiveIdenticalFailure(first.tool_name.clone()),
                    suggested_action: format!(
                        "Tool '{}' failed {} times with identical parameters. Halt current approach, re-analyze parameters or switch to alternative tools.",
                        first.tool_name, self.identical_threshold
                    ),
                };
            }
        }

        // 2. Check for no-progress loop (4 identical calls regardless of success status)
        if len >= 4 {
            let recent_4 = self.call_history.iter().rev().take(4).collect::<Vec<_>>();
            let first = &recent_4[0];
            if recent_4.iter().all(|r| r.tool_name == first.tool_name && r.args_hash == first.args_hash) {
                return StuckState::StuckDetected {
                    reason: StuckReason::NoProgressLoop(first.tool_name.clone()),
                    suggested_action: format!(
                        "Repeated call to '{}' 4 times yielded no new progress. Break out of current tool loop.",
                        first.tool_name
                    ),
                };
            }
        }

        // 3. High error frequency in last 5 calls
        if len >= 5 {
            let recent_5 = self.call_history.iter().rev().take(5).collect::<Vec<_>>();
            let error_count = recent_5.iter().filter(|r| !r.success).count();
            if error_count >= 4 {
                return StuckState::StuckDetected {
                    reason: StuckReason::HighErrorFrequency(error_count),
                    suggested_action: format!(
                        "High error frequency detected ({}/5 calls failed). Initiate self-correction reflection sequence.",
                        error_count
                    ),
                };
            }
        }

        StuckState::Normal
    }

    pub fn reset(&mut self) {
        self.call_history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_behavior() {
        let mut detector = StuckDetector::default();
        assert_eq!(detector.record_call("read_file", "{\"path\": \"a.txt\"}", true), StuckState::Normal);
        assert_eq!(detector.record_call("grep_search", "{\"pattern\": \"foo\"}", true), StuckState::Normal);
    }

    #[test]
    fn test_consecutive_failures() {
        let mut detector = StuckDetector::default();
        detector.record_call("run_command", "{\"cmd\": \"cargo test\"}", false);
        detector.record_call("run_command", "{\"cmd\": \"cargo test\"}", false);
        let res = detector.record_call("run_command", "{\"cmd\": \"cargo test\"}", false);
        match res {
            StuckState::StuckDetected { reason, .. } => {
                assert!(matches!(reason, StuckReason::ConsecutiveIdenticalFailure(_)));
            }
            _ => panic!("Expected StuckDetected"),
        }
    }

    #[test]
    fn test_reset() {
        let mut detector = StuckDetector::default();
        detector.record_call("run_command", "{\"cmd\": \"cargo test\"}", false);
        detector.record_call("run_command", "{\"cmd\": \"cargo test\"}", false);
        detector.reset();
        assert_eq!(detector.record_call("run_command", "{\"cmd\": \"cargo test\"}", false), StuckState::Normal);
    }
}
