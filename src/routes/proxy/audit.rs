use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use serde_json::Value;

use crate::{
    config::llm_settings,
    services::{
        auth_validator,
        brain::model_route_runtime::{
            provider_health_key, ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome,
            ProviderAttemptReceipt,
        },
        brain::provider_health_runtime::{ProviderAdmission, ProviderHealthRuntime},
        protocol_converter::{
            anthropic::AnthropicConverter, gemini::GeminiConverter, openai::OpenAiConverter,
            openai_responses::OpenAiResponsesConverter, upstream::adapter_for_provider,
            ProtocolConverter,
        },
        usage_tracker::ApiUsageRecord,
    },
    AppState,
};

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};


use super::*;

pub(crate) fn json_error_body(raw: &str) -> Value {
    let mut end = raw.len().min(512);
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::json!({"error": {"message": &raw[..end]}})
}

pub(crate) fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ExtractedUsage {
    pub(crate) prompt_tokens: u32,
    pub(crate) completion_tokens: u32,
    pub(crate) cached_tokens: u32,
    pub(crate) cache_write_tokens: u32,
}

pub(crate) fn usage_from_value(usage: Option<&Value>) -> ExtractedUsage {
    let Some(usage) = usage else {
        return ExtractedUsage::default();
    };
    ExtractedUsage {
        prompt_tokens: token_field(usage, &["prompt_tokens", "input_tokens"]),
        completion_tokens: token_field(usage, &["completion_tokens", "output_tokens"]),
        cached_tokens: token_field(
            usage,
            &[
                "cached_tokens",
                "cache_read_input_tokens",
                "cached_content_token_count",
                "prompt_tokens_details.cached_tokens",
                "input_tokens_details.cached_tokens",
                "input_token_details.cache_read",
            ],
        ),
        cache_write_tokens: token_field(
            usage,
            &[
                "cache_write_tokens",
                "cache_creation_input_tokens",
                "input_token_details.cache_creation",
            ],
        ),
    }
}

pub(crate) fn token_field(value: &Value, paths: &[&str]) -> u32 {
    paths
        .iter()
        .find_map(|path| {
            path.split('.')
                .try_fold(value, |current, segment| current.get(segment))
                .and_then(Value::as_u64)
        })
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32
}

pub(crate) fn protocol_label(provider: &str) -> String {
    match provider {
        "anthropic" | "claude" => "ANTHROPIC",
        "gemini" | "gemini_native" => "GEMINI",
        _ => "OpenAI",
    }
    .to_string()
}

pub(crate) fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

pub(crate) fn record_proxy_audit(record: ApiUsageRecord) {
    crate::services::usage_tracker::record_api_call(record);
}


pub(crate) struct StreamAuditState {
    pub(crate) account_id: String,
    pub(crate) model: String,
    pub(crate) protocol: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) status: u16,
    pub(crate) started: Instant,
    pub(crate) request_body: String,
    pub(crate) response_body: String,
    pub(crate) attempt_receipts: Vec<ProviderAttemptReceipt>,
    pub(crate) usage: ExtractedUsage,
    pub(crate) recorded: bool,
}

impl StreamAuditState {
    pub(crate) fn new(
        account_id: String,
        model: String,
        protocol: String,
        method: String,
        path: String,
        status: u16,
        started: Instant,
        request_body: String,
        attempt_receipts: Vec<ProviderAttemptReceipt>,
    ) -> Self {
        Self {
            account_id,
            model,
            protocol,
            method,
            path,
            status,
            started,
            request_body,
            response_body: String::new(),
            attempt_receipts,
            usage: ExtractedUsage::default(),
            recorded: false,
        }
    }

    pub(crate) fn push_response(&mut self, text: &str) {
        pub(crate) const MAX_RESPONSE_CHARS: usize = 200_000;
        let remaining = MAX_RESPONSE_CHARS.saturating_sub(self.response_body.chars().count());
        if remaining > 0 {
            self.response_body.extend(text.chars().take(remaining));
        }
    }

    pub(crate) fn apply_usage(&mut self, usage: ExtractedUsage) {
        self.usage = usage;
    }

    pub(crate) fn record_once(&mut self) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        let cost = crate::services::cost_calculator::CostCalculator::calculate_cost(
            &self.model,
            self.usage.prompt_tokens,
            self.usage.completion_tokens,
            self.usage.cached_tokens,
            self.usage.cache_write_tokens,
        );
        record_proxy_audit(ApiUsageRecord {
            account_id: self.account_id.clone(),
            model: self.model.clone(),
            protocol: self.protocol.clone(),
            method: self.method.clone(),
            path: self.path.clone(),
            status: self.status,
            duration_ms: elapsed_ms(self.started),
            prompt_tokens: self.usage.prompt_tokens,
            completion_tokens: self.usage.completion_tokens,
            cached_tokens: self.usage.cached_tokens,
            cache_write_tokens: self.usage.cache_write_tokens,
            total_cost: cost,
            request_body: Some(self.request_body.clone()),
            response_body: Some(self.response_body.clone()),
            attempt_receipts: Some(
                serde_json::to_value(&self.attempt_receipts).unwrap_or(Value::Null),
            ),
        });
    }
}

impl Drop for StreamAuditState {
    fn drop(&mut self) {
        if !self.recorded {
            if self.usage.prompt_tokens == 0 && self.usage.completion_tokens == 0 {
                // Try parsing the entire response_body as JSON for non-streaming scenarios
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&self.response_body) {
                    if let Some(usage) = json.get("usage") {
                        self.apply_usage(usage_from_value(Some(usage)));
                    }
                }
            }
            self.record_once();
        }
    }
}


