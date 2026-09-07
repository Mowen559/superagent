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

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/messages", post(handle_anthropic))
        .route("/v1/chat/completions", post(handle_openai_chat))
        .route("/v1/completions", post(handle_openai_completions))
        .route("/v1/responses", post(handle_openai_responses))
        .route("/v1/models", get(handle_unified_models))
        .route("/v1beta/models/{*path}", post(handle_gemini))
}

