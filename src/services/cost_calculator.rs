pub struct CostCalculator;

impl CostCalculator {
    /// Calculate the cost in USD based on model name and token usage.
    pub fn calculate_cost(
        model_name: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) -> f64 {
        let name = model_name.to_lowercase();
        // Strip provider prefix if exists (e.g., 'openai/gpt-4o' -> 'gpt-4o')
        let name = name.split('/').last().unwrap_or(&name);
        
        let (in_price_per_m, out_price_per_m, cache_read_price_per_m, cache_write_price_per_m) = 
        if name.starts_with("claude-3-5-sonnet") {
            (3.0, 15.0, 0.30, 3.75)
        } else if name.starts_with("claude-3-haiku") {
            (0.25, 1.25, 0.03, 0.30)
        } else if name.starts_with("claude-3-opus") {
            (15.0, 75.0, 1.50, 18.75)
        } else if name.starts_with("gpt-4o-mini") {
            (0.15, 0.60, 0.075, 0.15)
        } else if name.starts_with("gpt-4o") {
            (5.0, 15.0, 2.50, 5.0)
        } else if name.starts_with("gemini-1.5-pro") {
            (3.50, 10.50, 0.0, 0.0)
        } else if name.starts_with("gemini-1.5-flash") || name.starts_with("gemini-3-flash") {
            (0.075, 0.30, 0.0, 0.0)
        } else if name.starts_with("deepseek-coder") || name.starts_with("deepseek-chat") {
            (0.14, 0.28, 0.014, 0.14)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };

        // Use f64 for safer precision handling
        let in_cost = (input_tokens as f64 / 1_000_000.0) * in_price_per_m;
        let out_cost = (output_tokens as f64 / 1_000_000.0) * out_price_per_m;
        let cache_r_cost = (cache_read_tokens as f64 / 1_000_000.0) * cache_read_price_per_m;
        let cache_w_cost = (cache_write_tokens as f64 / 1_000_000.0) * cache_write_price_per_m;

        in_cost + out_cost + cache_r_cost + cache_w_cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_calculator_edge_cases() {
        // 1. Malformed uppercase model name
        let cost = CostCalculator::calculate_cost("GPT-4O-MINI", 1_000_000, 0, 0, 0);
        assert_eq!(cost, 0.15);

        // 2. Weird prefix model name (should be free if it's a fake/local model)
        let cost_fake = CostCalculator::calculate_cost("fake-gpt-4o-local", 1_000_000, 0, 0, 0);
        assert_eq!(cost_fake, 0.0, "Fake models should not be charged as gpt-4o");

        // 3. OpenRouter format
        let cost_or = CostCalculator::calculate_cost("openai/gpt-4o-mini", 1_000_000, 0, 0, 0);
        assert_eq!(cost_or, 0.15);

        // 4. Precision loss with huge numbers
        let cost_huge = CostCalculator::calculate_cost("claude-3-opus", u32::MAX, u32::MAX, u32::MAX, u32::MAX);
        assert!(cost_huge > 0.0);
    }
}
