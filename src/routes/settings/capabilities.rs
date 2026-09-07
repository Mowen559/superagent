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

pub(crate) async fn get_recertification_plan(
    Path(account_id): Path<String>,
) -> Result<Json<serde_json::Value>, CapabilityApiError> {
    let settings = llm_settings::read_llm_settings();
    let account = settings
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": "provider account was not found",
                    "kind": "account_not_found"
                })),
            )
        })?;
    Ok(Json(json!({
        "plan": plan_recertification_for_account(account),
        "networkRequested": false,
    })))
}


pub(crate) async fn discover_capabilities(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CapabilityDiscoveryRequest>,
) -> Result<Json<serde_json::Value>, CapabilityApiError> {
    require_capability_discovery_header(&headers)?;
    let settings = llm_settings::read_llm_settings();
    let runtime = ProviderCapabilityRuntime::new(state.http_client.clone());
    let report = runtime
        .discover(&settings, request)
        .await
        .map_err(capability_error)?;
    let updated = commit_capability_report(&report)?;

    Ok(Json(json!({
        "report": report,
        "committed": true,
        "updatedAt": updated.updated_at,
    })))
}

pub(crate) fn commit_capability_report(
    report: &crate::domains::model_gateway::capabilities::CapabilityDiscoveryReport,
) -> Result<llm_settings::LLMSettings, CapabilityApiError> {
    llm_settings::update_llm_settings(|settings| apply_discovery_report(settings, report)).map_err(
        |error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "capability discovery completed but its configuration checkpoint failed",
                    "kind": "capability_commit_failed",
                    "detail": error,
                })),
            )
        },
    )
}


