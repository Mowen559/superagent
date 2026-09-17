use reqwest::StatusCode;
use serde_json::Value;
use std::time::Instant;

use crate::config::llm_settings::{self, LLMSettings};
use crate::domains::model_gateway::routing::{
    ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome, ProviderAttemptReceipt,
    ProviderCandidate,
};
use crate::domains::model_gateway::health::{ProviderAdmission, ProviderHealthRuntime};
use crate::domains::model_gateway::protocols::upstream::{adapter_for_provider, UpstreamAdapter};
use crate::services::usage_tracker::ApiUsageRecord;
use crate::domains::model_gateway::providers::usage_from_standard;
use crate::services::cli_auth::AuthRuntime;

pub struct GatewayExecuteResult {
    pub status: StatusCode,
    pub standard_response: Option<Value>,
    pub response_body_text: Option<String>,
    pub raw_response: Option<reqwest::Response>,
    pub usage: crate::domains::model_gateway::providers::TokenUsage,
    pub receipts: Vec<ProviderAttemptReceipt>,
    pub account: Option<crate::config::llm_settings::LLMAccount>,
}

pub async fn execute_model_request(
    http_client: &reqwest::Client,
    requested_model: &str,
    requested_tags: &[String],
    standard_body: Value,
    override_token: Option<String>,
) -> Result<GatewayExecuteResult, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    let requirements = ModelRequirements::from_canonical_body(&standard_body);
    let route_runtime = ModelRouteRuntime::default();
    let route = route_runtime.plan_with_requirements(
        &settings,
        requested_model,
        requested_tags,
        requirements,
    );
    let candidate_count = route.candidates.len();
    let health = ProviderHealthRuntime::global();

    let mut last_error = (
        StatusCode::SERVICE_UNAVAILABLE,
        "No accounts available to handle request".to_string(),
    );

    let mut receipts = Vec::new();

    for (candidate_index, mut candidate) in route.candidates.into_iter().enumerate() {
        let attempt = candidate_index + 1;
        let attempt_started = Instant::now();
        let mut attempt_settings = candidate.attempt_settings(&settings);
        let mut has_retried_auth = false;

        let mut has_valid_oauth = false;
        if let Some(token) = &override_token {
            attempt_settings.api_key = token.clone();
            attempt_settings.enabled = true;
            has_valid_oauth = true;
        } else {
            // Get token
            match AuthRuntime::global()
                .get_valid_token(&mut candidate.account, http_client)
                .await
            {
                Ok(token) => {
                    attempt_settings.api_key = token;
                }
                Err(e) => {
                    let class = route_runtime.classify_failure(&format!("auth failed: {e}"));
                    let acc_key = candidate.account_health_key();
                    let mod_key = candidate.model_health_key();
                    let duration = crate::domains::model_gateway::providers::elapsed_ms(attempt_started);
                    health.finish_attempt(&acc_key, class.into(), duration);
                    health.finish_attempt(&mod_key, class.into(), duration);
                    receipts.push(ProviderAttemptReceipt::new(
                        &candidate,
                        attempt,
                        duration,
                        class.into(),
                        Some(401),
                    ));
                    last_error = (StatusCode::UNAUTHORIZED, format!("Auth error: {}", e));
                    if route_runtime.should_try_next(class, attempt < candidate_count) {
                        continue;
                    }
                    return Err(last_error);
                }
            }
        }

        if !has_valid_oauth && !llm_settings::has_usable_settings(&attempt_settings) {
            last_error = (
                StatusCode::SERVICE_UNAVAILABLE,
                "LLM provider is not configured".to_string(),
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
                load_gemini_native_project(http_client, &attempt_settings.api_key).await
            {
                attempt_settings.project_id = Some(project_id);
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

        let upstream_adapter = adapter_for_provider(&attempt_settings);
        let prep_res = upstream_adapter.prepare_request(&attempt_settings, attempt_body.clone());
        let (target_url, req_headers, upstream_body) = match prep_res {
            Ok(res) => res,
            Err(e) => {
                let class = route_runtime.classify_failure(&e.to_string());
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    crate::domains::model_gateway::providers::elapsed_ms(attempt_started),
                    class.into(),
                    Some(500),
                ));
                last_error = (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    continue;
                }
                return Err(last_error);
            }
        };

        let acc_key = candidate.account_health_key();
        let mod_key = candidate.model_health_key();

        // 1. 账号级断路器准入校验
        if let ProviderAdmission::Rejected { retry_after_ms } = health.begin_attempt(&acc_key) {
            last_error = (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Account Circuit open; retry after {retry_after_ms} ms"),
            );
            receipts.push(ProviderAttemptReceipt::new(
                &candidate,
                attempt,
                crate::domains::model_gateway::providers::elapsed_ms(attempt_started),
                ProviderAttemptOutcome::CircuitOpen,
                Some(503),
            ));
            if attempt < candidate_count {
                continue;
            }
            return Err(last_error);
        }

        // 2. 模型级断路器准入校验
        if let ProviderAdmission::Rejected { retry_after_ms } = health.begin_attempt(&mod_key) {
            // 释放账号级刚才可能开启的 HalfOpen 探针
            health.cancel_attempt(&acc_key);

            last_error = (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Model Circuit open for '{}'; retry after {retry_after_ms} ms", candidate.resolved_model),
            );
            receipts.push(ProviderAttemptReceipt::new(
                &candidate,
                attempt,
                crate::domains::model_gateway::providers::elapsed_ms(attempt_started),
                ProviderAttemptOutcome::CircuitOpen,
                Some(503),
            ));
            if attempt < candidate_count {
                continue;
            }
            return Err(last_error);
        }

        let mut req_builder = http_client.post(&target_url).json(&upstream_body);
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

        let res = match req_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                let class = route_runtime.classify_failure(&format!("request failed: {e}"));
                let attempt_duration = crate::domains::model_gateway::providers::elapsed_ms(attempt_started);
                health.finish_attempt(&mod_key, class.into(), attempt_duration);
                health.cancel_attempt(&acc_key);

                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    attempt_duration,
                    class.into(),
                    Some(502),
                ));
                last_error = (StatusCode::BAD_GATEWAY, e.to_string());
                if route_runtime.should_try_next(class, attempt < candidate_count) {
                    continue;
                }
                return Err(last_error);
            }
        };

        // 401 自动无感知刷新与原地重发机制 (In-place Seamless Retry)
        let mut res = res;
        if res.status() == StatusCode::UNAUTHORIZED
            && override_token.is_none()
            && candidate.account.refresh_token.is_some()
            && !has_retried_auth
        {
            has_retried_auth = true;
            tracing::warn!(
                "[Gateway] Upstream returned 401 Unauthorized for account '{}'. Attempting seamless token refresh and in-place retry...",
                candidate.account.id
            );

            let refresh_res = crate::services::cli_auth::runtime::AuthRuntime::global()
                .force_refresh_token(&mut candidate.account, http_client)
                .await;

            match refresh_res {
                Ok(new_token) => {
                    tracing::info!(
                        "[Gateway] Token refreshed successfully for account '{}'. Re-preparing request...",
                        candidate.account.id
                    );
                    attempt_settings.api_key = new_token;

                    if let Ok((retry_target_url, retry_req_headers, retry_upstream_body)) =
                        upstream_adapter.prepare_request(&attempt_settings, attempt_body.clone())
                    {
                        let mut retry_req_builder =
                            http_client.post(&retry_target_url).json(&retry_upstream_body);
                        for (k, v) in retry_req_headers.iter() {
                            if !k.as_str().eq_ignore_ascii_case("content-type") {
                                retry_req_builder = retry_req_builder.header(k, v);
                            }
                        }
                        if attempt_settings.provider != "gemini_native" {
                            if let Some(project_id) = &attempt_settings.project_id {
                                retry_req_builder =
                                    retry_req_builder.header("x-goog-user-project", project_id);
                            }
                        }

                        if let Ok(retry_res) = retry_req_builder.send().await {
                            tracing::info!(
                                "[Gateway] In-place retry completed with status: {}",
                                retry_res.status()
                            );
                            res = retry_res;
                        }
                    }
                }
                Err(err) => {
                    tracing::error!(
                        "[Gateway] Failed to force refresh token for account '{}': {}",
                        candidate.account.id,
                        err
                    );
                }
            }
        }

        let status = res.status();
        let attempt_duration_ms = crate::domains::model_gateway::providers::elapsed_ms(attempt_started);

        if !status.is_success() {
            // Need to read body for error analysis
            let error_text = res.text().await.unwrap_or_default();
            let status_u16 = status.as_u16();
            let class = route_runtime.classify_http_failure(status_u16, &error_text);
            let attempt_duration = attempt_duration_ms;

            match class {
                crate::domains::model_gateway::model_attempt_router::ModelErrorClass::Authentication => {
                    health.finish_attempt(&acc_key, ProviderAttemptOutcome::Authentication, attempt_duration);
                    health.finish_attempt(&mod_key, ProviderAttemptOutcome::Authentication, attempt_duration);
                }
                crate::domains::model_gateway::model_attempt_router::ModelErrorClass::RateLimited => {
                    health.finish_attempt(&mod_key, ProviderAttemptOutcome::RateLimited, attempt_duration);
                    health.finish_attempt(&acc_key, ProviderAttemptOutcome::Succeeded, attempt_duration);
                }
                crate::domains::model_gateway::model_attempt_router::ModelErrorClass::Transient => {
                    health.finish_attempt(&mod_key, ProviderAttemptOutcome::Transient, attempt_duration);
                    health.cancel_attempt(&acc_key);
                }
                _ => {
                    if status_u16 == 404 || (error_text.contains("model") && error_text.contains("not found")) {
                        health.finish_attempt(&mod_key, ProviderAttemptOutcome::Permanent, attempt_duration);
                    } else {
                        health.finish_attempt(&mod_key, class.into(), attempt_duration);
                    }
                    health.finish_attempt(&acc_key, ProviderAttemptOutcome::Succeeded, attempt_duration);
                }
            }

            receipts.push(ProviderAttemptReceipt::new(
                &candidate,
                attempt,
                attempt_duration_ms,
                class.into(),
                Some(status.as_u16()),
            ));
            if route_runtime.should_try_next(class, attempt < candidate_count) {
                last_error = (status, error_text);
                continue;
            } else {
                return Ok(GatewayExecuteResult {
                    status,
                    standard_response: None,
                    response_body_text: Some(error_text),
                    raw_response: None,
                    usage: crate::domains::model_gateway::providers::TokenUsage::default(),
                    receipts,
                    account: Some(candidate.account.clone()),
                });
            }
        }

        let is_stream = standard_body
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_stream {
            health.finish_attempt(&mod_key, ProviderAttemptOutcome::Succeeded, attempt_duration_ms);
            health.finish_attempt(&acc_key, ProviderAttemptOutcome::Succeeded, attempt_duration_ms);
            receipts.push(ProviderAttemptReceipt::new(
                &candidate,
                attempt,
                attempt_duration_ms,
                ProviderAttemptOutcome::Succeeded,
                Some(status.as_u16()),
            ));
            
            return Ok(GatewayExecuteResult {
                status,
                standard_response: None,
                response_body_text: None,
                raw_response: Some(res),
                usage: crate::domains::model_gateway::providers::TokenUsage::default(),
                receipts,
                account: Some(candidate.account.clone()),
            });
        }

        // Non-stream successful response
        let response_text = res.text().await.unwrap_or_default();
        let res_body: Value = match serde_json::from_str(&response_text) {
            Ok(body) => body,
            Err(error) => {
                health.finish_attempt(
                    &mod_key,
                    ProviderAttemptOutcome::Permanent,
                    attempt_duration_ms,
                );
                health.cancel_attempt(&acc_key);
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    attempt_duration_ms,
                    ProviderAttemptOutcome::Permanent,
                    Some(502),
                ));
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
                    &mod_key,
                    ProviderAttemptOutcome::Permanent,
                    attempt_duration_ms,
                );
                health.cancel_attempt(&acc_key);
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    attempt_duration_ms,
                    ProviderAttemptOutcome::Permanent,
                    Some(502),
                ));
                return Err((StatusCode::BAD_GATEWAY, error.to_string()));
            }
        };

        health.finish_attempt(&mod_key, ProviderAttemptOutcome::Succeeded, attempt_duration_ms);
        health.finish_attempt(&acc_key, ProviderAttemptOutcome::Succeeded, attempt_duration_ms);
        receipts.push(ProviderAttemptReceipt::new(
            &candidate,
            attempt,
            attempt_duration_ms,
            ProviderAttemptOutcome::Succeeded,
            Some(status.as_u16()),
        ));

        let mut usage = usage_from_standard(&standard_res);
        usage.duration_ms = attempt_duration_ms;
        usage.account_id = Some(candidate.account.id.clone());
        usage.provider = Some(candidate.account.provider.clone());
        usage.attempts = receipts.len().min(u8::MAX as usize) as u8;

        return Ok(GatewayExecuteResult {
            status,
            standard_response: Some(standard_res),
            response_body_text: Some(response_text),
            raw_response: None,
            usage,
            receipts,
            account: Some(candidate.account.clone()),
        });
    }

    Err(last_error)
}

pub async fn load_gemini_native_project(
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
