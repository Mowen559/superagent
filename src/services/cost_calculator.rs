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
        
        let (in_price_per_m, out_price_per_m, cache_read_price_per_m, cache_write_price_per_m) = 
        if name.contains("claude-3-5-sonnet") {
            (3.0, 15.0, 0.30, 3.75) // Anthropic Pricing (Prompt, Completion, Cache Read, Cache Write)
        } else if name.contains("claude-3-haiku") {
            (0.25, 1.25, 0.03, 0.30)
        } else if name.contains("claude-3-opus") {
            (15.0, 75.0, 1.50, 18.75)
        } else if name.contains("gpt-4o-mini") {
            (0.15, 0.60, 0.075, 0.15)
        } else if name.contains("gpt-4o") {
            (5.0, 15.0, 2.50, 5.0)
        } else if name.contains("gemini-1.5-pro") {
            (3.50, 10.50, 0.0, 0.0) // Assume standard if cache prices vary or not tracked
        } else if name.contains("gemini-1.5-flash") || name.contains("gemini-3-flash") {
            (0.075, 0.30, 0.0, 0.0)
        } else if name.contains("deepseek-coder") || name.contains("deepseek-chat") {
            (0.14, 0.28, 0.014, 0.14) // DeepSeek API pricing
        } else {
            (0.0, 0.0, 0.0, 0.0) // Unknown model or self-hosted, assume free
        };

        let in_cost = (input_tokens as f64 / 1_000_000.0) * in_price_per_m;
        let out_cost = (output_tokens as f64 / 1_000_000.0) * out_price_per_m;
        let cache_r_cost = (cache_read_tokens as f64 / 1_000_000.0) * cache_read_price_per_m;
        let cache_w_cost = (cache_write_tokens as f64 / 1_000_000.0) * cache_write_price_per_m;

        in_cost + out_cost + cache_r_cost + cache_w_cost
    }
}
