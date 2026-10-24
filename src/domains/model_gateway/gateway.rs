use reqwest::StatusCode;
use serde_json::Value;
use std::time::{Duration, Instant};

use crate::config::llm_settings::{self, LLMSettings};
use crate::domains::model_gateway::routing::{
    ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome, ProviderAttemptReceipt,
    ProviderCandidate,
};
use crate::domains::model_gateway::health::{ProviderAdmission, ProviderHealthRuntime};
use crate::domains::model_gateway::protocols::sse::SseEventScanner;
use crate::domains::model_gateway::protocols::upstream::{adapter_for_provider, UpstreamAdapter};
use crate::services::usage_tracker::ApiUsageRecord;
use crate::domains::model_gateway::providers::usage_from_standard;
use crate::services::cli_auth::AuthRuntime;

/// 等待首个 SSE 事件的预算：慢推理模型首字节可能很慢，上限取宽；
/// 超出视为上游挂死，按 Transient 进入 failover。
const FIRST_EVENT_TIMEOUT: Duration = Duration::from_secs(300);
/// 首事件前的最大缓冲字节：正常首个事件块远小于该值；超限说明上游在
/// 200 之后倾倒非 SSE 内容（如 HTML 错误页），按失败处理。
const FIRST_EVENT_MAX_BYTES: usize = 256 * 1024;

pub struct GatewayExecuteResult {
    pub status: StatusCode,
    pub standard_response: Option<Value>,
    pub response_body_text: Option<String>,
    /// 已确认存活（首个事件已到达）的原始上游流。反代拿到它即可安全转发：
    /// 若上游在首事件前就断流，execute_model_request 内部已按失败处理并
    /// 走完 failover，不会把空流交给调用方。
    pub raw_stream: Option<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>,
        >,
    >,
    /// 等待首事件期间已读出的字节，转发时需前插回流的头部，保证字节序完整。
    pub stream_head: Option<bytes::Bytes>,
    /// 原始响应的 Content-Type（流被消费前捕获）。
    pub content_type: Option<String>,
    pub usage: crate::domains::model_gateway::providers::TokenUsage,
    pub receipts: Vec<ProviderAttemptReceipt>,
    pub account: Option<crate::config::llm_settings::LLMAccount>,
}

/// 首事件缓冲的判定结果。
enum FirstEventOutcome {
    /// 首个完整 SSE 事件已确认：缓冲字节 + 剩余流。
    Confirmed {
        head: bytes::Bytes,
        rest: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>,
        >,
    },
    /// 流在产出首个事件前死亡/超时/返回错误载荷（message 用于失败分类）。
    Failed(String),
}

/// 检查首个完整 SSE 事件块是否为上游错误载荷（部分供应商用 200 + error 事件
/// 表达失败，此时仍有机会 failover）。
fn first_block_error(block: &str) -> Option<String> {
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("event:") && trimmed["event:".len()..].trim() == "error" {
            return Some("upstream stream error event before first token".to_string());
        }
        let Some(data) = trimmed
            .strip_prefix("data: ")
            .or_else(|| trimmed.strip_prefix("data:"))
        else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(data) {
            if let Some(error) = value.get("error") {
                let snippet = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown upstream stream error");
                return Some(format!("upstream stream error payload: {snippet}"));
            }
            return None;
        }
    }
    None
}

/// 从已收到 2xx 的流式响应中等待首个完整 SSE 事件。
async fn await_first_stream_event(response: reqwest::Response) -> (Option<String>, FirstEventOutcome) {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let mut stream = response.bytes_stream();
    let mut scanner = SseEventScanner::new();
    let mut head: Vec<u8> = Vec::new();

    loop {
        let next = tokio::time::timeout(FIRST_EVENT_TIMEOUT, futures::StreamExt::next(&mut stream)).await;
        match next {
            Err(_) => {
                return (content_type, FirstEventOutcome::Failed("first stream event timeout".into()))
            }
            Ok(None) => {
                // 流在分隔符前自然结束：把尾部缓冲当作最后一个事件块判定。
                // 尾部必须含至少一行 data: 才算产出过有效事件（200 + HTML
                // 错误页一类垃圾结尾仍应走失败路径）。
                if let Some(tail) = scanner.flush() {
                    if let Some(error) = first_block_error(&tail) {
                        return (content_type, FirstEventOutcome::Failed(error));
                    }
                    if tail.lines().any(|line| line.trim().starts_with("data:")) {
                        return (
                            content_type,
                            FirstEventOutcome::Confirmed {
                                head: bytes::Bytes::from(head),
                                rest: Box::pin(futures::stream::unfold(stream, |mut s| async move {
                                    futures::StreamExt::next(&mut s).await.map(|item| (item, s))
                                })),
                            },
                        );
                    }
                    return (
                        content_type,
                        FirstEventOutcome::Failed("stream ended without SSE data event".into()),
                    );
                }
                return (
                    content_type,
                    FirstEventOutcome::Failed("stream ended before first event".into()),
                );
            }
            Ok(Some(Err(error))) => {
                return (
                    content_type,
                    FirstEventOutcome::Failed(format!("stream error before first event: {error}")),
                )
            }
            Ok(Some(Ok(chunk))) => {
                head.extend_from_slice(&chunk);
                if head.len() > FIRST_EVENT_MAX_BYTES {
                    return (
                        content_type,
                        FirstEventOutcome::Failed(
                            "no complete SSE event within first 256KB of stream".into(),
                        ),
                    );
                }
                let blocks = scanner.push_chunk(&chunk);
                if let Some(block) = blocks.first() {
                    if let Some(error) = first_block_error(block) {
                        return (content_type, FirstEventOutcome::Failed(error));
                    }
                    return (
                        content_type,
                        FirstEventOutcome::Confirmed {
                            head: bytes::Bytes::from(head),
                            rest: Box::pin(futures::stream::unfold(stream, |mut s| async move {
                                futures::StreamExt::next(&mut s).await.map(|item| (item, s))
                            })),
                        },
                    );
                }
            }
        }
    }
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

        let is_gemini_native = attempt_settings
            .provider
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            == "gemini_native";

        if is_gemini_native
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

        if !is_gemini_native {
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
                        if !is_gemini_native {
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
                    raw_stream: None,
                    stream_head: None,
                    content_type: None,
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
            // 流式首事件缓冲：上游 200 后立即断流（0 事件）或直接给出错误载荷时，
            // 客户端尚未收到任何字节，此时仍可安全 failover 到下一候选；
            // 一旦首个完整事件确认，剩余流透传给反代，不再引入额外转发延迟。
            let (content_type, outcome) = await_first_stream_event(res).await;
            match outcome {
                FirstEventOutcome::Confirmed { head, rest } => {
                    // 流式尝试的健康生命周期由 track_provider_stream 收尾，
                    // 防止重复计数成功与 EMA 抖动。
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
                        raw_stream: Some(rest),
                        stream_head: Some(head),
                        content_type,
                        usage: crate::domains::model_gateway::providers::TokenUsage::default(),
                        receipts,
                        account: Some(candidate.account.clone()),
                    });
                }
                FirstEventOutcome::Failed(message) => {
                    let class = route_runtime.classify_failure(&message);
                    match class {
                        crate::domains::model_gateway::model_attempt_router::ModelErrorClass::Authentication => {
                            health.finish_attempt(&acc_key, ProviderAttemptOutcome::Authentication, attempt_duration_ms);
                            health.finish_attempt(&mod_key, ProviderAttemptOutcome::Authentication, attempt_duration_ms);
                        }
                        crate::domains::model_gateway::model_attempt_router::ModelErrorClass::RateLimited => {
                            health.finish_attempt(&mod_key, ProviderAttemptOutcome::RateLimited, attempt_duration_ms);
                            health.finish_attempt(&acc_key, ProviderAttemptOutcome::Succeeded, attempt_duration_ms);
                        }
                        crate::domains::model_gateway::model_attempt_router::ModelErrorClass::Transient => {
                            health.finish_attempt(&mod_key, ProviderAttemptOutcome::Transient, attempt_duration_ms);
                            health.cancel_attempt(&acc_key);
                        }
                        _ => {
                            health.finish_attempt(&mod_key, class.into(), attempt_duration_ms);
                            health.finish_attempt(&acc_key, ProviderAttemptOutcome::Succeeded, attempt_duration_ms);
                        }
                    }
                    receipts.push(ProviderAttemptReceipt::new(
                        &candidate,
                        attempt,
                        attempt_duration_ms,
                        class.into(),
                        Some(502),
                    ));
                    last_error = (StatusCode::BAD_GATEWAY, message);
                    if route_runtime.should_try_next(class, attempt < candidate_count) {
                        continue;
                    }
                    return Err(last_error);
                }
            }
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
            raw_stream: None,
            stream_head: None,
            content_type: None,
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

#[cfg(test)]
mod first_event_tests {
    use super::{await_first_stream_event, first_block_error, FirstEventOutcome};
    use futures::StreamExt;

    fn response_from_chunks(chunks: Vec<Result<bytes::Bytes, std::io::Error>>) -> reqwest::Response {
        let body = reqwest::Body::wrap_stream(futures::stream::iter(chunks));
        let response = http::Response::builder()
            .status(200)
            .header("content-type", "text/event-stream")
            .body(body)
            .unwrap();
        reqwest::Response::from(response)
    }

    #[tokio::test]
    async fn immediate_dead_stream_is_failed() {
        let (_, outcome) = await_first_stream_event(response_from_chunks(vec![])).await;
        match outcome {
            FirstEventOutcome::Failed(message) => {
                assert!(message.contains("before first event"), "got: {message}")
            }
            FirstEventOutcome::Confirmed { .. } => panic!("dead stream must not be confirmed"),
        }
    }

    #[tokio::test]
    async fn confirmed_stream_preserves_head_and_rest_order() {
        // 首事件在 chunk2 末尾完成确认：head=chunk1+2（含 A 不含 B），chunk3 未读留在 rest
        let response = response_from_chunks(vec![
            Ok(bytes::Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"A\"",
            )),
            Ok(bytes::Bytes::from_static(b"}}]}

")),
            Ok(bytes::Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}

data: [DONE]

",
            )),
        ]);
        let (content_type, outcome) = await_first_stream_event(response).await;
        assert_eq!(content_type.as_deref(), Some("text/event-stream"));
        match outcome {
            FirstEventOutcome::Confirmed { head, rest } => {
                let head_text = String::from_utf8_lossy(&head).to_string();
                assert!(head_text.contains("\"A\""), "head must carry the first event");
                assert!(!head_text.contains("\"B\""), "head must stop before unread chunks");
                let collected: Vec<_> = rest.collect().await;
                let joined = collected
                    .iter()
                    .map(|c| match c {
                        Ok(b) => String::from_utf8_lossy(b).to_string(),
                        Err(e) => format!("<err {e}>"),
                    })
                    .collect::<String>();
                assert!(joined.contains("\"B\""));
                assert!(joined.contains("[DONE]"));
            }
            FirstEventOutcome::Failed(m) => panic!("healthy stream failed: {m}"),
        }
    }

    #[tokio::test]
    async fn error_payload_in_first_event_is_failed() {
        let response = response_from_chunks(vec![Ok(bytes::Bytes::from_static(
            b"data: {\"error\":{\"message\":\"quota exceeded\"}}\n\n",
        ))]);
        let (_, outcome) = await_first_stream_event(response).await;
        match outcome {
            FirstEventOutcome::Failed(message) => {
                assert!(message.contains("quota exceeded"), "got: {message}")
            }
            FirstEventOutcome::Confirmed { .. } => panic!("error payload must not be confirmed"),
        }
    }

    #[tokio::test]
    async fn keepalive_first_block_confirms_liveness() {
        // 注释行/心跳构成的首个事件块足以证明上游存活
        let response = response_from_chunks(vec![
            Ok(bytes::Bytes::from_static(b": keep-alive\n\n")),
            Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n")),
        ]);
        let (_, outcome) = await_first_stream_event(response).await;
        assert!(matches!(outcome, FirstEventOutcome::Confirmed { .. }));
    }

    #[tokio::test]
    async fn garbage_200_body_without_data_lines_is_failed() {
        // 200 + HTML 错误页（无任何 data: 行，无分隔符直接结束）
        let response = response_from_chunks(vec![Ok(bytes::Bytes::from_static(
            b"<html><body>Bad Gateway</body></html>",
        ))]);
        let (_, outcome) = await_first_stream_event(response).await;
        match outcome {
            FirstEventOutcome::Failed(message) => {
                assert!(message.contains("without SSE data"), "got: {message}")
            }
            FirstEventOutcome::Confirmed { .. } => panic!("garbage body must not be confirmed"),
        }
    }

    #[test]
    fn first_block_error_detects_event_and_payload_forms() {
        assert!(first_block_error("event: error\ndata: {}").is_some());
        assert!(first_block_error("data: {\"error\":{\"message\":\"x\"}}").is_some());
        assert!(first_block_error("data: {\"choices\":[{\"delta\":{}}]}").is_none());
        assert!(first_block_error(": keep-alive").is_none());
    }
}
