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

pub(crate) async fn update_routing(Json(body): Json<RoutingBody>) -> axum::response::Response {
    let mut settings = llm_settings::read_llm_settings();

    if body.strategy != "failover" && body.strategy != "round_robin" {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error": "Strategy must be 'failover' or 'round_robin'"})),
        )
            .into_response();
    }

    settings.routing_strategy = body.strategy.clone();
    let updated =
        llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
    Json(json!({"success": true, "settings": redacted_settings_value(updated)})).into_response()
}


