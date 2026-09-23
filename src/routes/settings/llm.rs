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


use super::capabilities::commit_capability_report;
use super::types::*;

pub(crate) async fn test_llm(Json(body): Json<TestLlmBody>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let client = reqwest::Client::new();

    let model = body
        .model
        .clone()
        .unwrap_or_else(|| "test-model".to_string());
    let prompt = body.prompt.clone().unwrap_or_else(|| "hi".to_string());
    let mut api_key = body.api_key.clone();
    let mut base_url = body.base_url.clone();

    // Inject token if empty, masked, or if account_id is provided
    let mut global_settings = crate::config::llm_settings::read_llm_settings();
    let mut needs_save = false;
    if let Some(ref acc_id) = body.account_id {
        if let Some(account) = global_settings
            .accounts
            .iter_mut()
            .find(|a| &a.id == acc_id)
        {
            // Refresh token if needed
            needs_save =
                crate::services::cli_auth::refresh_account_token_if_needed(account, &client).await;
            api_key = Some(account.api_key.clone());
            if account.base_url.len() > 0 {
                base_url = Some(account.base_url.clone());
            }
        }
    } else if let (Some(provider), None) = (Some(&body.provider), &api_key) {
        if provider == "openai" {
            // Find active account token if possible, else skip
            if let Some(account) = global_settings
                .accounts
                .iter()
                .find(|a| a.provider == "openai")
            {
                let token = &account.api_key;
                if token.starts_with("eyJ") {
                    api_key = Some(token.clone());
                }
            }
        }
    }

    if needs_save {
        crate::config::llm_settings::write_llm_settings(
            serde_json::to_value(&global_settings).unwrap_or_default(),
        );
    }

    // Inject base url if empty
    if let (Some(provider), None) = (Some(&body.provider), &base_url) {
        if provider == "openai" {
            base_url = Some("https://api.openai.com/v1".to_string());
        }
    }

    // Construct dummy LLMSettings for the adapter
    let settings = crate::config::llm_settings::LLMSettings {
        provider: body.provider.clone(),
        base_url: base_url.unwrap_or_default(),
        api_key: api_key.unwrap_or_default(),
        model: model.clone(),
        ..crate::config::llm_settings::LLMSettings::default()
    };

    let standard_body = serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": prompt
            }
        ],
        "max_tokens": 512,
        "stream": true
    });

    let adapter = crate::domains::model_gateway::protocols::upstream::adapter_for_provider(&settings);

    let (url, headers, request_body) = match adapter.prepare_request(&settings, standard_body) {
        Ok(req) => req,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Failed to prepare request: {}", e)})),
            )
                .into_response();
        }
    };

    tracing::info!("Test LLM request -> URL: {}", url);
    tracing::info!("Test LLM request -> Headers: {:?}", headers);
    tracing::info!(
        "Test LLM request -> Body: {}",
        serde_json::to_string(&request_body).unwrap_or_default()
    );

    let response = client
        .post(&url)
        .headers(headers)
        .json(&request_body)
        .send()
        .await;

    match response {
        Ok(res) => {
            let status = res.status();
            let is_custom_model = body.model.is_some();

            tracing::info!("Test LLM response -> Status: {}", status);

            if status.is_success() {
                use futures::stream::StreamExt;
                let mut bytes_stream = res
                    .bytes_stream()
                    .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
                let mut accumulated_raw = String::new();
                while let Some(chunk_res) = bytes_stream.next().await {
                    if let Ok(bytes) = chunk_res {
                        accumulated_raw.push_str(&String::from_utf8_lossy(&bytes));
                    }
                }

                let mut parsed_text = String::new();
                for line in accumulated_raw.lines() {
                    if line.starts_with("data: ") && line != "data: [DONE]" {
                        let json_str = &line[6..];
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                            if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array())
                            {
                                if let Some(choice) = choices.get(0) {
                                    if let Some(delta) = choice.get("delta") {
                                        if let Some(content) =
                                            delta.get("content").and_then(|c| c.as_str())
                                        {
                                            parsed_text.push_str(content);
                                        }
                                    }
                                }
                            } else if let Some(event_type) =
                                parsed.get("type").and_then(|v| v.as_str())
                            {
                                if event_type == "response.text.delta"
                                    || event_type == "response.output_text.delta"
                                {
                                    if let Some(delta) =
                                        parsed.get("delta").and_then(|v| v.as_str())
                                    {
                                        parsed_text.push_str(delta);
                                    }
                                }
                            } else if let Some(candidates) =
                                parsed.get("candidates").and_then(|c| c.as_array())
                            {
                                if let Some(candidate) = candidates.get(0) {
                                    if let Some(content) = candidate.get("content") {
                                        if let Some(parts) =
                                            content.get("parts").and_then(|p| p.as_array())
                                        {
                                            if let Some(part) = parts.get(0) {
                                                if let Some(text) =
                                                    part.get("text").and_then(|t| t.as_str())
                                                {
                                                    parsed_text.push_str(text);
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if let Some(response_obj) = parsed.get("response") {
                                if let Some(candidates) =
                                    response_obj.get("candidates").and_then(|c| c.as_array())
                                {
                                    if let Some(candidate) = candidates.get(0) {
                                        if let Some(content) = candidate.get("content") {
                                            if let Some(parts) =
                                                content.get("parts").and_then(|p| p.as_array())
                                            {
                                                if let Some(part) = parts.get(0) {
                                                    if let Some(text) =
                                                        part.get("text").and_then(|t| t.as_str())
                                                    {
                                                        parsed_text.push_str(text);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let final_res = if parsed_text.is_empty() {
                    if accumulated_raw.is_empty() {
                        "连接成功 (Connection successful, but empty stream response)".to_string()
                    } else {
                        format!("连接成功 (Connection successful, but failed to parse stream). Raw response:\n{}", accumulated_raw.chars().take(2000).collect::<String>())
                    }
                } else {
                    parsed_text
                };

                (
                    axum::http::StatusCode::OK,
                    Json(serde_json::json!({"success": true, "response": final_res})),
                )
                    .into_response()
            } else if (status.as_u16() == 404 || status.as_u16() == 400) && !is_custom_model {
                (
                    axum::http::StatusCode::OK,
                    Json(serde_json::json!({"success": true, "response": "连接成功 (Connection successful, using test-model)"})),
                )
                    .into_response()
            } else {
                let err_text = res.text().await.unwrap_or_default();
                (
                    status,
                    Json(serde_json::json!({"success": false, "error": format!("API returned status {}: {}", status, err_text)})),
                )
                    .into_response()
            }
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Request failed: {}", e)})),
        )
            .into_response(),
    }
}

pub(crate) async fn test_models(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TestLlmBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let client = state.http_client.clone();

    let mut api_key = body.api_key.clone();
    let mut base_url = body.base_url.clone();

    // Inject token if empty, masked, or if account_id is provided
    let mut global_settings = crate::config::llm_settings::read_llm_settings();
    if let Some(ref acc_id) = body.account_id {
        if let Some(account) = global_settings
            .accounts
            .iter_mut()
            .find(|a| &a.id == acc_id)
        {
            crate::services::cli_auth::refresh_account_token_if_needed(account, &client).await;
            api_key = Some(account.api_key.clone());
            if account.base_url.len() > 0 {
                base_url = Some(account.base_url.clone());
            }
        }
    } else if let (Some(provider), None) = (Some(&body.provider), &api_key) {
        if provider == "openai" {
            // Find active account token if possible, else skip
            if let Some(account) = global_settings
                .accounts
                .iter()
                .find(|a| a.provider == "openai")
            {
                let token = &account.api_key;
                if token.starts_with("eyJ") {
                    api_key = Some(token.clone());
                }
            }
        }
    }

    // Inject base url if empty
    if let (Some(provider), None) = (Some(&body.provider), &base_url) {
        if provider == "openai" {
            base_url = Some("https://api.openai.com/v1".to_string());
        }
    }

    let account_id = body
        .account_id
        .unwrap_or_else(|| "test-account".to_string());

    let dummy_account = crate::config::llm_settings::LLMAccount {
        id: account_id.clone(),
        provider: body.provider.clone(),
        base_url: base_url.unwrap_or_default(),
        api_key: api_key.unwrap_or_default(),
        ..crate::config::llm_settings::LLMAccount::default()
    };

    let mut test_settings = crate::config::llm_settings::LLMSettings::default();
    test_settings.accounts.push(dummy_account);
    test_settings.active_account_id = Some(account_id.clone());

    let report = ProviderCapabilityRuntime::new(client)
        .discover(
            &test_settings,
            CapabilityDiscoveryRequest {
                account_id: account_id.clone(),
                models: Vec::new(),
                profiles: Vec::new(),
                include_catalog: true,
                run_probes: false,
                confirm_network: true,
                revalidate_expired: false,
            },
        )
        .await;

    match report {
        Ok(r) => {
            let models = r
                .catalog
                .as_ref()
                .map(|catalog| catalog.models.clone())
                .unwrap_or_default();
            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "models": models,
                    "accountId": r.account_id,
                })),
            )
                .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to fetch models: {}", e)
            })),
        )
            .into_response(),
    }
}


pub(crate) fn redact_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    if key.len() > 8 {
        format!("{}***{}", &key[0..4], &key[key.len() - 4..])
    } else {
        "***".to_string()
    }
}

pub(crate) async fn get_llm_settings() -> Json<serde_json::Value> {
    Json(redacted_settings_value(llm_settings::read_llm_settings()))
}

pub(crate) async fn put_llm_settings(Json(mut body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    restore_redacted_secrets(&mut body, &llm_settings::read_llm_settings());
    let updated = llm_settings::write_llm_settings(body);
    Json(redacted_settings_value(updated))
}

pub(crate) async fn fetch_models(
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

pub(crate) fn redacted_settings_value(mut settings: llm_settings::LLMSettings) -> serde_json::Value {
    settings.api_key = redact_api_key(&settings.api_key);
    for account in &mut settings.accounts {
        account.api_key = redact_api_key(&account.api_key);
        account.refresh_token = account.refresh_token.as_ref().map(|_| "***".to_string());
    }
    serde_json::to_value(settings).unwrap_or_default()
}

pub(crate) fn restore_redacted_secrets(body: &mut serde_json::Value, current: &llm_settings::LLMSettings) {
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


pub(crate) async fn fetch_quota() -> axum::response::Response {
    // For now we run the terminal task in the background and return a 202 accepted
    // since handle_quota() currently prints to stdout instead of returning data.
    // In the future this should be refactored in subscription.rs to return data.
    tokio::spawn(async move {
        crate::services::subscription::handle_quota().await;
    });
    Json(json!({"message": "Quota check started. Please check the server logs for the output."}))
        .into_response()
}


