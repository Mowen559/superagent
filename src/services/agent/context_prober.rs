use reqwest::Client;
use serde_json::json;
use regex::Regex;

/// Probes an LLM API to dynamically discover its maximum context length
/// by intentionally triggering a TokenLimitExceeded error (Zero-Cost).
pub async fn probe_max_context_length(
    base_url: &str,
    api_key: &str,
    model_name: &str,
) -> Result<u32, String> {
    let client = Client::new();
    
    // 1. The Probe: Send a 1-token prompt but request an impossible max_tokens limit.
    let payload = json!({
        "model": model_name,
        "messages": [{"role": "user", "content": "Hello"}],
        "max_tokens": 9999999
    });

    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };

    let res = client.post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Network error during probe: {}", e))?;

    let status = res.status();
    let body = res.text().await.unwrap_or_default();

    // 2. The Extraction: We expect HTTP 400 Bad Request
    if status.is_client_error() {
        // Look for common OpenAI-style error patterns
        // e.g. "This model's maximum context length is 128000 tokens."
        // e.g. "maximum context length is 200000 tokens."
        let re = Regex::new(r"maximum context length is (\d+) tokens").unwrap();
        
        if let Some(caps) = re.captures(&body) {
            if let Some(matched) = caps.get(1) {
                if let Ok(limit) = matched.as_str().parse::<u32>() {
                    return Ok(limit);
                }
            }
        }
        
        // DeepSeek/Anthropic fallback variations could go here
        let fallback_re = Regex::new(r"exceeds model context limit of (\d+)").unwrap();
        if let Some(caps) = fallback_re.captures(&body) {
            if let Some(matched) = caps.get(1) {
                if let Ok(limit) = matched.as_str().parse::<u32>() {
                    return Ok(limit);
                }
            }
        }

        Err(format!("Failed to extract limit from error message: {}", body))
    } else {
        // If it somehow succeeded, the API didn't block it!
        Err(format!("Probe failed: Expected HTTP 400, but got HTTP {}. Body: {}", status, body))
    }
}

