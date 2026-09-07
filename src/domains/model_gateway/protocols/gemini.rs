use super::ProtocolConverter;
use serde_json::{json, Value};

pub struct GeminiConverter;

impl ProtocolConverter for GeminiConverter {
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        let mut result = json!({});
        let mut messages = Vec::new();

        // Handle system instruction if present
        if let Some(sys) = body
            .get("systemInstruction")
            .and_then(|s| s.get("parts"))
            .and_then(|p| p.as_array())
        {
            let mut text = String::new();
            for part in sys {
                if let Some(t) = part.get("text").and_then(|txt| txt.as_str()) {
                    text.push_str(t);
                    text.push('\n');
                }
            }
            if !text.is_empty() {
                messages.push(json!({
                    "role": "system",
                    "content": text.trim_end()
                }));
            }
        }

        // Handle contents
        if let Some(contents) = body.get("contents").and_then(|c| c.as_array()) {
            for content in contents {
                let role = match content
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("user")
                {
                    "model" => "assistant",
                    "function" => "tool", // sometimes Gemini uses 'function'
                    other => other,
                };

                let mut openai_content = Vec::new();
                let mut tool_calls = Vec::new();

                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            openai_content.push(json!({
                                "type": "text",
                                "text": text
                            }));
                        } else if let Some(inline_data) = part.get("inlineData") {
                            let mime_type = inline_data
                                .get("mimeType")
                                .and_then(|m| m.as_str())
                                .unwrap_or("image/jpeg");
                            let data = inline_data
                                .get("data")
                                .and_then(|d| d.as_str())
                                .unwrap_or("");
                            openai_content.push(json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", mime_type, data)
                                }
                            }));
                        } else if let Some(func_call) = part.get("functionCall") {
                            let name = func_call.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let empty_args = json!({});
                            let args = func_call.get("args").unwrap_or(&empty_args);

                            // Gemini doesn't always provide an explicit ID for the call, but OpenAI requires one
                            let id = format!("call_{}", name);

                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
                                }
                            }));
                        } else if let Some(func_resp) = part.get("functionResponse") {
                            let name = func_resp.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let empty_resp = json!({});
                            let resp = func_resp.get("response").unwrap_or(&empty_resp);

                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": format!("call_{}", name),
                                "content": serde_json::to_string(resp).unwrap_or_else(|_| "{}".to_string())
                            }));
                        }
                    }
                }

                if !openai_content.is_empty() || !tool_calls.is_empty() {
                    let mut msg_obj = json!({
                        "role": role,
                        "content": if openai_content.len() == 1 && openai_content[0]["type"] == "text" {
                            openai_content[0]["text"].clone()
                        } else if !openai_content.is_empty() {
                            json!(openai_content)
                        } else {
                            json!(null)
                        }
                    });

                    if !tool_calls.is_empty() {
                        msg_obj["tool_calls"] = json!(tool_calls);
                    }

                    messages.push(msg_obj);
                }
            }
        }

        result["messages"] = json!(messages);

        // Settings
        if let Some(config) = body.get("generationConfig") {
            if let Some(temp) = config.get("temperature") {
                result["temperature"] = temp.clone();
            }
            if let Some(max_tokens) = config.get("maxOutputTokens") {
                result["max_tokens"] = max_tokens.clone();
            }
        }

        // Map tools
        if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
            let mut openai_tools = Vec::new();
            for tool in tools {
                if let Some(func_decls) =
                    tool.get("functionDeclarations").and_then(|f| f.as_array())
                {
                    for decl in func_decls {
                        openai_tools.push(json!({
                            "type": "function",
                            "function": {
                                "name": decl.get("name").unwrap_or(&json!("")),
                                "description": decl.get("description").unwrap_or(&json!("")),
                                "parameters": decl.get("parameters").unwrap_or(&json!({}))
                            }
                        }));
                    }
                }
            }
            if !openai_tools.is_empty() {
                result["tools"] = json!(openai_tools);
            }
        }

        Ok(result)
    }

    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error> {
        let mut parts = Vec::new();

        let message = body.pointer("/choices/0/message");
        if let Some(msg) = message {
            if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                if !text.is_empty() {
                    parts.push(json!({
                        "text": text
                    }));
                }
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

                        parts.push(json!({
                            "functionCall": {
                                "name": name,
                                "args": input
                            }
                        }));
                    }
                }
            }
        }

        // Ensure at least one part if empty
        if parts.is_empty() {
            parts.push(json!({
                "text": ""
            }));
        }

        let finish_reason = body
            .pointer("/choices/0/finish_reason")
            .and_then(|r| r.as_str())
            .unwrap_or("STOP");
        let model = body.get("model").cloned().unwrap_or(json!("gemini-stub"));

        Ok(json!({
            "candidates": [
                {
                    "content": {
                        "parts": parts,
                        "role": "model"
                    },
                    "finishReason": if finish_reason == "tool_calls" { "STOP" } else { finish_reason },
                    "index": 0,
                    "safetyRatings": []
                }
            ],
            "modelVersion": model
        }))
    }

    fn standard_to_response_stream(
        &self,
        stream: std::pin::Pin<
            Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
        >,
    ) -> std::pin::Pin<
        Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
    > {
        Box::pin(crate::domains::model_gateway::protocols::streaming::create_gemini_sse_stream(stream))
    }
}
