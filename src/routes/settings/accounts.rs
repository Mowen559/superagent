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

pub(crate) async fn toggle_account(
    Path(account_id): Path<String>,
    Json(body): Json<ToggleBody>,
) -> axum::response::Response {
    let mut settings = llm_settings::read_llm_settings();
    let mut found = false;
    for account in settings.accounts.iter_mut() {
        if account.id == account_id {
            let new_state = body.state.unwrap_or(!account.enabled);
            account.enabled = new_state;
            found = true;
            break;
        }
    }

    if !found {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Account with ID '{}' not found", account_id)})),
        )
            .into_response();
    }

    let updated =
        llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
    Json(json!({"success": true, "settings": redacted_settings_value(updated)})).into_response()
}

pub(crate) async fn remove_account(Path(account_id): Path<String>) -> axum::response::Response {
    let mut settings = llm_settings::read_llm_settings();
    let initial_len = settings.accounts.len();
    settings.accounts.retain(|a| a.id != account_id);

    if settings.accounts.len() == initial_len {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Account with ID '{}' not found", account_id)})),
        )
            .into_response();
    }

    if settings.active_account_id.as_deref() == Some(account_id.as_str()) {
        settings.active_account_id = None;
    }

    let updated =
        llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
    Json(json!({"success": true, "settings": redacted_settings_value(updated)})).into_response()
}


pub(crate) async fn get_accounts(Query(query): Query<AccountQuery>) -> Json<serde_json::Value> {
    let settings = llm_settings::read_llm_settings();
    let mut accounts = settings.accounts.clone();

    // Filter
    if let Some(search) = query.search {
        let search = search.to_lowercase();
        accounts.retain(|a| {
            a.name.to_lowercase().contains(&search) || a.provider.to_lowercase().contains(&search)
        });
    }

    let total = accounts.len();

    // Offset
    let offset = query.offset.unwrap_or(0);
    if offset >= accounts.len() {
        accounts.clear();
    } else {
        accounts = accounts.split_off(offset);
    }

    // Limit
    let limit = query.limit.unwrap_or(20);
    accounts.truncate(limit);

    let redacted_settings = redacted_settings_value(llm_settings::LLMSettings {
        accounts,
        ..Default::default()
    });

    Json(json!({
        "success": true,
        "accounts": redacted_settings["accounts"],
        "total": total,
        "offset": offset,
        "limit": limit
    }))
}


