use std::sync::Arc;
use axum::{Router, routing::{get, post, put, delete}, Json, extract::{State, Path}, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::AppState;
use crate::config::llm_settings;
use crate::services::llm_provider::LlmProvider;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/settings/llm", get(get_llm_settings).put(put_llm_settings))
        .route("/api/settings/models_fetch", get(fetch_models))
        .route("/api/settings/quota", get(fetch_quota))
        .route("/api/settings/accounts/{id}/toggle", post(toggle_account))
        .route("/api/settings/accounts/{id}", delete(remove_account))
        .route("/api/settings/routing", put(update_routing))
}

fn redact_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    if key.len() > 8 {
        format!("{}***{}", &key[0..4], &key[key.len()-4..])
    } else {
        "***".to_string()
    }
}

async fn get_llm_settings() -> Json<serde_json::Value> {
    let mut settings = llm_settings::read_llm_settings();
    settings.api_key = redact_api_key(&settings.api_key);
    Json(serde_json::to_value(settings).unwrap_or_default())
}

async fn put_llm_settings(
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let updated = llm_settings::write_llm_settings(body);
    Json(serde_json::to_value(updated).unwrap_or_default())
}

async fn fetch_models(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let provider = LlmProvider::new(state.http_client.clone());
    if !provider.is_available() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error": "LLM provider is not fully configured"}))
        ).into_response();
    }
    
    match provider.fetch_models_with_info().await {
        Ok((models, models_info)) => {
            let mut settings = llm_settings::read_llm_settings();
            let mut updated = false;
            
            // Try to find the active account and update its models_info
            if let Some(active_id) = &settings.active_account_id {
                for account in settings.accounts.iter_mut() {
                    if account.id == *active_id {
                        account.models_info = Some(models_info.clone());
                        updated = true;
                        break;
                    }
                }
            } else if !settings.accounts.is_empty() {
                // Fallback: update the first account if no active_account_id is set
                settings.accounts[0].models_info = Some(models_info);
                updated = true;
            }
            
            if updated {
                llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
            }
            
            Json(json!({"models": models})).into_response()
        },
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e}))
        ).into_response()
    }
}

async fn fetch_quota() -> axum::response::Response {
    // For now we run the terminal task in the background and return a 202 accepted
    // since handle_quota() currently prints to stdout instead of returning data.
    // In the future this should be refactored in subscription.rs to return data.
    tokio::spawn(async move {
        crate::services::subscription::handle_quota().await;
    });
    Json(json!({"message": "Quota check started. Please check the server logs for the output."})).into_response()
}

#[derive(Deserialize)]
struct ToggleBody {
    state: Option<bool>,
}

async fn toggle_account(Path(account_id): Path<String>, Json(body): Json<ToggleBody>) -> axum::response::Response {
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
            Json(json!({"error": format!("Account with ID '{}' not found", account_id)}))
        ).into_response();
    }
    
    let updated = llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
    Json(json!({"success": true, "settings": updated})).into_response()
}

async fn remove_account(Path(account_id): Path<String>) -> axum::response::Response {
    let mut settings = llm_settings::read_llm_settings();
    let initial_len = settings.accounts.len();
    settings.accounts.retain(|a| a.id != account_id);
    
    if settings.accounts.len() == initial_len {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Account with ID '{}' not found", account_id)}))
        ).into_response();
    }
    
    if settings.active_account_id.as_deref() == Some(account_id.as_str()) {
        settings.active_account_id = None;
    }
    
    let updated = llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
    Json(json!({"success": true, "settings": updated})).into_response()
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
            Json(json!({"error": "Strategy must be 'failover' or 'round_robin'"}))
        ).into_response();
    }
    
    settings.routing_strategy = body.strategy.clone();
    let updated = llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
    Json(json!({"success": true, "settings": updated})).into_response()
}
