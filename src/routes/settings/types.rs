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

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TestLlmBody {
    #[serde(default, rename = "accountId", alias = "account_id")]
    pub account_id: Option<String>,
    pub provider: String,
    #[serde(default, rename = "baseUrl", alias = "base_url")]
    pub base_url: Option<String>,
    #[serde(default, rename = "apiKey", alias = "api_key")]
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub prompt: Option<String>,
}


#[derive(Debug, Clone, serde::Deserialize)]
pub struct OAuthStartBody {
    pub provider: String,
    #[serde(rename = "accountName")]
    pub account_name: Option<String>,
    #[serde(rename = "sessionKey")]
    pub session_key: Option<String>,
}


pub(crate) type CapabilityApiError = (StatusCode, Json<serde_json::Value>);


pub(crate) fn require_capability_discovery_header(headers: &HeaderMap) -> Result<(), CapabilityApiError> {
    let supplied = headers
        .get("x-hub-capability-discovery")
        .and_then(|value| value.to_str().ok());
    if supplied == Some("confirm") {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "missing or invalid local capability discovery header",
                "kind": "capability_discovery_header_required"
            })),
        ))
    }
}

pub(crate) fn capability_error(error: CapabilityDiscoveryError) -> CapabilityApiError {
    let status = match error.kind {
        CapabilityDiscoveryErrorKind::InvalidRequest => StatusCode::BAD_REQUEST,
        CapabilityDiscoveryErrorKind::AccountNotFound => StatusCode::NOT_FOUND,
        CapabilityDiscoveryErrorKind::UnsupportedProvider => StatusCode::UNPROCESSABLE_ENTITY,
        CapabilityDiscoveryErrorKind::CatalogRequestFailed
        | CapabilityDiscoveryErrorKind::CatalogResponseInvalid => StatusCode::BAD_GATEWAY,
    };
    (status, Json(json!({ "error": error })))
}


#[derive(Deserialize)]
pub(crate) struct ToggleBody {
    pub(crate) state: Option<bool>,
}


#[derive(Deserialize)]
pub struct AccountQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub search: Option<String>,
}


#[derive(Deserialize)]
pub(crate) struct RoutingBody {
    pub(crate) strategy: String,
}


