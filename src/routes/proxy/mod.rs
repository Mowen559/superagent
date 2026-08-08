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
        protocol_converter::{
            anthropic::AnthropicConverter, gemini::GeminiConverter, openai::OpenAiConverter,
            openai_responses::OpenAiResponsesConverter, upstream::{OpenAiUpstreamAdapter, UpstreamAdapter}, gemini_internal_upstream::GeminiInternalUpstreamAdapter, ProtocolConverter,
        },
    },
    AppState,
};

use std::sync::Arc;

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
    body: Value,
) -> Result<Response, (StatusCode, String)> {
    let mut settings = llm_settings::read_llm_settings();
    
    // Auto-routing logic based on requested model
    let mut requested_model = String::new();
    if let Some(model_val) = body.get("model").and_then(|v| v.as_str()) {
        requested_model = model_val.to_string();
    }
    
    // 1. Determine attempt order
    let mut attempt_accounts = Vec::new();
    
    if !requested_model.is_empty() {
        let matching_accounts: Vec<_> = settings.accounts.iter().filter(|a| {
            a.enabled && (a.models.contains(&requested_model) || a.model_mapping.contains_key(&requested_model))
        }).cloned().collect();
        
        if !matching_accounts.is_empty() {
            if settings.routing_strategy == "round_robin" {
                let req_count = state.request_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let start_idx = req_count % matching_accounts.len();
                for i in 0..matching_accounts.len() {
                    let idx = (start_idx + i) % matching_accounts.len();
                    attempt_accounts.push(matching_accounts[idx].clone());
                }
            } else {
                // Failover (Primary/Standby) - strictly follow configured order
                attempt_accounts = matching_accounts;
            }
        }
    }
    
    // Fallback if no matching accounts found (use default active account)
    if attempt_accounts.is_empty() {
        if let Some(active_id) = &settings.active_account_id {
            if let Some(account) = settings.accounts.iter().find(|a| a.id == *active_id && a.enabled) {
                attempt_accounts.push(account.clone());
            }
        }
    }
    
    // 2. Convert incoming request to internal standard format (OpenAI-compatible)
    let standard_body = converter
        .request_to_standard(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        
    let mut last_error = (StatusCode::SERVICE_UNAVAILABLE, "No accounts available to handle request".to_string());
    
    for mut account in attempt_accounts {
        tracing::info!("Attempting request with account: {} (Provider: {})", account.name, account.provider);
        let mut attempt_settings = settings.clone();
        attempt_settings.provider = account.provider.clone();
        attempt_settings.base_url = account.base_url.clone();
        attempt_settings.api_key = account.api_key.clone();
        attempt_settings.project_id = account.project_id.clone();
        
        if let Some(real_model) = account.model_mapping.get(&requested_model) {
            attempt_settings.model = real_model.clone();
        } else if !requested_model.is_empty() {
            attempt_settings.model = requested_model.clone();
        }
        
        // Support using local Antigravity Manager OAuth token via OS Keyring
        if attempt_settings.provider == "gemini_native" {
            if let Some(token) = crate::services::keyring_auth::get_or_refresh_credential(
                &state.http_client, 
                "gemini:antigravity",
                "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com",
                Some("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")
            ).await {
                attempt_settings.api_key = token;
            }
        } else if attempt_settings.provider == "gemini" {
            if let Some(token) = crate::services::keyring_auth::get_or_refresh_credential(
                &state.http_client, 
                "gemini:oauth",
                "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com",
                None
            ).await {
                attempt_settings.api_key = token;
            }
        }
        
        // Token refresh logic
        let mut needs_save = false;
        if let (Some(expires_at), Some(refresh_token)) = (account.expires_at, &account.refresh_token) {
            // If token expires within the next 5 minutes
            if chrono::Utc::now().timestamp() + 300 >= expires_at {
                tracing::info!("Token expired or expiring soon, refreshing...");
                
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
                    
                    let req = state.http_client.post(token_url)
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
                            if let Some(access_token) = res_json.get("access_token").and_then(|v| v.as_str()) {
                                account.api_key = access_token.to_string();
                                
                                if let Some(new_refresh) = res_json.get("refresh_token").and_then(|v| v.as_str()) {
                                    account.refresh_token = Some(new_refresh.to_string());
                                }
                                
                                if let Some(expires_in) = res_json.get("expires_in").and_then(|v| v.as_i64()) {
                                    account.expires_at = Some(chrono::Utc::now().timestamp() + expires_in);
                                }
                                
                                attempt_settings.api_key = account.api_key.clone();
                                needs_save = true;
                            }
                        }
                    }
                }
            }
        }
        
        if needs_save {
            // Update the global settings file with new token
            if let Some(global_account) = settings.accounts.iter_mut().find(|a| a.id == account.id) {
                global_account.api_key = account.api_key.clone();
                global_account.refresh_token = account.refresh_token.clone();
                global_account.expires_at = account.expires_at;
                llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
            }
        }
        
        // Check for OAuth Token in Authorization header (User overriding)
        let mut has_valid_oauth = false;
        if let Some(auth_val) = headers.get("Authorization").and_then(|v| v.to_str().ok()) {
            if auth_val.starts_with("Bearer ") {
                let token = &auth_val[7..];
                if !auth_validator::is_api_key(token) {
                    match auth_validator::validate_oauth_token(&state.http_client, token, &state.runtime_config.sso_api_url).await {
                        Ok(real_api_key) => {
                            attempt_settings.api_key = real_api_key;
                            attempt_settings.enabled = true;
                            has_valid_oauth = true;
                        },
                        Err(e) => {
                            // OAuth failure is fatal
                            return Err((StatusCode::UNAUTHORIZED, e));
                        }
                    }
                }
            }
        }
        
        if !has_valid_oauth && !llm_settings::has_usable_settings(&attempt_settings) {
            last_error = (StatusCode::SERVICE_UNAVAILABLE, "LLM provider is not configured and no valid OAuth token was provided".to_string());
            continue;
        }
        
        // Clone standard body and inject model for this specific account
        let mut attempt_body = standard_body.clone();
        if !attempt_settings.model.is_empty() {
            if let Some(obj) = attempt_body.as_object_mut() {
                obj.insert("model".to_string(), Value::String(attempt_settings.model.clone()));
            }
        }
        
        // Prepare HTTP request to the upstream provider
        let upstream_adapter: Box<dyn UpstreamAdapter> = if attempt_settings.provider == "gemini_native" {
            Box::new(GeminiInternalUpstreamAdapter)
        } else {
            Box::new(OpenAiUpstreamAdapter)
        };
        
        let prep_res = upstream_adapter.prepare_request(&attempt_settings, attempt_body);
        let (target_url, req_headers, upstream_body) = match prep_res {
            Ok(res) => res,
            Err(e) => {
                last_error = (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                continue;
            }
        };
        
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
                continue;
            }
        };
        
        let status = res.status();
        
        // 3. Handle Streaming vs Non-streaming
        let is_stream = upstream_body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
        
        if !is_stream {
            let res_body: Value = match res.json().await {
                Ok(b) => b,
                Err(e) => {
                    last_error = (StatusCode::BAD_GATEWAY, e.to_string());
                    continue;
                }
            };
    
            if !status.is_success() {
                if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    // Rate limited or server error -> FAILOVER
                    let err_text = res_body.to_string();
                    tracing::warn!("Account {} returned {}. Failing over. Body: {}", account.name, status, err_text);
                    last_error = (status, err_text);
                    continue;
                } else {
                    // 400 Bad Request etc -> return directly
                    return Ok((status, Json(res_body)).into_response());
                }
            }
    
            let standard_res = upstream_adapter
                .response_to_standard(res_body)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
            let converted_res = converter
                .standard_to_response(standard_res)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
            return Ok(Json(converted_res).into_response());
        } else {
            if !status.is_success() {
                let err_text = res.text().await.unwrap_or_default();
                if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    tracing::warn!("Account {} returned {}. Failing over. Body: {}", account.name, status, err_text);
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
            let mapped_stream = converter.standard_to_response_stream(standard_stream);
    
            let body = axum::body::Body::from_stream(mapped_stream);
    
            return response_builder
                .body(body)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to build stream response: {}", e)));
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
    handle_request(State(state), headers, &AnthropicConverter, body).await
}

async fn handle_openai_chat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), headers, &OpenAiConverter, body).await
}

async fn handle_openai_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), headers, &OpenAiConverter, body).await
}

async fn handle_openai_responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), headers, &OpenAiResponsesConverter, body).await
}

async fn handle_gemini(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), headers, &GeminiConverter, body).await
}

async fn handle_unified_models() -> Result<Response, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    let mut unique_models = std::collections::HashSet::new();
    
    // Add models and aliases from all accounts
    for account in settings.accounts.iter() {
        for model in &account.models {
            unique_models.insert(model.clone());
        }
        for alias in account.model_mapping.keys() {
            unique_models.insert(alias.clone());
        }
    }
    
    // Fallback if no models are configured
    if unique_models.is_empty() {
        unique_models.insert("gpt-4o".to_string());
    }
    
    let mut data = Vec::new();
    for model in unique_models {
        data.push(serde_json::json!({
            "id": model,
            "object": "model",
            "created": 1686935002,
            "owned_by": "hub-rs"
        }));
    }
    
    let res = serde_json::json!({
        "object": "list",
        "data": data
    });
    
    Ok(Json(res).into_response())
}

