use super::ProtocolConverter;

use serde_json::{json, Value};

pub struct AnthropicConverter;

fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }

    let mut end = max_bytes;

    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    &text[..end]
}

impl ProtocolConverter for AnthropicConverter {
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        let mut result = json!({});

        if let Some(model) = body.get("model") {
            result["model"] = model.clone();
        }

        if let Some(max_tokens) = body.get("max_tokens") {
            result["max_tokens"] = max_tokens.clone();
        }

        if let Some(temp) = body.get("temperature") {
            result["temperature"] = temp.clone();
        }

        if let Some(stream) = body.get("stream") {
            result["stream"] = stream.clone();
        }

        if result
            .get("stream")
            .and_then(|s| s.as_bool())
            .unwrap_or(false)
        {
            result["stream_options"] = json!({"include_usage": true});
        }

        let mut messages = Vec::new();

        if let Some(system) = body.get("system") {
            if let Some(text) = system.as_str() {
                messages.push(json!({"role": "system", "content": truncate_utf8(text, 1_000_000)}));
            } else if let Some(arr) = system.as_array() {
                let text: Vec<&str> = arr
                    .iter()
                    .filter_map(|m| m.get("text").and_then(|t| t.as_str()))
                    .collect();

                let joined_text = text.join("\n");

                messages.push(
                    json!({"role": "system", "content": truncate_utf8(&joined_text, 1_000_000)}),
                );
            }
        }

        if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
            for msg in msgs {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

                if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                    let mut openai_content = Vec::new();

                    let mut tool_calls = Vec::new();

                    for block in arr {
                        if let Some(btype) = block.get("type").and_then(|t| t.as_str()) {
                            match btype {
                                "text" => {
                                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                        openai_content.push(json!({

                                            "type": "text",

                                            "text": truncate_utf8(text, 1_000_000)

                                        }));
                                    }
                                }

                                "image" => {
                                    if let Some(source) = block.get("source") {
                                        let media_type = source
                                            .get("media_type")
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("image/jpeg");

                                        let data = source
                                            .get("data")
                                            .and_then(|d| d.as_str())
                                            .unwrap_or("");

                                        openai_content.push(json!({

                                            "type": "image_url",

                                            "image_url": {

                                                "url": format!("data:{};base64,{}", media_type, data)

                                            }

                                        }));
                                    }
                                }

                                "tool_use" => {
                                    let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");

                                    let name =
                                        block.get("name").and_then(|n| n.as_str()).unwrap_or("");

                                    let empty_input = json!({});

                                    let input = block.get("input").unwrap_or(&empty_input);

                                    tool_calls.push(json!({

                                        "id": id,

                                        "type": "function",

                                        "function": {

                                            "name": name,

                                            "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string())

                                        }

                                    }));
                                }

                                "tool_result" => {
                                    let tool_use_id = block
                                        .get("tool_use_id")
                                        .and_then(|i| i.as_str())
                                        .unwrap_or("");

                                    let content = if let Some(text) =
                                        block.get("content").and_then(|c| c.as_str())
                                    {
                                        truncate_utf8(text, 1_000_000).to_string()
                                    } else if let Some(c_arr) =
                                        block.get("content").and_then(|c| c.as_array())
                                    {
                                        let texts: Vec<&str> = c_arr
                                            .iter()
                                            .filter_map(|c| {
                                                if c.get("type").and_then(|t| t.as_str())
                                                    == Some("text")
                                                {
                                                    c.get("text").and_then(|t| t.as_str())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();

                                        truncate_utf8(&texts.join("\n"), 1_000_000).to_string()
                                    } else {
                                        "".to_string()
                                    };

                                    messages.push(json!({

                                        "role": "tool",

                                        "tool_call_id": tool_use_id,

                                        "content": content

                                    }));
                                }

                                _ => {}
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

                        if !openai_content.is_empty() || !tool_calls.is_empty() {
                            messages.push(msg_obj);
                        }
                    }
                } else if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                    messages.push(json!({

                        "role": role,

                        "content": truncate_utf8(text, 1_000_000)

                    }));
                }
            }
        }

        result["messages"] = json!(messages);

        if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
            let openai_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({

                        "type": "function",

                        "function": {

                            "name": t.get("name").unwrap_or(&json!("")),

                            "description": t.get("description").unwrap_or(&json!("")),

                            "parameters": t.get("input_schema").unwrap_or(&json!({}))

                        }

                    })
                })
                .collect();

            if !openai_tools.is_empty() {
                result["tools"] = json!(openai_tools);
            }
        }

        Ok(result)
    }

    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error> {
        let mut content_blocks = Vec::new();

        let message = body.pointer("/choices/0/message");

        if let Some(msg) = message {
            if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                if !text.is_empty() {
                    content_blocks.push(json!({

                        "type": "text",

                        "text": truncate_utf8(text, 1_000_000)

                    }));
                }
            }

            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    if let Some(func) = tc.get("function") {
                        let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");

                        let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");

                        let args_str = func
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}");

                        let input: Value =
                            serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));

                        content_blocks.push(json!({

                            "type": "tool_use",

                           "id": id,

                            "name": name,

                           "input": input

                        }));
                    }
                }
            }
        }

        if content_blocks.is_empty() {
            content_blocks.push(json!({

                "type": "text",

                "text": ""

            }));
        }

        let model = body.get("model").cloned().unwrap_or(json!(""));

        let id = body.get("id").cloned().unwrap_or(json!(""));

        let usage = body.get("usage").cloned().unwrap_or(json!({

            "input_tokens": 0,

            "output_tokens": 0

        }));

        let stop_reason = if message.and_then(|m| m.get("tool_calls")).is_some() {
            "tool_use"
        } else {
            "end_turn"
        };

        Ok(json!({

            "id": id,

            "type": "message",

            "role": "assistant",

            "content": content_blocks,

            "model": model,

            "stop_reason": stop_reason,

            "stop_sequence": null,

            "usage": usage

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
        Box::pin(
            crate::services::protocol_converter::streaming::create_anthropic_sse_stream(stream),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_anthropic_extreme_long_string_truncation_utf8() {
        let converter = AnthropicConverter;
        let mut long_string = String::from_utf8(vec![b'A'; 999_999]).unwrap();
        long_string.push_str("你好");
        let body = json!({
            "messages": [{
                "role": "user",
                "content": long_string
            }]
        });
        let result = converter.request_to_standard(body).unwrap();
        let messages = result.get("messages").unwrap().as_array().unwrap();
        let content = messages[0].get("content").unwrap().as_str().unwrap();
        assert!(content.len() <= 1_000_000);
        assert!(content.len() >= 999_999);
    }

    #[test]
    fn test_anthropic_empty_payload_resilience() {
        let converter = AnthropicConverter;
        let body = json!({});
        let result = converter.request_to_standard(body).unwrap();
        assert!(result
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        let response_body = json!({});
        let response_result = converter.standard_to_response(response_body).unwrap();
        assert_eq!(
            response_result.get("role").unwrap().as_str().unwrap(),
            "assistant"
        );
        assert_eq!(
            response_result
                .get("content")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
