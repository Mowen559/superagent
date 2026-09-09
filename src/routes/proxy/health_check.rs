use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use serde_json::Value;

use crate::config::llm_settings;
use crate::services::auth_validator;
use crate::services::usage_tracker::ApiUsageRecord;
use crate::domains::model_gateway::routing::{provider_health_key, ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome, ProviderAttemptReceipt};
use crate::domains::model_gateway::health::{ProviderAdmission, ProviderHealthRuntime};
use crate::domains::model_gateway::protocols::{anthropic::AnthropicConverter, gemini::GeminiConverter, openai::OpenAiConverter, openai_responses::OpenAiResponsesConverter, upstream::adapter_for_provider, ProtocolConverter};
use crate::AppState;

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};


use super::*;

pub(crate) fn track_provider_stream(
    mut stream: std::pin::Pin<
        Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
    >,
    acc_key: String,
    mod_key: String,
    started: Instant,
) -> std::pin::Pin<
    Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
> {
    Box::pin(async_stream::stream! {
        let mut guard = ProviderStreamHealthGuard::new(acc_key, mod_key, started);
        while let Some(item) = futures::StreamExt::next(&mut stream).await {
            match &item {
                Ok(chunk) => {
                    if let Some(outcome) = canonical_stream_terminal_outcome(chunk) {
                        guard.finish(outcome);
                    }
                }
                Err(_) => guard.finish(ProviderAttemptOutcome::Transient),
            }
            let failed = item.is_err();
            yield item;
            if failed {
                break;
            }
        }
    })
}

pub(crate) struct ProviderStreamHealthGuard {
    pub(crate) acc_key: String,
    pub(crate) mod_key: String,
    pub(crate) started: Instant,
    pub(crate) finished: bool,
}

impl ProviderStreamHealthGuard {
    pub(crate) fn new(acc_key: String, mod_key: String, started: Instant) -> Self {
        Self {
            acc_key,
            mod_key,
            started,
            finished: false,
        }
    }

    pub(crate) fn finish(&mut self, outcome: ProviderAttemptOutcome) {
        if self.finished {
            return;
        }
        let duration = elapsed_ms(self.started);
        match outcome {
            ProviderAttemptOutcome::Succeeded => {
                ProviderHealthRuntime::global().finish_attempt(&self.mod_key, ProviderAttemptOutcome::Succeeded, duration);
                ProviderHealthRuntime::global().finish_attempt(&self.acc_key, ProviderAttemptOutcome::Succeeded, duration);
            }
            ProviderAttemptOutcome::Authentication => {
                ProviderHealthRuntime::global().finish_attempt(&self.acc_key, ProviderAttemptOutcome::Authentication, duration);
                ProviderHealthRuntime::global().finish_attempt(&self.mod_key, ProviderAttemptOutcome::Authentication, duration);
            }
            ProviderAttemptOutcome::RateLimited => {
                ProviderHealthRuntime::global().finish_attempt(&self.mod_key, ProviderAttemptOutcome::RateLimited, duration);
                ProviderHealthRuntime::global().finish_attempt(&self.acc_key, ProviderAttemptOutcome::Succeeded, duration);
            }
            ProviderAttemptOutcome::Transient => {
                ProviderHealthRuntime::global().finish_attempt(&self.mod_key, ProviderAttemptOutcome::Transient, duration);
                ProviderHealthRuntime::global().cancel_attempt(&self.acc_key);
            }
            _ => {
                ProviderHealthRuntime::global().finish_attempt(&self.mod_key, outcome, duration);
                ProviderHealthRuntime::global().finish_attempt(&self.acc_key, ProviderAttemptOutcome::Succeeded, duration);
            }
        }
        self.finished = true;
    }
}

impl Drop for ProviderStreamHealthGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(ProviderAttemptOutcome::Transient);
        }
    }
}

pub(crate) fn canonical_stream_terminal_outcome(chunk: &bytes::Bytes) -> Option<ProviderAttemptOutcome> {
    let text = std::str::from_utf8(chunk).ok()?;
    for line in text.lines() {
        let Some(data) = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
        else {
            continue;
        };
        if data.trim() == "[DONE]" {
            return Some(ProviderAttemptOutcome::Succeeded);
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if value.get("error").is_some() {
            return Some(ProviderAttemptOutcome::Transient);
        }
        let finished = value
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|choices| {
                choices.iter().any(|choice| {
                    choice
                        .get("finish_reason")
                        .is_some_and(|reason| !reason.is_null())
                })
            });
        if finished {
            return Some(ProviderAttemptOutcome::Succeeded);
        }
    }
    None
}

