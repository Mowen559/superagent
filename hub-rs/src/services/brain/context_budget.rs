use serde_json::Value;

const MIN_COMPLETION_TOKENS: usize = 256;
const MIN_SAFETY_MARGIN_TOKENS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub context_window_tokens: usize,
    pub requested_completion_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestBudget {
    pub estimated_prompt_tokens: usize,
    pub safety_margin_tokens: usize,
    pub completion_tokens: usize,
    pub overflow_tokens: usize,
}

impl RequestBudget {
    pub fn requires_compaction(self) -> bool {
        self.overflow_tokens > 0
    }

    pub fn utilization_percent(self, context_window_tokens: usize) -> u8 {
        if context_window_tokens == 0 {
            return 100;
        }
        let used = self
            .estimated_prompt_tokens
            .saturating_add(self.safety_margin_tokens)
            .saturating_add(self.completion_tokens);
        ((used.saturating_mul(100) / context_window_tokens).min(100)) as u8
    }
}

impl ContextBudget {
    pub fn plan(self, messages: &[Value], tools: Option<&Value>) -> RequestBudget {
        let message_tokens = estimate_json_tokens(&Value::Array(messages.to_vec()));
        let tool_tokens = tools.map(estimate_json_tokens).unwrap_or(0);
        self.plan_tokens(message_tokens.saturating_add(tool_tokens))
    }

    pub fn plan_tokens(self, estimated_prompt_tokens: usize) -> RequestBudget {
        let safety_margin_tokens = (self.context_window_tokens / 20)
            .max(MIN_SAFETY_MARGIN_TOKENS)
            .min(self.context_window_tokens / 4);
        let usable_tokens = self
            .context_window_tokens
            .saturating_sub(safety_margin_tokens);
        let available_completion = usable_tokens.saturating_sub(estimated_prompt_tokens);
        let completion_tokens = self
            .requested_completion_tokens
            .min(available_completion)
            .max(MIN_COMPLETION_TOKENS);
        let required_tokens = estimated_prompt_tokens
            .saturating_add(safety_margin_tokens)
            .saturating_add(MIN_COMPLETION_TOKENS);

        RequestBudget {
            estimated_prompt_tokens,
            safety_margin_tokens,
            completion_tokens,
            overflow_tokens: required_tokens.saturating_sub(self.context_window_tokens),
        }
    }
}

pub fn estimate_json_tokens(value: &Value) -> usize {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    estimate_text_tokens(&serialized)
}

pub fn estimate_text_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }

    // English prose is usually near four characters per token. CJK text is
    // closer to one character per token. The fixed overhead covers message
    // framing that is not visible in serialized content.
    ascii
        .div_ceil(4)
        .saturating_add(non_ascii)
        .saturating_add(8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimator_is_language_aware() {
        assert_eq!(estimate_text_tokens("a".repeat(400).as_str()), 108);
        assert_eq!(estimate_text_tokens("上下文预算"), 13);
    }

    #[test]
    fn plan_reserves_output_and_safety_margin() {
        let plan = ContextBudget {
            context_window_tokens: 32_000,
            requested_completion_tokens: 16_000,
        }
        .plan_tokens(20_000);

        assert_eq!(plan.safety_margin_tokens, 1_600);
        assert_eq!(plan.completion_tokens, 10_400);
        assert!(!plan.requires_compaction());
        assert_eq!(plan.utilization_percent(32_000), 100);
    }

    #[test]
    fn plan_marks_prompt_that_cannot_fit_minimum_completion() {
        let plan = ContextBudget {
            context_window_tokens: 8_000,
            requested_completion_tokens: 4_000,
        }
        .plan(
            &[json!({ "role": "user", "content": "x".repeat(32_000) })],
            None,
        );

        assert!(plan.requires_compaction());
        assert!(plan.overflow_tokens > 0);
    }
}
