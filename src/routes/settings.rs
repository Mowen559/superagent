use crate::config::llm_settings;
use crate::services::brain::provider_capability_runtime::{
    apply_discovery_report, plan_recertification_for_account, CapabilityDiscoveryError,
    CapabilityDiscoveryErrorKind, CapabilityDiscoveryRequest, ProviderCapabilityRuntime,
};
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

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
        .route("/api/settings/accounts/{id}/toggle", post(toggle_account))
        .route("/api/settings/accounts/{id}", delete(remove_account))
        .route("/api/settings/routing", put(update_routing))
}

async fn get_recertification_plan(
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

type CapabilityApiError = (StatusCode, Json<serde_json::Value>);

async fn discover_capabilities(
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

fn commit_capability_report(
    report: &crate::services::brain::provider_capability_runtime::CapabilityDiscoveryReport,
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

fn require_capability_discovery_header(headers: &HeaderMap) -> Result<(), CapabilityApiError> {
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

fn capability_error(error: CapabilityDiscoveryError) -> CapabilityApiError {
    let status = match error.kind {
        CapabilityDiscoveryErrorKind::InvalidRequest => StatusCode::BAD_REQUEST,
        CapabilityDiscoveryErrorKind::AccountNotFound => StatusCode::NOT_FOUND,
        CapabilityDiscoveryErrorKind::UnsupportedProvider => StatusCode::UNPROCESSABLE_ENTITY,
        CapabilityDiscoveryErrorKind::CatalogRequestFailed
        | CapabilityDiscoveryErrorKind::CatalogResponseInvalid => StatusCode::BAD_GATEWAY,
    };
    (status, Json(json!({ "error": error })))
}

fn redact_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    if key.len() > 8 {
        format!("{}***{}", &key[0..4], &key[key.len() - 4..])
    } else {
        "***".to_string()
    }
}

async fn get_llm_settings() -> Json<serde_json::Value> {
    Json(redacted_settings_value(llm_settings::read_llm_settings()))
}

async fn put_llm_settings(Json(mut body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    restore_redacted_secrets(&mut body, &llm_settings::read_llm_settings());
    let updated = llm_settings::write_llm_settings(body);
    Json(redacted_settings_value(updated))
}

async fn fetch_models(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, CapabilityApiError> {
    require_capability_discovery_header(&headers)?;
    let settings = llm_settings::read_llm_settings();
    let account_id = settings
        .active_account_id
        .clone()
        .or_else(|| settings.accounts.first().map(|account| account.id.clone()))
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "an existing provider account is required for model discovery",
                    "kind": "provider_account_required"
                })),
            )
        })?;
    let report = ProviderCapabilityRuntime::new(state.http_client.clone())
        .discover(
            &settings,
            CapabilityDiscoveryRequest {
                account_id,
                models: Vec::new(),
                profiles: Vec::new(),
                include_catalog: true,
                run_probes: false,
                confirm_network: true,
                revalidate_expired: false,
            },
        )
        .await
        .map_err(capability_error)?;
    let models = report
        .catalog
        .as_ref()
        .map(|catalog| catalog.models.clone())
        .unwrap_or_default();
    let updated = commit_capability_report(&report)?;
    Ok(Json(json!({
        "models": models,
        "accountId": report.account_id,
        "committed": true,
        "updatedAt": updated.updated_at,
    })))
}

fn redacted_settings_value(mut settings: llm_settings::LLMSettings) -> serde_json::Value {
    settings.api_key = redact_api_key(&settings.api_key);
    for account in &mut settings.accounts {
        account.api_key = redact_api_key(&account.api_key);
        account.refresh_token = account.refresh_token.as_ref().map(|_| "***".to_string());
    }
    serde_json::to_value(settings).unwrap_or_default()
}

fn restore_redacted_secrets(body: &mut serde_json::Value, current: &llm_settings::LLMSettings) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    if object
        .get("apiKey")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.contains("***"))
    {
        object.insert(
            "apiKey".to_string(),
            serde_json::Value::String(current.api_key.clone()),
        );
    }
    let Some(accounts) = object
        .get_mut("accounts")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for value in accounts {
        let Some(account) = value.as_object_mut() else {
            continue;
        };
        let Some(id) = account
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string)
        else {
            continue;
        };
        let Some(existing) = current.accounts.iter().find(|candidate| candidate.id == id) else {
            continue;
        };
        if account
            .get("apiKey")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.contains("***"))
        {
            account.insert(
                "apiKey".to_string(),
                serde_json::Value::String(existing.api_key.clone()),
            );
        }
        if account
            .get("refreshToken")
            .is_some_and(|value| value.is_null() || value.as_str() == Some("***"))
        {
            account.insert(
                "refreshToken".to_string(),
                existing
                    .refresh_token
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            );
        }
    }
}

async fn fetch_quota() -> axum::response::Response {
    // For now we run the terminal task in the background and return a 202 accepted
    // since handle_quota() currently prints to stdout instead of returning data.
    // In the future this should be refactored in subscription.rs to return data.
    tokio::spawn(async move {
        crate::services::subscription::handle_quota().await;
    });
    Json(json!({"message": "Quota check started. Please check the server logs for the output."}))
        .into_response()
}

#[derive(Deserialize)]
struct ToggleBody {
    state: Option<bool>,
}

async fn toggle_account(
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

async fn remove_account(Path(account_id): Path<String>) -> axum::response::Response {
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

#[derive(Deserialize)]
struct RoutingBody {
    strategy: String,
}

async fn update_routing(Json(body): Json<RoutingBody>) -> axum::response::Response {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_discovery_requires_local_control_plane_header() {
        let error = require_capability_discovery_header(&HeaderMap::new()).unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-capability-discovery", "confirm".parse().unwrap());
        assert!(require_capability_discovery_header(&headers).is_ok());
    }

    #[test]
    fn settings_responses_redact_account_credentials_and_round_trip_masks() {
        let current = llm_settings::LLMSettings {
            api_key: "top-level-secret".to_string(),
            accounts: vec![llm_settings::LLMAccount {
                id: "account-1".to_string(),
                api_key: "account-secret".to_string(),
                refresh_token: Some("refresh-secret".to_string()),
                ..llm_settings::LLMAccount::default()
            }],
            ..llm_settings::LLMSettings::default()
        };
        let mut redacted = redacted_settings_value(current.clone());
        assert!(!redacted.to_string().contains("account-secret"));
        assert!(!redacted.to_string().contains("refresh-secret"));

        restore_redacted_secrets(&mut redacted, &current);
        assert_eq!(redacted["accounts"][0]["apiKey"], "account-secret");
        assert_eq!(redacted["accounts"][0]["refreshToken"], "refresh-secret");
    }
}
