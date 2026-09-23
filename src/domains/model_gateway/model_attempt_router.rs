use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelErrorClass {
    ContextOverflow,
    Authentication,
    RateLimited,
    Transient,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrainTier {
    Fast,
    Slow,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DualBrainRouter;

impl DualBrainRouter {
    pub fn select_tier(
        reasoning_mode: crate::services::brain::cognitive_runtime::ReasoningMode,
        task_kind: crate::services::brain::cognitive_runtime::TaskKind,
        tool_calls: usize,
        failed_tools: usize,
        consecutive_failures: usize,
    ) -> BrainTier {
        use crate::services::brain::cognitive_runtime::{ReasoningMode, TaskKind};
        if reasoning_mode == ReasoningMode::Fast {
            return BrainTier::Fast;
        }
        if reasoning_mode == ReasoningMode::Deliberate {
            return BrainTier::Slow;
        }
        if task_kind == TaskKind::Conversational {
            return BrainTier::Fast;
        }
        // Adaptive mode:
        if failed_tools > 0 || consecutive_failures > 0 {
            return BrainTier::Slow;
        }
        if task_kind == TaskKind::Execution && tool_calls > 0 {
            return BrainTier::Slow;
        }
        BrainTier::Fast
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ModelAttemptRouter {
    max_attempts: u8,
}

impl Default for ModelAttemptRouter {
    fn default() -> Self {
        Self { max_attempts: 3 }
    }
}

impl ModelAttemptRouter {
    pub fn classify(&self, error: &str) -> ModelErrorClass {
        let error = error.to_ascii_lowercase();
        if contains_any(
            &error,
            &[
                "context budget",
                "context length",
                "too many tokens",
                "maximum context",
            ],
        ) {
            ModelErrorClass::ContextOverflow
        } else if contains_any(
            &error,
            &["401", "unauthorized", "invalid api key", "authentication"],
        ) {
            ModelErrorClass::Authentication
        } else if contains_any(
            &error,
            &["429", "rate limit", "quota exceeded", "resource_exhausted"],
        ) {
            ModelErrorClass::RateLimited
        } else if contains_any(
            &error,
            &[
                "408",
                "409",
                "425",
                "500",
                "502",
                "503",
                "504",
                "timeout",
                "timed out",
                "connection",
                "temporarily unavailable",
                "request failed",
            ],
        ) {
            ModelErrorClass::Transient
        } else {
            ModelErrorClass::Permanent
        }
    }

    /// `failed_attempt` starts at one. None returns control to the owning
    /// recovery Module instead of retrying inside the provider call site.
    pub fn retry_delay(&self, failed_attempt: u8, error: &str) -> Option<Duration> {
        if failed_attempt >= self.max_attempts {
            return None;
        }
        match self.classify(error) {
            ModelErrorClass::RateLimited => {
                Some(Duration::from_millis(1_000 * u64::from(failed_attempt)))
            }
            ModelErrorClass::Transient => Some(Duration::from_millis(
                250 * 4_u64.pow(u32::from(failed_attempt - 1)),
            )),
            ModelErrorClass::ContextOverflow
            | ModelErrorClass::Authentication
            | ModelErrorClass::Permanent => None,
        }
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_recovery_owners_without_retrying_them() {
        let router = ModelAttemptRouter::default();
        assert_eq!(
            router.classify("context budget exceeded before request"),
            ModelErrorClass::ContextOverflow
        );
        assert_eq!(
            router.classify("401 Unauthorized"),
            ModelErrorClass::Authentication
        );
        assert!(router.retry_delay(1, "context length exceeded").is_none());
        assert!(router.retry_delay(1, "invalid request schema").is_none());
    }

    #[test]
    fn transient_and_rate_limit_retries_are_finite() {
        let router = ModelAttemptRouter::default();
        assert_eq!(
            router.retry_delay(1, "503 temporarily unavailable"),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            router.retry_delay(2, "429 rate limit"),
            Some(Duration::from_millis(2_000))
        );
        assert!(router.retry_delay(3, "connection reset").is_none());
    }

    #[test]
    fn dual_brain_routing_rules_test() {
        use crate::services::brain::cognitive_runtime::{ReasoningMode, TaskKind};

        // Fast mode always picks Fast Brain
        assert_eq!(
            DualBrainRouter::select_tier(ReasoningMode::Fast, TaskKind::Execution, 5, 1, 1),
            BrainTier::Fast
        );

        // Conversational task always picks Fast Brain
        assert_eq!(
            DualBrainRouter::select_tier(ReasoningMode::Adaptive, TaskKind::Conversational, 0, 0, 0),
            BrainTier::Fast
        );

        // Deliberate mode always picks Slow Brain
        assert_eq!(
            DualBrainRouter::select_tier(ReasoningMode::Deliberate, TaskKind::Conversational, 0, 0, 0),
            BrainTier::Slow
        );

        // Adaptive exploration (tool_calls == 0, 0 failures) -> Fast Brain
        assert_eq!(
            DualBrainRouter::select_tier(ReasoningMode::Adaptive, TaskKind::Execution, 0, 0, 0),
            BrainTier::Fast
        );

        // Adaptive execution (tool_calls > 0) -> Slow Brain
        assert_eq!(
            DualBrainRouter::select_tier(ReasoningMode::Adaptive, TaskKind::Execution, 1, 0, 0),
            BrainTier::Slow
        );

        // Failure occurs -> Slow Brain for deep reflection
        assert_eq!(
            DualBrainRouter::select_tier(ReasoningMode::Adaptive, TaskKind::Investigative, 0, 1, 1),
            BrainTier::Slow
        );
    }
}
