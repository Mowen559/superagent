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

pub(crate) fn track_provider_stream(
    mut stream: std::pin::Pin<
        Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
    >,
    health_key: String,
    started: Instant,
) -> std::pin::Pin<
    Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
> {
    Box::pin(async_stream::stream! {
        let mut guard = ProviderStreamHealthGuard::new(health_key, started);
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
    pub(crate) health_key: String,
    pub(crate) started: Instant,
    pub(crate) finished: bool,
}

impl ProviderStreamHealthGuard {
    pub(crate) fn new(health_key: String, started: Instant) -> Self {
        Self {
            health_key,
            started,
            finished: false,
        }
    }

    pub(crate) fn finish(&mut self, outcome: ProviderAttemptOutcome) {
        if self.finished {
            return;
        }
        ProviderHealthRuntime::global().finish_attempt(
            &self.health_key,
            outcome,
            elapsed_ms(self.started),
        );
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

