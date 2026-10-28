use crate::config::llm_settings;
use crate::domains::model_gateway::capabilities::{
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

    let provider_clean = body.provider.to_ascii_lowercase().replace([' ', '-'], "_");
    let result = match provider_clean.as_str() {
        "openai_native" | "openai" | "codex" => {
            crate::services::cli_auth::handle_openai_pkce_flow(body.account_name).await
        }
        "gemini_native" | "gemini" | "google_official" | "google" => {
            crate::services::cli_auth::handle_gemini_oauth_flow(body.account_name).await
        }
        "claude_desktop_official" | "claude_official" | "claude" | "anthropic" => {
            crate::services::cli_auth::handle_claude_oauth_flow(body.session_key).await
        }
        "github_copilot" | "copilot" | "github" => {
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
                "token": account.api_key,
                "id": account.id,
                "provider": account.provider,
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
