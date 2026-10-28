use crate::config::llm_settings::LLMAccount;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::domains::model_gateway::model_attempt_router::ModelErrorClass;
use crate::domains::model_gateway::routing::{ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome, ProviderAttemptReceipt, ProviderCandidate};
use crate::domains::model_gateway::health::{ProviderAdmission, ProviderHealthRuntime};
use crate::domains::model_gateway::protocols::upstream::{adapter_for_provider, UpstreamAdapter};



/// Map a BCP-47 locale (e.g. `zh-CN` / `en-US`) to an output-language
/// instruction for prompts. Defaults to Simplified Chinese.
pub fn lang_directive(lang: &str) -> (&'static str, &'static str) {
    if lang.to_lowercase().starts_with("en") {
        ("Respond in English.", "en")
    } else {
        ("请用简体中文回答。", "zh")
    }
}

/// Strip Markdown code fences (e.g. ```json ... ```) that LLMs often wrap
/// around structured payloads, returning the inner content ready for parsing.
pub fn strip_code_fences(response: &str) -> String {
    let trimmed = response.trim();
    if !trimmed.contains("```") {
        return trimmed.to_string();
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        // Drop the info string (e.g. "json") up to the first newline.
        let body = after.split_once('\n').map(|(_, b)| b).unwrap_or(after);
        if let Some(end) = body.find("```") {
            return body[..end].trim().to_string();
        }
        return body.trim().to_string();
    }
    trimmed.to_string()
}

#[derive(Debug, Default, Clone)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    pub cache_write_tokens: u32,
    pub cost: f64,
    pub duration_ms: u64,
    pub account_id: Option<String>,
    pub provider: Option<String>,
    pub attempts: u8,
    pub attempt_receipts: Vec<ProviderAttemptReceipt>,
}

pub(crate) fn max_context_chars_for(account: Option<&LLMAccount>, model: &str) -> usize {
    let normalized = model.to_ascii_lowercase();
    if let Some(info_map) = account.and_then(|account| account.models_info.as_ref()) {
        if let Some(info) = info_map.get(model).or_else(|| info_map.get(&normalized)) {
            let configured_limit = info
                .get("inputTokenLimit")
                .or_else(|| info.get("context_length"))
                .or_else(|| info.get("context_window"))
                .or_else(|| info.get("max_position_embeddings"))
                .and_then(Value::as_u64);
            if let Some(limit) = configured_limit {
                return limit.saturating_mul(4).min(usize::MAX as u64) as usize;
            }
        }
    }

    if normalized.contains("gemini-1.5-pro") {
        8_000_000
    } else if normalized.contains("gemini") {
        4_000_000
    } else if normalized.contains("claude") {
        800_000
    } else if normalized.contains("gpt-4o")
        || normalized.contains("gpt-4-turbo")
        || normalized.contains("gpt-5")
    {
        500_000
    } else if normalized.contains("deepseek")
        || normalized.contains("qwen")
        || normalized.contains("kimi")
        || normalized.contains("moonshot")
        || normalized.contains("glm")
        || normalized.contains("yi-")
    {
        512_000
    } else {
        256_000
    }
}

pub(crate) fn failure_receipt(
    candidate: &ProviderCandidate,
    attempt: usize,
    started: Instant,
    class: ModelErrorClass,
    status_code: Option<u16>,
) -> ProviderAttemptReceipt {
    ProviderAttemptReceipt::new(
        candidate,
        attempt,
        elapsed_ms(started),
        class.into(),
        status_code,
    )
}

pub(crate) fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub(crate) fn usage_from_standard(response: &Value) -> TokenUsage {
    let usage = response.get("usage").unwrap_or(&Value::Null);
    let cached_tokens = usage
        .get("cached_tokens")
        .or_else(|| usage.get("cache_read_input_tokens"))
        .or_else(|| usage.get("cached_content_token_count"))
        .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.pointer("/input_token_details/cache_read"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32;

    let cache_write_tokens = usage
        .get("cache_write_tokens")
        .or_else(|| usage.get("cache_creation_input_tokens"))
        .or_else(|| usage.pointer("/input_token_details/cache_creation"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32;

    TokenUsage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        completion_tokens: usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        cached_tokens,
        cache_write_tokens,
        ..TokenUsage::default()
    }
}

pub(crate) fn render_receipts(receipts: &[ProviderAttemptReceipt]) -> String {
    serde_json::to_string(receipts).unwrap_or_else(|_| "[]".to_string())
}

pub(crate) fn failed_route_message(receipts: &[ProviderAttemptReceipt]) -> String {
    format!(
        "model request failed after {} Provider Route attempt(s); receipts={}",
        receipts.len(),
        render_receipts(receipts)
    )
}

