use bytes::Bytes;
use futures::stream::Stream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, AUTHORIZATION, USER_AGENT};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::llm_settings::LLMSettings;
use super::upstream::UpstreamAdapter;

pub struct GeminiInternalUpstreamAdapter;

impl UpstreamAdapter for GeminiInternalUpstreamAdapter {
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error> {
        let is_stream = standard_body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
        let method = if is_stream { "streamGenerateContent" } else { "generateContent" };
        let query_string = if is_stream { "?alt=sse" } else { "" };
        let base_url = "https://cloudcode-pa.googleapis.com/v1internal";
        let target_url = format!("{}:{}{}", base_url, method, query_string);

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(AUTHORIZATION, format!("Bearer {}", settings.api_key).parse().unwrap());
        headers.insert(USER_AGENT, HeaderValue::from_static("antigravity"));
        headers.insert("x-client-name", HeaderValue::from_static("antigravity"));
        
        let version = "vscode/1.X.X (Antigravity/4.3.6)";
        headers.insert("x-client-version", version.parse().unwrap());

        if let Ok(mid) = machine_uid::get() {
            headers.insert("x-machine-id", mid.parse().unwrap());
        } else {
            headers.insert("x-machine-id", "0000000000000000000000000000000000000000000000000000000000000000".parse().unwrap());
        }
        
        let session_id = Uuid::new_v4().to_string();
        headers.insert("x-vscode-sessionid", session_id.parse().unwrap());

        // We do not send x-goog-user-project header anymore because the API complains with duplicated projects when both the body and header are set

        // Convert standard (OpenAI) to Gemini
        let gemini_body = convert_standard_to_gemini(standard_body, settings.model.clone());

        // Wrap into v1internal
        let project_id = settings.project_id.clone().unwrap_or_else(|| "project-id".to_string());
        
        // Generate official-looking request ID
        let timestamp_ms = chrono::Utc::now().timestamp_millis();
        let random_hex = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let official_request_id = format!("agent/{}/{}", timestamp_ms, random_hex);

        let v1internal_body = json!({
            "project": project_id,
            "request": gemini_body,
            "model": settings.model,
            "userAgent": "antigravity",
            "requestType": "agent",
            "requestId": official_request_id,
            "enabledCreditTypes": ["GOOGLE_ONE_AI"],
        });

        tracing::info!("gemini_native target_url: {}", target_url);
        tracing::debug!("gemini_native req body: {}", serde_json::to_string(&v1internal_body).unwrap_or_default());

        Ok((target_url, headers, v1internal_body))
    }

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        // v1internal returns {"response": { actual_gemini_response... }}
        let actual_response = body.get("response").unwrap_or(&body);
        convert_gemini_to_standard(actual_response.clone())
    }

    fn response_stream_to_standard(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        // Due to complexity, we will implement a basic passthrough map, but ideally we'd parse SSE lines, 
        // extract `response`, and convert it to OpenAI stream format.
        
        let mapped = futures::stream::StreamExt::map(stream, move |res| match res {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let mut out_text = String::new();
                for line in text.lines() {
                    if line.starts_with("data: ") {
                        let data = &line[6..];
                        if data == "[DONE]" {
                            out_text.push_str("data: [DONE]\n\n");
                            continue;
                        }
                        if let Ok(mut json) = serde_json::from_str::<Value>(data) {
                            if let Some(resp) = json.get("response").cloned() {
                                // Simple mapping for standard compatibility
                                if let Ok(standard) = convert_gemini_to_standard(resp) {
                                    // For streams, OpenAI returns delta.
                                    // We approximate it by wrapping in stream format
                                    let content = standard["choices"][0]["message"]["content"].as_str().unwrap_or("");
                                    let stream_json = json!({
                                        "id": standard["id"],
                                        "object": "chat.completion.chunk",
                                        "created": standard["created"],
                                        "model": standard["model"],
                                        "choices": [{
                                            "index": 0,
                                            "delta": {
                                                "content": content
                                            },
                                            "finish_reason": serde_json::Value::Null
                                        }]
                                    });
                                    out_text.push_str(&format!("data: {}\n\n", serde_json::to_string(&stream_json).unwrap()));
                                    continue;
                                }
                            }
                        }
                    }
                    out_text.push_str(line);
                    out_text.push('\n');
                }
                Ok(Bytes::from(out_text))
            }
            Err(e) => Err(e),
        });

        Box::pin(mapped)
    }
}

// Helper: Standard OpenAI -> Gemini v1beta
fn convert_standard_to_gemini(mut standard: Value, fallback_model: String) -> Value {
    let mut contents = Vec::new();
    let mut system_instruction = None;

    if let Some(messages) = standard.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
            
            if role == "system" {
                let current_sys = system_instruction.unwrap_or_else(|| String::new());
                system_instruction = Some(format!("{}{}\n", current_sys, content));
            } else {
                let gemini_role = if role == "assistant" { "model" } else { "user" };
                contents.push(json!({
                    "role": gemini_role,
                    "parts": [{"text": content}]
                }));
            }
        }
    }

    let model = standard.get("model").and_then(|m| m.as_str()).unwrap_or(&fallback_model);
    
    let mut gemini_req = json!({
        "contents": contents,
        "model": model,
        "generationConfig": {
            "temperature": standard.get("temperature").unwrap_or(&json!(0.7)),
        }
    });

    if let Some(sys) = system_instruction {
        gemini_req.as_object_mut().unwrap().insert("systemInstruction".to_string(), json!({
            "parts": [{"text": sys.trim_end()}]
        }));
    }

    gemini_req
}

// Helper: Gemini v1beta -> Standard OpenAI
fn convert_gemini_to_standard(gemini: Value) -> Result<Value, anyhow::Error> {
    let mut text_content = String::new();
    if let Some(candidates) = gemini.get("candidates").and_then(|c| c.as_array()) {
        if let Some(first) = candidates.first() {
            if let Some(parts) = first.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()) {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        text_content.push_str(text);
                    }
                }
            }
        }
    }

    Ok(json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": "gemini",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": text_content
            },
            "finish_reason": "stop"
        }]
    }))
}
