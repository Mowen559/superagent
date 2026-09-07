use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use serde_json::Value;

use crate::{
    config::llm_settings,
    services::{
        auth_validator,
        brain::model_route_runtime::{
            provider_health_key, ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome,
            ProviderAttemptReceipt,
        },
        brain::provider_health_runtime::{ProviderAdmission, ProviderHealthRuntime},
        protocol_converter::{
            anthropic::AnthropicConverter, gemini::GeminiConverter, openai::OpenAiConverter,
            openai_responses::OpenAiResponsesConverter, upstream::adapter_for_provider,
            ProtocolConverter,
        },
        usage_tracker::ApiUsageRecord,
    },
    AppState,
};

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};


use super::*;

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
    let mut settings = llm_settings::read_llm_settings();

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
    // Convert once at the inbound Adapter Seam before route eligibility is decided.
    let standard_body = converter
        .request_to_standard(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let requirements = ModelRequirements::from_canonical_body(&standard_body);
    let route_runtime = ModelRouteRuntime::default();
    let route = route_runtime.plan_with_requirements(
        &settings,
        &requested_model,
        &requested_tags,
        requirements,
    );
    let candidate_count = route.candidates.len();
    let health = ProviderHealthRuntime::global();

    let mut last_error = (
        StatusCode::SERVICE_UNAVAILABLE,
        "No accounts available to handle request".to_string(),
    );

    let mut receipts = Vec::new();

    for (candidate_index, candidate) in route.candidates.into_iter().enumerate() {
        let attempt = candidate_index + 1;
        let attempt_started = Instant::now();
        let mut account = candidate.account.clone();
        tracing::info!(
            account_id = %account.id,
            provider = %account.provider,
            model = %candidate.resolved_model,
            attempt,
            "executing proxy Provider Route candidate"
        );
        let mut attempt_settings = candidate.attempt_settings(&settings);

        // Support using local Antigravity Manager OAuth token via OS Keyring
        if attempt_settings.provider == "gemini_native" {
            if let Some(token) = crate::services::keyring_auth::get_or_refresh_credential(
                &state.http_client,
                "gemini:antigravity",
                "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com",
                Some("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"),
            )
            .await
            {
                attempt_settings.api_key = token;
            }
        } else if attempt_settings.provider == "openai"
            || attempt_settings.provider == "openai_native"
        {
            if let Some(token) = crate::services::keyring_auth::get_or_refresh_credential(
                &state.http_client,
                "openai:antigravity",
                "app_EMoamEEZ73f0CkXaXp7hrann",
                None,
            )
            .await
            {
                attempt_settings.api_key = token;
            }
        } else if attempt_settings.provider == "gemini" {
            if let Some(token) = crate::services::keyring_auth::get_or_refresh_credential(
                &state.http_client,
                "gemini:oauth",
                "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com",
                None,
            )
            .await
            {
                attempt_settings.api_key = token;
            }
        }

        // Get token through AuthRuntime (centralized refresh)
        match crate::services::cli_auth::AuthRuntime::global().get_valid_token(&mut account, &state.http_client).await {
            Ok(token) => {
                attempt_settings.api_key = token;
            }
            Err(e) => {
                tracing::warn!("AuthRuntime failed to get valid token: {}", e);
                last_error = (StatusCode::UNAUTHORIZED, format!("Auth error: {}", e));
                let class = route_runtime.classify_failure(&format!("auth failed: {e}"));
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    elapsed_ms(attempt_started),
                    class.into(),
                    Some(401),
                ));
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    continue;
                }
                return Err(last_error);
            }
        }

        // Check for OAuth Token in Authorization header (User overriding)
        let mut has_valid_oauth = false;
        if let Some(auth_val) = headers.get("Authorization").and_then(|v| v.to_str().ok()) {
            if auth_val.starts_with("Bearer ") {
                let token = &auth_val[7..];
                if !auth_validator::is_api_key(token) {
                    match auth_validator::validate_oauth_token(
                        &state.http_client,
                        token,
                        &state.runtime_config.sso_api_url,
                    )
                    .await
                    {
                        Ok(real_api_key) => {
                            attempt_settings.api_key = real_api_key;
                            attempt_settings.enabled = true;
                            has_valid_oauth = true;
                        }
                        Err(e) => {
                            // OAuth failure is fatal
                            return Err((StatusCode::UNAUTHORIZED, e));
                        }
                    }
                }
            }
        }

        if !has_valid_oauth && !llm_settings::has_usable_settings(&attempt_settings) {
            last_error = (
                StatusCode::SERVICE_UNAVAILABLE,
                "LLM provider is not configured and no valid OAuth token was provided".to_string(),
            );
            continue;
        }

        if attempt_settings.provider == "gemini_native"
            && attempt_settings
                .project_id
                .as_deref()
                .is_none_or(|project| project.is_empty() || project == "project-id")
        {
            if let Some(project_id) =
                load_gemini_native_project(&state.http_client, &attempt_settings.api_key).await
            {
                tracing::info!(
                    account_id = %account.id,
                    project_id = %project_id,
                    "resolved gemini_native Cloud AI companion project"
                );
                attempt_settings.project_id = Some(project_id.clone());
                if let Some(global_account) = settings
                    .accounts
                    .iter_mut()
                    .find(|item| item.id == account.id)
                {
                    global_account.project_id = Some(project_id);
                    llm_settings::write_llm_settings(
                        serde_json::to_value(&settings).unwrap_or_default(),
                    );
                }
            }
        }

        // Clone standard body and inject model for this specific account
        let mut attempt_body = standard_body.clone();
        if !attempt_settings.model.is_empty() {
            if let Some(obj) = attempt_body.as_object_mut() {
                obj.insert(
                    "model".to_string(),
                    Value::String(attempt_settings.model.clone()),
                );
            }
        }

        // 3. Handle Streaming vs Non-streaming
        let is_stream = attempt_body
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Prepare HTTP request to the upstream provider
        let upstream_adapter = adapter_for_provider(&attempt_settings);

        let prep_res = upstream_adapter.prepare_request(&attempt_settings, attempt_body);
        let (target_url, req_headers, upstream_body) = match prep_res {
            Ok(res) => res,
            Err(e) => {
                last_error = (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                let class = route_runtime.classify_failure(&e.to_string());
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    elapsed_ms(attempt_started),
                    class.into(),
                    Some(500),
                ));
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    continue;
                }
                return Err(last_error);
            }
        };

        let health_key = candidate.health_key();
        if let ProviderAdmission::Rejected { retry_after_ms } = health.begin_attempt(&health_key) {
            last_error = (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Provider Circuit open; retry after {retry_after_ms} ms"),
            );
            receipts.push(ProviderAttemptReceipt::new(
                &candidate,
                attempt,
                elapsed_ms(attempt_started),
                ProviderAttemptOutcome::CircuitOpen,
                Some(503),
            ));
            if attempt < candidate_count {
                continue;
            }
            return Err(last_error);
        }

        let mut req_builder = state.http_client.post(&target_url).json(&upstream_body);
        for (k, v) in req_headers.iter() {
            if k.as_str().eq_ignore_ascii_case("content-type") {
                continue;
            }
            req_builder = req_builder.header(k, v);
        }

        if attempt_settings.provider != "gemini_native" {
            if let Some(project_id) = &attempt_settings.project_id {
                req_builder = req_builder.header("x-goog-user-project", project_id);
            }
        }

        // Send request
        let res = match req_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Upstream network error for {}: {}", account.name, e);
                last_error = (StatusCode::BAD_GATEWAY, e.to_string());
                let class = route_runtime.classify_failure(&format!("request failed: {e}"));
                health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    elapsed_ms(attempt_started),
                    class.into(),
                    Some(502),
                ));
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    continue;
                }
                return Err(last_error);
            }
        };

        let status = res.status();

        if !is_stream {
            let response_text = res.text().await.unwrap_or_default();
            if !status.is_success() {
                let class = route_runtime.classify_http_failure(status.as_u16(), &response_text);
                health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    elapsed_ms(attempt_started),
                    class.into(),
                    Some(status.as_u16()),
                ));
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    tracing::warn!(
                        "Account {} returned {}. Failing over.",
                        account.name,
                        status
                    );
                    last_error = (status, response_text);
                    continue;
                } else {
                    record_proxy_audit(ApiUsageRecord {
                        account_id: account.id.clone(),
                        model: requested_model.clone(),
                        protocol: protocol_label(&account.provider),
                        method: method.clone(),
                        path: path.clone(),
                        status: status.as_u16(),
                        duration_ms: elapsed_ms(attempt_started),
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        cached_tokens: 0,
                        cache_write_tokens: 0,
                        total_cost: 0.0,
                        request_body: Some(inbound_body_text.clone()),
                        response_body: Some(response_text.clone()),
                        attempt_receipts: Some(
                            serde_json::to_value(&receipts).unwrap_or(Value::Null),
                        ),
                    });
                    let res_body = serde_json::from_str::<Value>(&response_text)
                        .unwrap_or_else(|_| json_error_body(&response_text));
                    return Ok((status, Json(res_body)).into_response());
                }
            }

            let res_body: Value = match serde_json::from_str(&response_text) {
                Ok(body) => body,
                Err(error) => {
                    health.finish_attempt(
                        &health_key,
                        ProviderAttemptOutcome::Permanent,
                        elapsed_ms(attempt_started),
                    );
                    receipts.push(ProviderAttemptReceipt::new(
                        &candidate,
                        attempt,
                        elapsed_ms(attempt_started),
                        ProviderAttemptOutcome::Permanent,
                        Some(502),
                    ));
                    record_proxy_audit(ApiUsageRecord {
                        account_id: account.id.clone(),
                        model: requested_model.clone(),
                        protocol: protocol_label(&account.provider),
                        method: method.clone(),
                        path: path.clone(),
                        status: 502,
                        duration_ms: elapsed_ms(attempt_started),
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        cached_tokens: 0,
                        cache_write_tokens: 0,
                        total_cost: 0.0,
                        request_body: Some(inbound_body_text.clone()),
                        response_body: Some(response_text.clone()),
                        attempt_receipts: Some(
                            serde_json::to_value(&receipts).unwrap_or(Value::Null),
                        ),
                    });
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        format!("Provider returned invalid JSON: {error}"),
                    ));
                }
            };

            let standard_res = match upstream_adapter.response_to_standard(res_body) {
                Ok(response) => response,
                Err(error) => {
                    health.finish_attempt(
                        &health_key,
                        ProviderAttemptOutcome::Permanent,
                        elapsed_ms(attempt_started),
                    );
                    receipts.push(ProviderAttemptReceipt::new(
                        &candidate,
                        attempt,
                        elapsed_ms(attempt_started),
                        ProviderAttemptOutcome::Permanent,
                        Some(502),
                    ));
                    record_proxy_audit(ApiUsageRecord {
                        account_id: account.id.clone(),
                        model: requested_model.clone(),
                        protocol: protocol_label(&account.provider),
                        method: method.clone(),
                        path: path.clone(),
                        status: 502,
                        duration_ms: elapsed_ms(attempt_started),
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        cached_tokens: 0,
                        cache_write_tokens: 0,
                        total_cost: 0.0,
                        request_body: Some(inbound_body_text.clone()),
                        response_body: Some(error.to_string()),
                        attempt_receipts: Some(
                            serde_json::to_value(&receipts).unwrap_or(Value::Null),
                        ),
                    });
                    return Err((StatusCode::BAD_GATEWAY, error.to_string()));
                }
            };
            health.finish_attempt(
                &health_key,
                ProviderAttemptOutcome::Succeeded,
                elapsed_ms(attempt_started),
            );

            receipts.push(ProviderAttemptReceipt::new(
                &candidate,
                attempt,
                elapsed_ms(attempt_started),
                ProviderAttemptOutcome::Succeeded,
                Some(status.as_u16()),
            ));

            let usage = usage_from_value(standard_res.get("usage"));
            let converted_res = converter
                .standard_to_response(standard_res)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let cost = crate::services::cost_calculator::CostCalculator::calculate_cost(
                &requested_model,
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.cached_tokens,
                usage.cache_write_tokens,
            );
            record_proxy_audit(ApiUsageRecord {
                account_id: account.id.clone(),
                model: requested_model.clone(),
                protocol: protocol_label(&account.provider),
                method: method.clone(),
                path: path.clone(),
                status: status.as_u16(),
                duration_ms: elapsed_ms(attempt_started),
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                cached_tokens: usage.cached_tokens,
                cache_write_tokens: usage.cache_write_tokens,
                total_cost: cost,
                request_body: Some(inbound_body_text.clone()),
                response_body: Some(pretty_json(&converted_res)),
                attempt_receipts: Some(serde_json::to_value(&receipts).unwrap_or(Value::Null)),
            });

            return Ok(Json(converted_res).into_response());
        } else {
            if !status.is_success() {
                let err_text = res.text().await.unwrap_or_default();
                let class = route_runtime.classify_http_failure(status.as_u16(), &err_text);
                health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    elapsed_ms(attempt_started),
                    class.into(),
                    Some(status.as_u16()),
                ));
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    tracing::warn!(
                        "Account {} returned {}. Failing over.",
                        account.name,
                        status
                    );
                    last_error = (status, err_text);
                    continue;
                } else {
                    record_proxy_audit(ApiUsageRecord {
                        account_id: account.id.clone(),
                        model: requested_model.clone(),
                        protocol: protocol_label(&account.provider),
                        method: method.clone(),
                        path: path.clone(),
                        status: status.as_u16(),
                        duration_ms: elapsed_ms(attempt_started),
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        cached_tokens: 0,
                        cache_write_tokens: 0,
                        total_cost: 0.0,
                        request_body: Some(inbound_body_text.clone()),
                        response_body: Some(err_text.clone()),
                        attempt_receipts: Some(
                            serde_json::to_value(&receipts).unwrap_or(Value::Null),
                        ),
                    });
                    let mut response_builder = axum::response::Response::builder().status(status);
                    response_builder = response_builder.header("Content-Type", "application/json");
                    return response_builder
                        .body(axum::body::Body::from(err_text))
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
                }
            }

            receipts.push(ProviderAttemptReceipt::new(
                &candidate,
                attempt,
                elapsed_ms(attempt_started),
                ProviderAttemptOutcome::Succeeded,
                Some(status.as_u16()),
            ));

            let mut response_builder = axum::response::Response::builder().status(status);

            if let Some(content_type) = res.headers().get("Content-Type") {
                response_builder = response_builder.header("Content-Type", content_type);
            }

            let raw_stream = res.bytes_stream();
            let io_stream = futures::stream::StreamExt::map(raw_stream, |r| {
                r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            });
            let standard_stream = upstream_adapter.response_stream_to_standard(Box::pin(io_stream));
            let standard_stream =
                track_provider_stream(standard_stream, health_key.clone(), attempt_started);

            let stream_audit = Arc::new(Mutex::new(StreamAuditState::new(
                account.id.clone(),
                requested_model.clone(),
                protocol_label(&account.provider),
                method.clone(),
                path.clone(),
                status.as_u16(),
                attempt_started,
                inbound_body_text.clone(),
                receipts,
            )));
            let intercepted_stream =
                futures::stream::StreamExt::map(standard_stream, move |chunk_res| {
                    if let Ok(chunk) = &chunk_res {
                        if let Ok(text) = std::str::from_utf8(chunk) {
                            if let Ok(mut audit) = stream_audit.lock() {
                                audit.push_response(text);
                            }
                            for line in text.lines() {
                                if line.starts_with("data: ") && line != "data: [DONE]" {
                                    if let Ok(json) =
                                        serde_json::from_str::<serde_json::Value>(&line[6..])
                                    {
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

            let mapped_stream = converter.standard_to_response_stream(Box::pin(intercepted_stream));

            let body = axum::body::Body::from_stream(mapped_stream);

            return response_builder.body(body).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to build stream response: {}", e),
                )
            });
        }
    }

    // If we exhaust the loop, return the last error we got
    Err(last_error)
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

pub(crate) async fn handle_unified_models() -> Result<Response, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    let mut unique_models: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();

    // Add models and aliases from all accounts
    for account in settings.accounts.iter() {
        if !account.enabled {
            continue;
        }

        let source_info = serde_json::json!({
            "accountId": account.id,
            "accountName": account.name,
            "provider": account.provider,
            "tags": account.tags,
            "priority": account.priority,
            "weight": account.weight,
            "health": ProviderHealthRuntime::global().snapshot(&provider_health_key(account))
        });

        // Add actual models
        for model in &account.models {
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


