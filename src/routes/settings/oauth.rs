use crate::config::llm_settings;
use crate::services::brain::provider_capability_runtime::{
    apply_discovery_report, plan_recertification_for_account, CapabilityDiscoveryError,
    CapabilityDiscoveryErrorKind, CapabilityDiscoveryRequest, ProviderCapabilityRuntime,
};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;


use super::*;
use super::types::*;

pub(crate) async fn start_oauth(Json(body): Json<OAuthStartBody>) -> axum::response::Response {
    use axum::response::IntoResponse;

    let result = match body.provider.as_str() {
        "OpenAI Native" => {
            crate::services::cli_auth::handle_openai_pkce_flow(body.account_name).await
        }
        "Gemini Native" => {
            crate::services::cli_auth::handle_gemini_oauth_flow(body.account_name).await
        }
        "Claude Desktop Official" => {
            crate::services::cli_auth::handle_claude_oauth_flow(body.session_key).await
        }
        "GitHub Copilot" => {
            crate::services::cli_auth::handle_github_copilot_flow(body.account_name).await
        }
        _ => Err(format!(
            "OAuth not supported or implemented for provider: {}",
            body.provider
        )),
    };

    match result {
        Ok(account) => axum::Json(serde_json::json!({
            "success": true,
            "account": {
                "baseUrl": account.base_url,
                "projectId": account.project_id,
                "apiKey": account.api_key,
                "id": account.id,
                "refreshToken": account.refresh_token,
                "expiresAt": account.expires_at,
            }
        }))
        .into_response(),
        Err(e) => axum::Json(serde_json::json!({
            "success": false,
            "error": e
        }))
        .into_response(),
    }
}


