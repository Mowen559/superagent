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


pub mod accounts;
pub mod capabilities;
pub mod llm;
pub mod oauth;
pub mod routing;
pub mod types;

use accounts::{get_accounts, remove_account, toggle_account};
use capabilities::{discover_capabilities, get_recertification_plan};
use llm::{fetch_models, fetch_quota, get_llm_settings, put_llm_settings, test_llm, test_models};
use oauth::start_oauth;
use routing::update_routing;

#[cfg(test)]
pub(crate) use types::require_capability_discovery_header;
#[cfg(test)]
pub(crate) use llm::{redacted_settings_value, restore_redacted_secrets};

#[cfg(test)]
mod tests;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/settings/llm",
            get(get_llm_settings).put(put_llm_settings),
        )
        .route("/api/settings/models_fetch", get(fetch_models))
        .route(
            "/api/settings/llm/capabilities/discover",
            post(discover_capabilities),
        )
        .route(
            "/api/settings/llm/capabilities/recertification/{id}",
            get(get_recertification_plan),
        )
        .route("/api/settings/quota", get(fetch_quota))
        .route("/api/settings/accounts", get(get_accounts))
        .route("/api/settings/accounts/{id}/toggle", post(toggle_account))
        .route("/api/settings/accounts/{id}", delete(remove_account))
        .route("/api/settings/routing", put(update_routing))
        .route("/api/settings/llm/test", post(test_llm))
        .route("/api/settings/llm/models/test", post(test_models))
        .route("/api/settings/oauth/start", post(start_oauth))
        .merge(logs::router())
}


pub mod logs;
