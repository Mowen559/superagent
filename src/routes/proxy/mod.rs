use axum::{
    extract::State,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use serde_json::Value;

use crate::{
    config::llm_settings,
    services::{
        protocol_converter::{
            anthropic::AnthropicConverter, gemini::GeminiConverter, openai::OpenAiConverter,
            openai_responses::OpenAiResponsesConverter, upstream::{OpenAiUpstreamAdapter, UpstreamAdapter}, ProtocolConverter,
        },
    },
    AppState,
};

use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/anthropic/v1/messages", post(handle_anthropic))
        .route("/anthropic/v1/models", get(handle_anthropic_models))
        .route("/openai/v1/chat/completions", post(handle_openai_chat))
        .route("/openai/v1/completions", post(handle_openai_completions))
        .route("/openai/v1/responses", post(handle_openai_responses))
        .route("/openai/v1/models", get(handle_openai_models))
        .route("/gemini/v1beta/models/{*path}", post(handle_gemini))
        .route("/gemini/v1beta/models", get(handle_gemini_models))
}

async fn handle_request(
    state: State<Arc<AppState>>,
    converter: &dyn ProtocolConverter,
    body: Value,
) -> Result<Response, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    if !llm_settings::has_usable_settings(&settings) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM provider is not configured".to_string(),
        ));
    }

    // 1. Convert incoming request to internal standard format (OpenAI-compatible)
    let mut standard_body = converter
        .request_to_standard(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Enforce configured model overriding client's choice
    if !settings.model.is_empty() {
        if let Some(obj) = standard_body.as_object_mut() {
            obj.insert("model".to_string(), Value::String(settings.model.clone()));
        }
    }

    // 2. Prepare HTTP request to the upstream provider
    // In MVP, we only support OpenAiUpstreamAdapter because Anthropic/Gemini Upstream parsing is complex
    // and would require full reverse mapping. Since we use compatible APIs for now, this works for OpenAI and Minimax.
    let upstream_adapter: Box<dyn UpstreamAdapter> = Box::new(OpenAiUpstreamAdapter);

    let (target_url, headers, upstream_body) = upstream_adapter
        .prepare_request(&settings, standard_body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut req_builder = state.http_client.post(&target_url).json(&upstream_body);
    for (k, v) in headers.iter() {
        req_builder = req_builder.header(k, v);
    }

    let res = req_builder
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    // 3. Handle Streaming vs Non-streaming
    let is_stream = upstream_body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    if !is_stream {
        let status = res.status();
        let res_body: Value = res
            .json()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
            
        if !status.is_success() {
            return Ok((status, Json(res_body)).into_response());
        }
            
        let standard_res = upstream_adapter
            .response_to_standard(res_body)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            
        let converted_res = converter
            .standard_to_response(standard_res)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            
        Ok(Json(converted_res).into_response())
    } else {
        let status = res.status();
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
        
        response_builder
            .body(body)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to build stream response: {}", e)))
    }
}

async fn handle_anthropic(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), &AnthropicConverter, body).await
}

async fn handle_openai_chat(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), &OpenAiConverter, body).await
}

async fn handle_openai_completions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), &OpenAiConverter, body).await
}

async fn handle_openai_responses(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), &OpenAiResponsesConverter, body).await
}

async fn handle_gemini(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    handle_request(State(state), &GeminiConverter, body).await
}

async fn handle_anthropic_models() -> Result<Response, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    let model = if settings.model.is_empty() { "claude-3-5-sonnet-20241022" } else { &settings.model };
    
    let res = serde_json::json!({
        "data": [
            {
                "type": "model",
                "id": model,
                "display_name": model,
                "created_at": "2024-01-01T00:00:00Z"
            }
        ],
        "has_more": false,
        "first_id": serde_json::Value::Null,
        "last_id": serde_json::Value::Null
    });
    
    Ok(Json(res).into_response())
}

async fn handle_openai_models() -> Result<Response, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    let model = if settings.model.is_empty() { "gpt-4o" } else { &settings.model };
    
    let res = serde_json::json!({
        "object": "list",
        "data": [
            {
                "id": model,
                "object": "model",
                "created": 1686935002,
                "owned_by": "hub-rs"
            }
        ]
    });
    
    Ok(Json(res).into_response())
}

async fn handle_gemini_models() -> Result<Response, (StatusCode, String)> {
    let settings = llm_settings::read_llm_settings();
    let model = if settings.model.is_empty() { "gemini-1.5-pro" } else { &settings.model };
    
    let res = serde_json::json!({
        "models": [
            {
                "name": format!("models/{}", model),
                "version": "001",
                "displayName": model,
                "description": "Proxy model",
                "inputTokenLimit": 32768,
                "outputTokenLimit": 8192,
                "supportedGenerationMethods": [
                    "generateContent",
                    "countTokens"
                ]
            }
        ]
    });
    
    Ok(Json(res).into_response())
}

