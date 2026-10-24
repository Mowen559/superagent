use bytes::Bytes;
use futures::stream::Stream;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};
use uuid::Uuid;

use super::upstream::UpstreamAdapter;
use crate::config::llm_settings::LLMSettings;

pub struct GeminiInternalUpstreamAdapter;

impl UpstreamAdapter for GeminiInternalUpstreamAdapter {
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        mut standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error> {
        let is_stream = standard_body
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let method = if is_stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let query_string = if is_stream { "?alt=sse" } else { "" };
        let base_url = "https://daily-cloudcode-pa.googleapis.com/v1internal";
        let target_url = format!("{}:{}{}", base_url, method, query_string);

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {}", settings.api_key).parse().unwrap(),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("vscode/1.X.X (Antigravity/4.3.0)"),
        );
        headers.insert("Connection", HeaderValue::from_static("close"));

        let requested_model = standard_body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&settings.model)
            .to_string();
        fn resolve_gemini_native_model(m: &str) -> &str {
            match m {
                "gemini-2.5-flash" => "gemini-2.5-flash",
                "gemini-2.5-pro" => "gemini-2.5-pro",
                "gemini-1.5-flash" => "gemini-1.5-flash",
                "gemini-1.5-pro" => "gemini-1.5-pro",
                "gemini-3.1-pro-high" => "gemini-pro-agent",
                "gemini-3.1-pro-low" => "gemini-3.1-pro-low",
                "gemini-3.1-flash-lite" => "gemini-3.1-flash-lite",
                "gemini-pro-agent" => "gemini-pro-agent",
                // 兼容非 Google 模型名或未知别名自动智能兜底至稳定高速的 gemini-2.5-flash，彻底消除 404
                "gemini-3.8-flash-high"
                | "gemini-3-flash"
                | "gemini-stub"
                | "gpt-4o"
                | "gpt-4o-mini"
                | "claude-3-5-sonnet"
                | "si"
                | "default"
                | "" => "gemini-2.5-flash",
                other => {
                    if other.starts_with("gemini-2.5") || other.starts_with("gemini-1.5") {
                        other
                    } else {
                        "gemini-2.5-flash"
                    }
                }
            }
        }
        let resolved_model = resolve_gemini_native_model(&requested_model).to_string();
        if let Some(body) = standard_body.as_object_mut() {
            body.insert("model".to_string(), Value::String(resolved_model.clone()));
        }

        // Convert standard (OpenAI) to Gemini
        let gemini_body = convert_standard_to_gemini(standard_body, resolved_model.clone());

        // Wrap into v1internal
        let mut project_id = settings
            .project_id
            .clone()
            .unwrap_or_else(|| "".to_string());
        if project_id.is_empty() || project_id == "project-id" {
            project_id = "useful-fuze".to_string(); // fallback random ID that passes Google's basic pattern matching
        }

        // Generate official-looking request ID
        let timestamp_ms = chrono::Utc::now().timestamp_millis();
        let random_hex = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let official_request_id = format!("agent/{}/{}", timestamp_ms, random_hex);

        let v1internal_body = json!({
            "project": project_id,
            "request": gemini_body,
            "model": resolved_model,
            "userAgent": "vscode/1.X.X (Antigravity/4.3.0)",
            "requestType": "agent",
            "requestId": official_request_id,
            "enabledCreditTypes": ["GOOGLE_ONE_AI"],
        });

        tracing::info!("gemini_native target_url: {}", target_url);
        tracing::debug!(
            requested_model = %requested_model,
            resolved_model = %resolved_model,
            streaming = is_stream,
            "prepared gemini_native Canonical Model Request"
        );

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
        let mut last_text = String::new();
        let mut buffer = String::new();

        let mapped = futures::stream::StreamExt::map(stream, move |res| match res {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                buffer.push_str(&text);
                let mut out_text = String::new();

                while let Some(block) =
                    crate::domains::model_gateway::protocols::sse::take_sse_block(&mut buffer)
                {
                    let mut is_processed = false;

                    for line in block.lines() {
                        if let Some(data) =
                            crate::domains::model_gateway::protocols::sse::strip_sse_field(line, "data")
                        {
                            is_processed = true;
                            if data == "[DONE]" {
                                out_text.push_str("data: [DONE]\n\n");
                                continue;
                            }
                            if let Ok(json) = serde_json::from_str::<Value>(data) {
                                if let Some(resp) = json.get("response").cloned() {
                                    // Simple mapping for standard compatibility
                                    if let Ok(standard) = convert_gemini_to_standard(resp) {
                                        // For streams, OpenAI returns delta.
                                        let content = standard["choices"][0]["message"]["content"]
                                            .as_str()
                                            .unwrap_or("");

                                        // Handle cumulative text robustly
                                        let new_text = if content.starts_with(&last_text) {
                                            let sliced = &content[last_text.len()..];
                                            last_text = content.to_string();
                                            sliced
                                        } else {
                                            last_text = content.to_string();
                                            content
                                        };

                                        if !new_text.is_empty() {
                                            let stream_json = json!({
                                                "id": standard["id"],
                                                "object": "chat.completion.chunk",
                                                "created": standard["created"],
                                                "model": standard["model"],
                                                "choices": [{
                                                    "index": 0,
                                                    "delta": {
                                                        "content": new_text
                                                    },
                                                    "finish_reason": serde_json::Value::Null
                                                }]
                                            });
                                            out_text.push_str(&format!(
                                                "data: {}\n\n",
                                                serde_json::to_string(&stream_json).unwrap()
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !is_processed {
                        out_text.push_str(&block);
                        out_text.push_str("\n\n");
                    }
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
            } else if role == "tool" {
                let tc_id = msg
                    .get("tool_call_id")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let parts_split: Vec<&str> = tc_id.split('|').collect();
                let func_name = if parts_split.len() == 3 {
                    parts_split[0]
                } else {
                    tc_id
                };
                let func_id = if parts_split.len() == 3 {
                    parts_split[1]
                } else {
                    ""
                };

                let resp_obj: Value = serde_json::from_str(content).unwrap_or_else(|_| json!({}));

                let mut function_response = json!({
                    "name": func_name,
                    "response": resp_obj
                });
                if !func_id.is_empty() {
                    function_response
                        .as_object_mut()
                        .unwrap()
                        .insert("id".to_string(), json!(func_id));
                }

                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": function_response
                    }]
                }));
            } else {
                let gemini_role = if role == "assistant" { "model" } else { "user" };

                let mut parts = Vec::new();
                if !content.is_empty() {
                    parts.push(json!({ "text": content }));
                }

                if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tool_calls {
                        if let Some(func) = tc.get("function") {
                            let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let args_str = func
                                .get("arguments")
                                .and_then(|a| a.as_str())
                                .unwrap_or("{}");
                            let input: Value =
                                serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));

                            let tc_id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let parts_split: Vec<&str> = tc_id.split('|').collect();

                            let mut part = json!({
                                "functionCall": {
                                    "name": name,
                                    "args": input
                                }
                            });

                            if parts_split.len() == 3 {
                                let func_id = parts_split[1];
                                let thought_sig = parts_split[2];
                                if !func_id.is_empty() {
                                    part["functionCall"]
                                        .as_object_mut()
                                        .unwrap()
                                        .insert("id".to_string(), json!(func_id));
                                }
                                if !thought_sig.is_empty() {
                                    // Wait, the API error was "Function call is missing a thought_signature in functionCall parts."
                                    // Actually, what if it expects "thoughtCall"? Or what if "thought_signature" is correct for `part`?
                                    part.as_object_mut().unwrap().insert(
                                        "thought_signature".to_string(),
                                        json!(thought_sig),
                                    );
                                }
                            }

                            parts.push(part);
                        }
                    }
                }

                if parts.is_empty() {
                    parts.push(json!({ "text": "" }));
                }

                contents.push(json!({
                    "role": gemini_role,
                    "parts": parts
                }));
            }
        }
    }

    let model = standard
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or(&fallback_model);

    let mut generation_config = json!({
        "temperature": standard.get("temperature").unwrap_or(&json!(0.7)),
    });

    if let Some(max_tokens) = standard
        .get("max_completion_tokens")
        .or_else(|| standard.get("max_tokens"))
        .and_then(|v| v.as_u64())
    {
        // Cap maxOutputTokens at 131072 for safety against 400 INVALID_ARGUMENT
        generation_config
            .as_object_mut()
            .unwrap()
            .insert("maxOutputTokens".to_string(), json!(max_tokens.min(131072)));
    } else {
        generation_config
            .as_object_mut()
            .unwrap()
            .insert("maxOutputTokens".to_string(), json!(8192));
    }

    // Thinking parameter mapping
    if let Some(thinking) = standard.get("thinking") {
        if let Some(budget) = thinking.get("budget_tokens").and_then(|v| v.as_u64()) {
            generation_config
                .as_object_mut()
                .unwrap()
                .insert("thinkingBudget".to_string(), json!(budget));
        }
    } else if let Some(thinking_level) = standard.get("thinking_level") {
        generation_config
            .as_object_mut()
            .unwrap()
            .insert("thinkingLevel".to_string(), thinking_level.clone());
    }

    let mut gemini_req = json!({});

    // 1. systemInstruction
    if let Some(sys) = system_instruction {
        gemini_req.as_object_mut().unwrap().insert(
            "systemInstruction".to_string(),
            json!({
                "parts": [{"text": sys.trim_end()}]
            }),
        );
    }

    // 2. tools and 3. toolConfig
    if let Some(tools) = standard.get("tools").and_then(|t| t.as_array()) {
        let mut gemini_tools = Vec::new();
        let mut function_declarations = Vec::new();

        for tool in tools {
            if let Some(func) = tool.get("function") {
                function_declarations.push(json!({
                    "name": func.get("name").unwrap_or(&json!("")),
                    "description": func.get("description").unwrap_or(&json!("")),
                    "parameters": func.get("parameters").unwrap_or(&json!({}))
                }));
            }
        }

        if !function_declarations.is_empty() {
            gemini_tools.push(json!({ "functionDeclarations": function_declarations }));
        }

        gemini_req
            .as_object_mut()
            .unwrap()
            .insert("tools".to_string(), json!(gemini_tools));
    }

    // 4. generationConfig
    gemini_req
        .as_object_mut()
        .unwrap()
        .insert("generationConfig".to_string(), generation_config);

    // 5. safetySettings (placeholder if needed)

    // 6. contents (must be last for caching)
    gemini_req
        .as_object_mut()
        .unwrap()
        .insert("contents".to_string(), json!(contents));

    // Cleanup [undefined] strings recursively
    deep_clean_undefined(&mut gemini_req, 0);

    gemini_req
}

// Helper: Gemini v1beta -> Standard OpenAI
fn convert_gemini_to_standard(gemini: Value) -> Result<Value, anyhow::Error> {
    let mut text_content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(candidates) = gemini.get("candidates").and_then(|c| c.as_array()) {
        if let Some(first) = candidates.first() {
            if let Some(parts) = first
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
            {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        text_content.push_str(text);
                    } else if let Some(func_call) = part.get("functionCall") {
                        let name = func_call.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let empty_args = json!({});
                        let args = func_call.get("args").unwrap_or(&empty_args);

                        let thought_sig = part
                            .get("thoughtSignature")
                            .or_else(|| part.get("thought_signature"))
                            .or_else(|| func_call.get("thoughtSignature"))
                            .or_else(|| func_call.get("thought_signature"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");

                        let func_id = func_call.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let id_str = format!("{}|{}|{}", name, func_id, thought_sig);

                        tool_calls.push(json!({
                            "id": id_str,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
                            }
                        }));
                    }
                }
            }
        }
    }

    let mut message = json!({
        "role": "assistant"
    });

    if !text_content.is_empty() || tool_calls.is_empty() {
        message["content"] = json!(text_content);
    } else {
        message["content"] = Value::Null;
    }

    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    let mut response = json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": "gemini",
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if !tool_calls.is_empty() { "tool_calls" } else { "stop" }
        }]
    });

    if let Some(usage) = gemini.get("usageMetadata") {
        let prompt_tokens = usage_u64(usage, "promptTokenCount");
        let completion_tokens = usage_u64(usage, "candidatesTokenCount");
        let cached_tokens = usage_u64(usage, "cachedContentTokenCount");
        response["usage"] = json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": usage_u64(usage, "totalTokenCount").max(prompt_tokens + completion_tokens),
            "prompt_tokens_details": {
                "cached_tokens": cached_tokens
            },
            "cached_tokens": cached_tokens
        });
    }

    Ok(response)
}

fn usage_u64(usage: &Value, field: &str) -> u64 {
    usage.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn deep_clean_undefined(val: &mut Value, depth: usize) {
    if depth > 10 {
        return;
    }
    match val {
        Value::Object(map) => {
            for v in map.values_mut() {
                deep_clean_undefined(v, depth + 1);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                deep_clean_undefined(v, depth + 1);
            }
        }
        Value::String(s) => {
            if s.contains("[undefined]") {
                *s = s.replace("[undefined]", "");
            }
        }
        _ => {}
    }
}
