use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use serde_json::Value;

use crate::config::llm_settings::{self, LLMAccount};
use crate::services::auth_validator;
use crate::services::usage_tracker::ApiUsageRecord;
use crate::domains::model_gateway::routing::{
    account_health_key, model_health_key, provider_health_key, ModelRequirements,
    ModelRouteRuntime, ProviderAttemptOutcome, ProviderAttemptReceipt,
};
use crate::domains::model_gateway::health::{ProviderAdmission, ProviderHealthRuntime};
use crate::domains::model_gateway::protocols::{anthropic::AnthropicConverter, gemini::GeminiConverter, openai::OpenAiConverter, openai_responses::OpenAiResponsesConverter, upstream::adapter_for_provider, ProtocolConverter};
use crate::AppState;

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};


use super::audit::{
    pretty_json, protocol_label, record_proxy_audit, StreamAuditState, usage_from_value,
};
use super::health_check::track_provider_stream;

pub(crate) async fn handle_request(
    state: State<Arc<AppState>>,
    method: String,
    path: String,
    protocol: String,
    headers: HeaderMap,
    converter: &dyn ProtocolConverter,
    body: Value,
) -> Result<Response, (StatusCode, String)> {
    let inbound_body_text = pretty_json(&body);
    let settings = llm_settings::read_llm_settings();

    let requested_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(&settings.model)
        .to_string();
    let requested_tags: Vec<String> = headers
        .get("X-Hub-Routing-Tags")
        .and_then(|value| value.to_str().ok())
        .map(|tags| {
            tags.split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
        
    let standard_body = converter
        .request_to_standard(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let is_stream = standard_body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut raw_token = None;
    if let Some(auth_val) = headers.get("Authorization").and_then(|v| v.to_str().ok()) {
        if let Some(stripped) = auth_val.strip_prefix("Bearer ") {
            raw_token = Some(stripped.trim());
        } else {
            raw_token = Some(auth_val.trim());
        }
    } else if let Some(key_val) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        raw_token = Some(key_val.trim());
    } else if let Some(key_val) = headers.get("anthropic-auth-token").and_then(|v| v.to_str().ok()) {
        raw_token = Some(key_val.trim());
    } else if let Some(key_val) = headers.get("api-key").and_then(|v| v.to_str().ok()) {
        raw_token = Some(key_val.trim());
    }

    let mut override_token = None;
    if let Some(token) = raw_token {
        if !token.is_empty() {
            if !auth_validator::is_api_key(token) {
                match auth_validator::validate_oauth_token(
                    &state.http_client,
                    token,
                    &state.runtime_config.sso_api_url,
                )
                .await
                {
                    Ok(real_api_key) => {
                        override_token = Some(real_api_key);
                    }
                    Err(e) => {
                        return Err((StatusCode::UNAUTHORIZED, e));
                    }
                }
            } else {
                override_token = Some(token.to_string());
            }
        }
    }
    
    let attempt_started = Instant::now();

    let execute_result = crate::domains::model_gateway::gateway::execute_model_request(
        &state.http_client,
        &requested_model,
        &requested_tags,
        standard_body,
        override_token,
    ).await;

    let mut res = match execute_result {
        Ok(r) => r,
        Err(e) => return Err(e),
    };

    let mut response_builder = axum::response::Response::builder().status(res.status);

    if let Some(raw_res) = res.raw_response.take() {
        if let Some(content_type) = raw_res.headers().get("Content-Type") {
            response_builder = response_builder.header("Content-Type", content_type);
        }

        let raw_stream = raw_res.bytes_stream();
        let io_stream = futures::stream::StreamExt::map(raw_stream, |r| {
            r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        });
        
        let account = res.account.unwrap_or_default();
        let dummy_settings = crate::config::llm_settings::LLMSettings {
            provider: account.provider.clone(),
            base_url: account.base_url.clone(),
            api_key: account.api_key.clone(),
            model: requested_model.clone(),
            ..crate::config::llm_settings::LLMSettings::default()
        };
        let upstream_adapter = adapter_for_provider(&dummy_settings);
        let standard_stream = upstream_adapter.response_stream_to_standard(Box::pin(io_stream));
        let acc_key = account_health_key(&account);
        let mod_key = model_health_key(&account, &requested_model);
        let standard_stream = track_provider_stream(standard_stream, acc_key, mod_key, attempt_started);

        let stream_audit = Arc::new(Mutex::new(StreamAuditState::new(
            account.id.clone(),
            requested_model.clone(),
            protocol_label(&account.provider),
            method.clone(),
            path.clone(),
            res.status.as_u16(),
            attempt_started,
            inbound_body_text.clone(),
            res.receipts,
        )));
        let intercepted_stream = futures::stream::StreamExt::map(standard_stream, move |chunk_res| {
            if let Ok(chunk) = &chunk_res {
                if let Ok(text) = std::str::from_utf8(chunk) {
                    if let Ok(mut audit) = stream_audit.lock() {
                        audit.push_response(text);
                    }
                    for line in text.lines() {
                        if line.starts_with("data: ") && line != "data: [DONE]" {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line[6..]) {
                                // 1. Check for first token (TTFT)
                                if let Some(choices) = json.get("choices").and_then(Value::as_array) {
                                    for choice in choices {
                                        if let Some(delta) = choice.get("delta") {
                                            let has_content = delta
                                                .get("content")
                                                .and_then(Value::as_str)
                                                .is_some_and(|s| !s.is_empty());
                                            let has_reasoning = delta
                                                .get("reasoning")
                                                .or_else(|| delta.get("reasoning_content"))
                                                .and_then(Value::as_str)
                                                .is_some_and(|s| !s.is_empty());
                                            let has_tools = delta
                                                .get("tool_calls")
                                                .and_then(Value::as_array)
                                                .is_some_and(|arr| !arr.is_empty());

                                            if has_content || has_reasoning || has_tools {
                                                if let Ok(mut audit) = stream_audit.lock() {
                                                    audit.record_ttft();
                                                }
                                                break;
                                            }
                                        }
                                    }
                                }

                                if let Some(usage) = json.get("usage") {
                                    if let Ok(mut audit) = stream_audit.lock() {
                                        audit.apply_usage(usage_from_value(Some(usage)));
                                    }
                                }
                                let finished = json
                                    .get("choices")
                                    .and_then(Value::as_array)
                                    .is_some_and(|choices| {
                                        choices.iter().any(|choice| {
                                            choice
                                                .get("finish_reason")
                                                .is_some_and(|reason| !reason.is_null())
                                        })
                                    });
                                if finished {
                                    if let Ok(mut audit) = stream_audit.lock() {
                                        audit.record_once();
                                    }
                                }
                            }
                        } else if line == "data: [DONE]" {
                            if let Ok(mut audit) = stream_audit.lock() {
                                audit.record_once();
                            }
                        }
                    }
                }
            }
            chunk_res
        });

        let final_stream = converter.standard_to_response_stream(Box::pin(intercepted_stream));
        let body_stream = axum::body::Body::from_stream(final_stream);

        return response_builder
            .body(body_stream)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    if let Some(error_text) = res.response_body_text {
        if !res.status.is_success() {
            let account = res.account.unwrap_or_default();
            record_proxy_audit(ApiUsageRecord {
                account_id: account.id.clone(),
                model: requested_model.clone(),
                protocol: protocol_label(&account.provider),
                method: method.clone(),
                path: path.clone(),
                status: res.status.as_u16(),
                duration_ms: crate::domains::model_gateway::providers::elapsed_ms(attempt_started),
                ttft_ms: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                reasoning_tokens: 0,
                cached_tokens: 0,
                cache_write_tokens: 0,
                total_cost: 0.0,
                request_body: Some(inbound_body_text.clone()),
                response_body: Some(error_text.clone()),
                attempt_receipts: Some(serde_json::to_value(&res.receipts).unwrap_or(Value::Null)),
            });
            response_builder = response_builder.header("Content-Type", "application/json");
            return response_builder
                .body(axum::body::Body::from(error_text))
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    }

    let standard_res = res.standard_response.unwrap_or_default();
    let final_res = match converter.standard_to_response(standard_res.clone()) {
        Ok(r) => r,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let response_body_text = serde_json::to_string(&final_res).unwrap_or_default();
    let account = res.account.unwrap_or_default();
    
    let mut usage = res.usage;
    record_proxy_audit(ApiUsageRecord {
        account_id: account.id.clone(),
        model: requested_model.clone(),
        protocol: protocol_label(&account.provider),
        method,
        path,
        status: res.status.as_u16(),
        duration_ms: usage.duration_ms,
        ttft_ms: Some(usage.duration_ms),
        prompt_tokens: usage.prompt_tokens as u32,
        completion_tokens: usage.completion_tokens as u32,
        reasoning_tokens: 0,
        cached_tokens: usage.cached_tokens as u32,
        cache_write_tokens: usage.cache_write_tokens as u32,
        total_cost: usage.cost,
        request_body: Some(inbound_body_text),
        response_body: Some(response_body_text.clone()),
        attempt_receipts: Some(serde_json::to_value(&res.receipts).unwrap_or(Value::Null)),
    });

    response_builder = response_builder.header("Content-Type", "application/json");
    response_builder
        .body(axum::body::Body::from(response_body_text))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub(crate) async fn handle_anthropic(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        method.as_str().to_string(),
        uri.path().to_string(),
        "Anthropic".to_string(),
        headers,
        &AnthropicConverter,
        body,
    )
    .await
}

pub(crate) async fn handle_openai_chat(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        method.as_str().to_string(),
        uri.path().to_string(),
        "OpenAI".to_string(),
        headers,
        &OpenAiConverter,
        body,
    )
    .await
}

pub(crate) async fn handle_openai_completions(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        method.as_str().to_string(),
        uri.path().to_string(),
        "OpenAI".to_string(),
        headers,
        &OpenAiConverter,
        body,
    )
    .await
}

pub(crate) async fn handle_openai_responses(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        method.as_str().to_string(),
        uri.path().to_string(),
        "OpenAI".to_string(),
        headers,
        &OpenAiResponsesConverter,
        body,
    )
    .await
}

pub(crate) async fn handle_gemini(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        method.as_str().to_string(),
        uri.path().to_string(),
        "Gemini".to_string(),
        headers,
        &GeminiConverter,
        body,
    )
    .await
}

fn build_source_info(account: &LLMAccount, model_name: &str) -> serde_json::Value {
    let health = ProviderHealthRuntime::global();
    let acc_key = account_health_key(account);
    let mod_key = model_health_key(account, model_name);

    let acc_snapshot = health.snapshot(&acc_key);
    let mod_snapshot = health.snapshot(&mod_key);

    let (circuit, failure_tier, primary_health) = match (&acc_snapshot, &mod_snapshot) {
        (Some(acc), _) if acc.circuit == crate::domains::model_gateway::health::ProviderCircuitState::Open => {
            (crate::domains::model_gateway::health::ProviderCircuitState::Open, "account", Some(acc.clone()))
        }
        (_, Some(m)) if m.circuit == crate::domains::model_gateway::health::ProviderCircuitState::Open => {
            (crate::domains::model_gateway::health::ProviderCircuitState::Open, "model", Some(m.clone()))
        }
        (Some(acc), _) if acc.circuit == crate::domains::model_gateway::health::ProviderCircuitState::HalfOpen => {
            (crate::domains::model_gateway::health::ProviderCircuitState::HalfOpen, "account", Some(acc.clone()))
        }
        (_, Some(m)) if m.circuit == crate::domains::model_gateway::health::ProviderCircuitState::HalfOpen => {
            (crate::domains::model_gateway::health::ProviderCircuitState::HalfOpen, "model", Some(m.clone()))
        }
        (_, Some(m)) => {
            (crate::domains::model_gateway::health::ProviderCircuitState::Closed, "none", Some(m.clone()))
        }
        (Some(acc), None) => {
            (crate::domains::model_gateway::health::ProviderCircuitState::Closed, "none", Some(acc.clone()))
        }
        (None, None) => {
            (crate::domains::model_gateway::health::ProviderCircuitState::Closed, "none", None)
        }
    };

    serde_json::json!({
        "accountId": account.id,
        "accountName": account.name,
        "provider": account.provider,
        "tags": account.tags,
        "priority": account.priority,
        "weight": account.weight,
        "circuit": circuit,
        "failureTier": failure_tier,
        "health": primary_health,
        "accountHealth": acc_snapshot,
        "modelHealth": mod_snapshot,
    })
}

pub(crate) async fn handle_unified_models() -> Result<Response, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    let mut unique_models: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();

    // Add models and aliases from all accounts
    for account in settings.accounts.iter() {
        if !account.enabled {
            continue;
        }

        // Add actual models
        for model in &account.models {
            let source_info = build_source_info(account, model);
            if let Some(existing) = unique_models.get_mut(model) {
                if let Some(sources) = existing.get_mut("sources").and_then(|v| v.as_array_mut()) {
                    sources.push(source_info.clone());
                }
            } else {
                let mut model_obj = account
                    .models_info
                    .as_ref()
                    .and_then(|info| info.get(model))
                    .cloned()
                    .unwrap_or_else(|| {
                        serde_json::json!({
                            "id": model,
                            "object": "model",
                            "created": 1686935002,
                            "owned_by": "hub-rs"
                        })
                    });

                // Ensure ID is correct
                model_obj["id"] = serde_json::json!(model);
                if model_obj.get("object").is_none() {
                    model_obj["object"] = serde_json::json!("model");
                }
                model_obj["sources"] = serde_json::json!([source_info.clone()]);

                unique_models.insert(model.clone(), model_obj);
            }
        }

        // Add aliases (mapping)
        for (alias, target) in &account.model_mapping {
            let source_info = build_source_info(account, target);
            if let Some(existing) = unique_models.get_mut(alias) {
                if let Some(sources) = existing.get_mut("sources").and_then(|v| v.as_array_mut()) {
                    sources.push(source_info.clone());
                }
            } else {
                let mut model_obj = account
                    .models_info
                    .as_ref()
                    .and_then(|info| info.get(target))
                    .cloned()
                    .unwrap_or_else(|| {
                        serde_json::json!({
                            "id": target,
                            "object": "model",
                            "created": 1686935002,
                            "owned_by": "hub-rs"
                        })
                    });

                // Use alias as the ID
                model_obj["id"] = serde_json::json!(alias);
                if model_obj.get("object").is_none() {
                    model_obj["object"] = serde_json::json!("model");
                }
                model_obj["sources"] = serde_json::json!([source_info.clone()]);

                unique_models.insert(alias.clone(), model_obj);
            }
        }
    }

    // Fallback if no models are configured
    if unique_models.is_empty() {
        unique_models.insert(
            "gpt-4o".to_string(),
            serde_json::json!({
                "id": "gpt-4o",
                "object": "model",
                "created": 1686935002,
                "owned_by": "hub-rs",
                "sources": []
            }),
        );
    }

    // Inject experimental / hardcoded Gemini models (Antigravity-Manager compatibility)
    let hardcoded_models = vec![
        "gemini-3.1-pro-high",
        "gemini-3.1-pro-low",
        "gemini-3.0-pro",
        "gemini-3.0-flash",
        "gemini-2.5-pro",
        "gemini-2.5-flash",
        "gemini-2.0-flash-exp",
    ];

    for m in hardcoded_models {
        if !unique_models.contains_key(m) {
            unique_models.insert(
                m.to_string(),
                serde_json::json!({
                    "id": m,
                    "object": "model",
                    "created": 1686935002,
                    "owned_by": "google",
                    "sources": []
                }),
            );
        }
    }

    let mut data: Vec<serde_json::Value> = unique_models.into_values().collect();
    data.sort_by(|left, right| {
        left.get("id")
            .and_then(Value::as_str)
            .cmp(&right.get("id").and_then(Value::as_str))
    });

    let res = serde_json::json!({
        "object": "list",
        "data": data
    });

    Ok(Json(res).into_response())
}

pub(crate) async fn load_gemini_native_project(
    client: &reqwest::Client,
    access_token: &str,
) -> Option<String> {
    for base_url in [
        "https://daily-cloudcode-pa.googleapis.com",
        "https://daily-cloudcode-pa.sandbox.googleapis.com",
    ] {
        let response = client
            .post(format!("{base_url}/v1internal:loadCodeAssist"))
            .bearer_auth(access_token)
            .header("User-Agent", "vscode/1.X.X (Antigravity/4.3.0)")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "metadata": { "ideType": "ANTIGRAVITY" }
            }))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        let Ok(response) = response else {
            tracing::warn!(
                base_url,
                "failed to request gemini_native Cloud AI companion project"
            );
            continue;
        };
        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            tracing::warn!(
                base_url,
                status = status.as_u16(),
                body = %body_text,
                "failed to load gemini_native Cloud AI companion project"
            );
            continue;
        }

        let body = serde_json::from_str::<Value>(&body_text).ok()?;
        if let Some(project) = body
            .get("cloudaicompanionProject")
            .and_then(|value| {
                value.as_str().map(ToString::to_string).or_else(|| {
                    value
                        .get("id")
                        .or_else(|| value.get("projectId"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                })
            })
            .filter(|project| !project.is_empty())
        {
            return Some(project);
        }
    }

    None
}


