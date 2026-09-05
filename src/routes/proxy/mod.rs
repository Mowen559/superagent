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
        brain::{
            model_route_runtime::{
                provider_health_key, ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome,
            },
            provider_health_runtime::{ProviderAdmission, ProviderHealthRuntime},
        },
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

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/messages", post(handle_anthropic))
        .route("/v1/chat/completions", post(handle_openai_chat))
        .route("/v1/completions", post(handle_openai_completions))
        .route("/v1/responses", post(handle_openai_responses))
        .route("/v1/models", get(handle_unified_models))
        .route("/v1beta/models/{*path}", post(handle_gemini))
}

async fn handle_request(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    converter: &dyn ProtocolConverter,
    audit_path: &'static str,
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
        } else if attempt_settings.provider == "openai" {
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

        // Token refresh logic
        let mut needs_save = false;
        if let (Some(expires_at), Some(refresh_token)) =
            (account.expires_at, &account.refresh_token)
        {
            // If token expires within the next 5 minutes
            if chrono::Utc::now().timestamp() + 300 >= expires_at {
                tracing::info!("Token expired or expiring soon, refreshing...");

                if account.provider == "github_copilot" {
                    // Copilot uses a specific internal token exchange with the gho_ token
                    let req = state
                        .http_client
                        .get("https://api.github.com/copilot_internal/v2/token")
                        .header("Authorization", format!("token {}", refresh_token))
                        .header("Accept", "application/json")
                        .header("User-Agent", "GitHubCopilotChat/0.11.1");

                    if let Ok(res) = req.send().await {
                        if let Ok(json_res) = res.json::<serde_json::Value>().await {
                            if let Some(token) = json_res.get("token").and_then(|v| v.as_str()) {
                                account.api_key = token.to_string();
                                if let Some(expires_in) =
                                    json_res.get("expires_at").and_then(|v| v.as_i64())
                                {
                                    account.expires_at = Some(expires_in);
                                }
                                attempt_settings.api_key = account.api_key.clone();
                                needs_save = true;
                            }
                        }
                    }
                } else {
                    let token_url = if account.provider == "openai" {
                        "https://auth.openai.com/oauth/token"
                    } else if account.provider == "gemini" || account.provider == "gemini_native" {
                        "https://oauth2.googleapis.com/token"
                    } else {
                        ""
                    };

                    if !token_url.is_empty() {
                        let client_id = if account.provider == "openai" {
                            "app_EMoamEEZ73f0CkXaXp7hrann"
                        } else if account.provider == "gemini_native" {
                            "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
                        } else {
                            "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com"
                        };

                        let client_secret = if account.provider == "gemini" {
                            Some("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")
                        } else if account.provider == "gemini_native" {
                            Some("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")
                        } else {
                            None
                        };

                        let req = state
                            .http_client
                            .post(token_url)
                            .header("Content-Type", "application/x-www-form-urlencoded");

                        let mut body_str = format!(
                            "grant_type=refresh_token&refresh_token={}&client_id={}",
                            refresh_token, client_id
                        );
                        if let Some(secret) = client_secret {
                            body_str.push_str(&format!("&client_secret={}", secret));
                        }

                        if let Ok(res) = req.body(body_str).send().await {
                            if let Ok(res_json) = res.json::<serde_json::Value>().await {
                                if let Some(access_token) =
                                    res_json.get("access_token").and_then(|v| v.as_str())
                                {
                                    account.api_key = access_token.to_string();

                                    if let Some(new_refresh) =
                                        res_json.get("refresh_token").and_then(|v| v.as_str())
                                    {
                                        account.refresh_token = Some(new_refresh.to_string());
                                    }

                                    if let Some(expires_in) =
                                        res_json.get("expires_in").and_then(|v| v.as_i64())
                                    {
                                        account.expires_at =
                                            Some(chrono::Utc::now().timestamp() + expires_in);
                                    }

                                    attempt_settings.api_key = account.api_key.clone();
                                    needs_save = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        if needs_save {
            // Update the global settings file with new token
            if let Some(global_account) = settings.accounts.iter_mut().find(|a| a.id == account.id)
            {
                global_account.api_key = account.api_key.clone();
                global_account.refresh_token = account.refresh_token.clone();
                global_account.expires_at = account.expires_at;
                llm_settings::write_llm_settings(
                    serde_json::to_value(&settings).unwrap_or_default(),
                );
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
        let upstream_adapter = adapter_for_provider(&attempt_settings.provider);

        let prep_res = upstream_adapter.prepare_request(&attempt_settings, attempt_body);
        let (target_url, req_headers, upstream_body) = match prep_res {
            Ok(res) => res,
            Err(e) => {
                last_error = (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                let class = route_runtime.classify_failure(&e.to_string());
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    continue;
                }
                return Err(last_error);
            }
        };

        let health_key = candidate.health_key();
        if let ProviderAdmission::Rejected { retry_after_ms } = health.begin_attempt(&health_key) {
            tracing::warn!(
                account_id = %account.id,
                provider = %account.provider,
                retry_after_ms,
                "Provider Circuit is open; skipping proxy candidate"
            );
            last_error = (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Provider Circuit open; retry after {retry_after_ms} ms"),
            );
            if attempt < candidate_count {
                continue;
            }
            return Err(last_error);
        }

        let mut req_builder = state.http_client.post(&target_url).json(&upstream_body);
        for (k, v) in req_headers.iter() {
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
                record_proxy_audit(ApiUsageRecord {
                    account_id: account.id.clone(),
                    model: requested_model.clone(),
                    protocol: protocol_label(&account.provider),
                    method: "POST".to_string(),
                    path: audit_path.to_string(),
                    status: status.as_u16(),
                    duration_ms: elapsed_ms(attempt_started),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                    total_cost: 0.0,
                    request_body: Some(inbound_body_text.clone()),
                    response_body: Some(response_text.clone()),
                });
                let class = route_runtime.classify_http_failure(status.as_u16(), &response_text);
                health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    tracing::warn!(
                        "Account {} returned {}. Failing over.",
                        account.name,
                        status
                    );
                    last_error = (status, response_text);
                    continue;
                } else {
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
                    return Err((StatusCode::BAD_GATEWAY, error.to_string()));
                }
            };
            health.finish_attempt(
                &health_key,
                ProviderAttemptOutcome::Succeeded,
                elapsed_ms(attempt_started),
            );

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
                method: "POST".to_string(),
                path: audit_path.to_string(),
                status: status.as_u16(),
                duration_ms: elapsed_ms(attempt_started),
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                cached_tokens: usage.cached_tokens,
                cache_write_tokens: usage.cache_write_tokens,
                total_cost: cost,
                request_body: Some(inbound_body_text.clone()),
                response_body: Some(pretty_json(&converted_res)),
            });

            return Ok(Json(converted_res).into_response());
        } else {
            if !status.is_success() {
                let err_text = res.text().await.unwrap_or_default();
                record_proxy_audit(ApiUsageRecord {
                    account_id: account.id.clone(),
                    model: requested_model.clone(),
                    protocol: protocol_label(&account.provider),
                    method: "POST".to_string(),
                    path: audit_path.to_string(),
                    status: status.as_u16(),
                    duration_ms: elapsed_ms(attempt_started),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                    total_cost: 0.0,
                    request_body: Some(inbound_body_text.clone()),
                    response_body: Some(err_text.clone()),
                });
                let class = route_runtime.classify_http_failure(status.as_u16(), &err_text);
                health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    tracing::warn!(
                        "Account {} returned {}. Failing over.",
                        account.name,
                        status
                    );
                    last_error = (status, err_text);
                    continue;
                } else {
                    let mut response_builder = axum::response::Response::builder().status(status);
                    response_builder = response_builder.header("Content-Type", "application/json");
                    return response_builder
                        .body(axum::body::Body::from(err_text))
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
                }
            }

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

            // Intercept standard_stream for SSE usage extraction
            let stream_audit = Arc::new(Mutex::new(StreamAuditState::new(
                account.id.clone(),
                requested_model.clone(),
                protocol_label(&account.provider),
                audit_path.to_string(),
                status.as_u16(),
                attempt_started,
                inbound_body_text.clone(),
            )));
            let intercepted_stream =
                futures::stream::StreamExt::map(standard_stream, move |chunk_res| {
                    if let Ok(chunk) = &chunk_res {
                        if let Ok(text) = std::str::from_utf8(chunk) {
                            if let Ok(mut audit) = stream_audit.lock() {
                                audit.push_response(text);
                            }
                            // Quick and dirty SSE parse for usage
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

async fn handle_anthropic(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        headers,
        &AnthropicConverter,
        "/v1/messages",
        body,
    )
    .await
}

async fn handle_openai_chat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        headers,
        &OpenAiConverter,
        "/v1/chat/completions",
        body,
    )
    .await
}

async fn handle_openai_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        headers,
        &OpenAiConverter,
        "/v1/completions",
        body,
    )
    .await
}

async fn handle_openai_responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        headers,
        &OpenAiResponsesConverter,
        "/v1/responses",
        body,
    )
    .await
}

async fn handle_gemini(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(
        State(state),
        headers,
        &GeminiConverter,
        "/v1beta/models/*",
        body,
    )
    .await
}

async fn handle_unified_models() -> Result<Response, (StatusCode, String)> {
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

fn track_provider_stream(
    mut stream: std::pin::Pin<
        Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
    >,
    health_key: String,
    started: Instant,
) -> std::pin::Pin<
    Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
> {
    Box::pin(async_stream::stream! {
        let mut guard = ProviderStreamHealthGuard::new(health_key, started);
        while let Some(item) = futures::StreamExt::next(&mut stream).await {
            match &item {
                Ok(chunk) => {
                    if let Some(outcome) = canonical_stream_terminal_outcome(chunk) {
                        guard.finish(outcome);
                    }
                }
                Err(_) => guard.finish(ProviderAttemptOutcome::Transient),
            }
            let failed = item.is_err();
            yield item;
            if failed {
                break;
            }
        }
    })
}

struct ProviderStreamHealthGuard {
    health_key: String,
    started: Instant,
    finished: bool,
}

impl ProviderStreamHealthGuard {
    fn new(health_key: String, started: Instant) -> Self {
        Self {
            health_key,
            started,
            finished: false,
        }
    }

    fn finish(&mut self, outcome: ProviderAttemptOutcome) {
        if self.finished {
            return;
        }
        ProviderHealthRuntime::global().finish_attempt(
            &self.health_key,
            outcome,
            elapsed_ms(self.started),
        );
        self.finished = true;
    }
}

impl Drop for ProviderStreamHealthGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(ProviderAttemptOutcome::Transient);
        }
    }
}

fn canonical_stream_terminal_outcome(chunk: &bytes::Bytes) -> Option<ProviderAttemptOutcome> {
    let text = std::str::from_utf8(chunk).ok()?;
    for line in text.lines() {
        let Some(data) = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
        else {
            continue;
        };
        if data.trim() == "[DONE]" {
            return Some(ProviderAttemptOutcome::Succeeded);
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if value.get("error").is_some() {
            return Some(ProviderAttemptOutcome::Transient);
        }
        let finished = value
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
            return Some(ProviderAttemptOutcome::Succeeded);
        }
    }
    None
}

fn json_error_body(raw: &str) -> Value {
    let mut end = raw.len().min(512);
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::json!({"error": {"message": &raw[..end]}})
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, Default, Clone, Copy)]
struct ExtractedUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    cached_tokens: u32,
    cache_write_tokens: u32,
}

fn usage_from_value(usage: Option<&Value>) -> ExtractedUsage {
    let Some(usage) = usage else {
        return ExtractedUsage::default();
    };
    ExtractedUsage {
        prompt_tokens: token_field(usage, &["prompt_tokens", "input_tokens"]),
        completion_tokens: token_field(usage, &["completion_tokens", "output_tokens"]),
        cached_tokens: token_field(
            usage,
            &[
                "cached_tokens",
                "cache_read_input_tokens",
                "cached_content_token_count",
                "prompt_tokens_details.cached_tokens",
                "input_tokens_details.cached_tokens",
                "input_token_details.cache_read",
            ],
        ),
        cache_write_tokens: token_field(
            usage,
            &[
                "cache_write_tokens",
                "cache_creation_input_tokens",
                "input_token_details.cache_creation",
            ],
        ),
    }
}

fn token_field(value: &Value, paths: &[&str]) -> u32 {
    paths
        .iter()
        .find_map(|path| {
            path.split('.')
                .try_fold(value, |current, segment| current.get(segment))
                .and_then(Value::as_u64)
        })
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32
}

fn protocol_label(provider: &str) -> String {
    match provider {
        "anthropic" | "claude" => "ANTHROPIC",
        "gemini" | "gemini_native" => "GEMINI",
        _ => "OpenAI",
    }
    .to_string()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn record_proxy_audit(record: ApiUsageRecord) {
    crate::services::usage_tracker::record_api_call(record);
}

struct StreamAuditState {
    account_id: String,
    model: String,
    protocol: String,
    path: String,
    status: u16,
    started: Instant,
    request_body: String,
    response_body: String,
    usage: ExtractedUsage,
    recorded: bool,
}

impl StreamAuditState {
    fn new(
        account_id: String,
        model: String,
        protocol: String,
        path: String,
        status: u16,
        started: Instant,
        request_body: String,
    ) -> Self {
        Self {
            account_id,
            model,
            protocol,
            path,
            status,
            started,
            request_body,
            response_body: String::new(),
            usage: ExtractedUsage::default(),
            recorded: false,
        }
    }

    fn push_response(&mut self, text: &str) {
        const MAX_RESPONSE_CHARS: usize = 200_000;
        let remaining = MAX_RESPONSE_CHARS.saturating_sub(self.response_body.chars().count());
        if remaining > 0 {
            self.response_body.extend(text.chars().take(remaining));
        }
    }

    fn apply_usage(&mut self, usage: ExtractedUsage) {
        self.usage = usage;
    }

    fn record_once(&mut self) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        let cost = crate::services::cost_calculator::CostCalculator::calculate_cost(
            &self.model,
            self.usage.prompt_tokens,
            self.usage.completion_tokens,
            self.usage.cached_tokens,
            self.usage.cache_write_tokens,
        );
        record_proxy_audit(ApiUsageRecord {
            account_id: self.account_id.clone(),
            model: self.model.clone(),
            protocol: self.protocol.clone(),
            method: "POST".to_string(),
            path: self.path.clone(),
            status: self.status,
            duration_ms: elapsed_ms(self.started),
            prompt_tokens: self.usage.prompt_tokens,
            completion_tokens: self.usage.completion_tokens,
            cached_tokens: self.usage.cached_tokens,
            cache_write_tokens: self.usage.cache_write_tokens,
            total_cost: cost,
            request_body: Some(self.request_body.clone()),
            response_body: Some(self.response_body.clone()),
        });
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;

    #[test]
    fn detects_stream_success_and_error_terminals() {
        assert_eq!(
            canonical_stream_terminal_outcome(&bytes::Bytes::from_static(b"data: [DONE]\n\n")),
            Some(ProviderAttemptOutcome::Succeeded)
        );
        assert_eq!(
            canonical_stream_terminal_outcome(&bytes::Bytes::from_static(
                b"data: {\"error\":{\"message\":\"failed\"}}\n\n"
            )),
            Some(ProviderAttemptOutcome::Transient)
        );
    }

    #[tokio::test]
    async fn stream_tracker_releases_health_on_success_and_incomplete_end() {
        let health = ProviderHealthRuntime::global();
        let success_key = format!("proxy-stream-success-{}", uuid::Uuid::new_v4());
        health.reset(&success_key);
        assert_eq!(
            health.begin_attempt(&success_key),
            ProviderAdmission::Allowed
        );
        let success_stream =
            futures::stream::iter(vec![Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"))]);
        track_provider_stream(
            Box::pin(success_stream),
            success_key.clone(),
            Instant::now(),
        )
        .collect::<Vec<_>>()
        .await;
        let snapshot = health.snapshot(&success_key).unwrap();
        assert_eq!(snapshot.total_successes, 1);
        health.reset(&success_key);

        let incomplete_key = format!("proxy-stream-incomplete-{}", uuid::Uuid::new_v4());
        health.reset(&incomplete_key);
        assert_eq!(
            health.begin_attempt(&incomplete_key),
            ProviderAdmission::Allowed
        );
        let incomplete_stream = futures::stream::iter(vec![Ok(bytes::Bytes::from_static(
            b"data: {\"choices\":[]}\n\n",
        ))]);
        track_provider_stream(
            Box::pin(incomplete_stream),
            incomplete_key.clone(),
            Instant::now(),
        )
        .collect::<Vec<_>>()
        .await;
        let snapshot = health.snapshot(&incomplete_key).unwrap();
        assert_eq!(snapshot.total_recoverable_failures, 1);
        health.reset(&incomplete_key);
    }
}
