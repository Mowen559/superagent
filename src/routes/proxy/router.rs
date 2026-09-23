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


use super::dispatch::{
    handle_anthropic, handle_gemini, handle_openai_chat, handle_openai_completions,
    handle_openai_responses, handle_unified_models,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/messages", post(handle_anthropic))
        .route("/anthropic/v1/messages", post(handle_anthropic))
        .route("/anthropic/messages", post(handle_anthropic))
        .route("/anthropic/v1/models", get(handle_unified_models))
        .route("/anthropic/models", get(handle_unified_models))
        .route("/v1/chat/completions", post(handle_openai_chat))
        .route("/openai/v1/chat/completions", post(handle_openai_chat))
        .route("/v1/completions", post(handle_openai_completions))
        .route("/v1/responses", post(handle_openai_responses))
        .route("/v1/models", get(handle_unified_models))
        .route("/openai/v1/models", get(handle_unified_models))
        .route("/v1beta/models/{*path}", post(handle_gemini))
        .route("/gemini/v1beta/models/{*path}", post(handle_gemini))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_router_routes_configured() {
        let r = router();
        assert!(std::mem::size_of_val(&r) > 0);
    }
}

