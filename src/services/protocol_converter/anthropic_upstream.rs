use std::{io, pin::Pin};

use bytes::Bytes;
use futures::{stream::Stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Map, Value};

use crate::config::llm_settings::LLMSettings;

use super::{
    sse::{append_utf8_safe, strip_sse_field, take_sse_block},
    upstream::UpstreamAdapter,
};

pub struct AnthropicUpstreamAdapter;

impl UpstreamAdapter for AnthropicUpstreamAdapter {
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error> {
        let base = settings.base_url.trim_end_matches('/');
        let target_url = if base.ends_with("/v1") {
            format!("{base}/messages")
        } else {
            format!("{base}/v1/messages")
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

        if settings.api_key.starts_with("sk-ant-") && !settings.api_key.starts_with("sk-ant-oauth")
        {
            headers.insert("x-api-key", HeaderValue::from_str(&settings.api_key)?);
        } else {
            let bearer = format!("Bearer {}", settings.api_key);
            headers.insert("Authorization", HeaderValue::from_str(&bearer)?);
        }

        Ok((
            target_url,
            headers,
            convert_standard_request(standard_body, settings),
        ))
    }

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        convert_anthropic_response(body)
    }

    fn response_stream_to_standard(
        &self,
        stream: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>> {
        Box::pin(convert_anthropic_stream(stream))
    }
}

fn convert_standard_request(standard: Value, settings: &LLMSettings) -> Value {
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();

    for message in standard
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        if role == "system" {
            system_parts.extend(text_parts(message.get("content")));
            continue;
        }

        let (anthropic_role, mut content) = if role == "tool" {
            let tool_call_id = message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content_text(message.get("content")),
                })],
            )
        } else {
            let anthropic_role = if role == "assistant" {
                "assistant"
            } else {
                "user"
            };
            let mut blocks = content_blocks(message.get("content"));
            if role == "assistant" {
                for tool_call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let function = tool_call.get("function").unwrap_or(&Value::Null);
                    let arguments = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or_else(|| json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tool_call.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "name": function.get("name").and_then(Value::as_str).unwrap_or_default(),
                        "input": arguments,
                    }));
                }
            }
            (anthropic_role, blocks)
        };

        if content.is_empty() {
            content.push(json!({ "type": "text", "text": "" }));
        }
        push_message(&mut messages, anthropic_role, content);
    }

    let max_tokens = standard
        .get("max_tokens")
        .or_else(|| standard.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(settings.max_tokens));
    let mut request = json!({
        "model": standard.get("model").and_then(Value::as_str).unwrap_or(&settings.model),
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": standard.get("stream").and_then(Value::as_bool).unwrap_or(false),
    });

    if !system_parts.is_empty() {
        request["system"] = Value::String(system_parts.join("\n\n"));
    }
    if let Some(temperature) = standard.get("temperature") {
        request["temperature"] = temperature.clone();
    }
    if let Some(stop) = standard.get("stop") {
        request["stop_sequences"] = stop.clone();
    }
    if let Some(tools) = standard.get("tools").and_then(Value::as_array) {
        let tools = tools
            .iter()
            .filter_map(|tool| tool.get("function"))
            .map(|function| {
                json!({
                    "name": function.get("name").cloned().unwrap_or(Value::String(String::new())),
                    "description": function.get("description").cloned().unwrap_or(Value::String(String::new())),
                    "input_schema": function.get("parameters").cloned().unwrap_or_else(|| json!({"type": "object"})),
                })
            })
            .collect::<Vec<_>>();
        if !tools.is_empty() {
            request["tools"] = Value::Array(tools);
        }
    }
    if let Some(tool_choice) = standard.get("tool_choice") {
        request["tool_choice"] = match tool_choice {
            Value::String(value) if value == "required" => json!({"type": "any"}),
            Value::String(value) if value == "auto" => json!({"type": "auto"}),
            Value::Object(object) => object
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(|name| json!({"type": "tool", "name": name}))
                .unwrap_or_else(|| tool_choice.clone()),
            _ => tool_choice.clone(),
        };
    }

    request
}

fn text_parts(content: Option<&Value>) -> Vec<String> {
    match content {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn content_text(content: Option<&Value>) -> String {
    text_parts(content).join("\n")
}

fn content_blocks(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({ "type": "text", "text": text })]
        }
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    Some(json!({ "type": "text", "text": text }))
                } else {
                    anthropic_image_block(part)
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn anthropic_image_block(part: &Value) -> Option<Value> {
    if part.get("type").and_then(Value::as_str) != Some("image_url") {
        return None;
    }
    let url = part
        .pointer("/image_url/url")
        .or_else(|| part.get("image_url"))
        .and_then(Value::as_str)?;
    if let Some(data) = url.strip_prefix("data:") {
        let (media_type, encoded) = data.split_once(";base64,")?;
        if media_type.is_empty() || encoded.is_empty() {
            return None;
        }
        Some(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": encoded,
            }
        }))
    } else if url.starts_with("https://") {
        Some(json!({
            "type": "image",
            "source": {"type": "url", "url": url}
        }))
    } else {
        None
    }
}

fn push_message(messages: &mut Vec<Value>, role: &str, mut content: Vec<Value>) {
    if let Some(previous) = messages.last_mut() {
        if previous.get("role").and_then(Value::as_str) == Some(role) {
            if let Some(previous_content) =
                previous.get_mut("content").and_then(Value::as_array_mut)
            {
                previous_content.append(&mut content);
                return;
            }
        }
    }
    messages.push(json!({ "role": role, "content": content }));
}

fn convert_anthropic_response(body: Value) -> Result<Value, anyhow::Error> {
    if body.get("type").and_then(Value::as_str) == Some("error") {
        anyhow::bail!(
            "Anthropic error: {}",
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown provider error")
        );
    }

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in body
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("tool_use") => tool_calls.push(json!({
                "id": block.get("id").cloned().unwrap_or(Value::String(String::new())),
                "type": "function",
                "function": {
                    "name": block.get("name").cloned().unwrap_or(Value::String(String::new())),
                    "arguments": serde_json::to_string(block.get("input").unwrap_or(&json!({})))?,
                }
            })),
            _ => {}
        }
    }

    let input_tokens = body
        .pointer("/usage/input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = body
        .pointer("/usage/output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let stop_reason = match body.get("stop_reason").and_then(Value::as_str) {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        _ => "stop",
    };
    let message_content = if text.is_empty() && !tool_calls.is_empty() {
        Value::Null
    } else {
        Value::String(text)
    };

    Ok(json!({
        "id": body.get("id").cloned().unwrap_or(Value::String(String::new())),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": body.get("model").cloned().unwrap_or(Value::String(String::new())),
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": message_content,
                "tool_calls": tool_calls,
            },
            "finish_reason": stop_reason,
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens,
        }
    }))
}

fn convert_anthropic_stream(
    mut input: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>,
) -> impl Stream<Item = Result<Bytes, io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder = Vec::new();
        let mut message_id = String::new();
        let mut model = String::new();

        while let Some(chunk) = input.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(error);
                    continue;
                }
            };
            append_utf8_safe(&mut buffer, &mut utf8_remainder, &chunk);

            while let Some(block) = take_sse_block(&mut buffer) {
                if let Some(output) = convert_sse_block(&block, &mut message_id, &mut model) {
                    yield Ok(Bytes::from(format!("data: {}\n\n", output)));
                }
            }
        }

        if !utf8_remainder.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
        }
        if !buffer.trim().is_empty() {
            if let Some(output) = convert_sse_block(&buffer, &mut message_id, &mut model) {
                yield Ok(Bytes::from(format!("data: {}\n\n", output)));
            }
        }
    }
}

fn convert_sse_block(block: &str, message_id: &mut String, model: &mut String) -> Option<String> {
    let mut event = None;
    let mut data = Vec::new();
    for line in block.lines() {
        if let Some(value) = strip_sse_field(line, "event") {
            event = Some(value.trim());
        } else if let Some(value) = strip_sse_field(line, "data") {
            data.push(value);
        }
    }
    let payload: Value = serde_json::from_str(&data.join("\n")).ok()?;
    let event = event
        .or_else(|| payload.get("type").and_then(Value::as_str))
        .unwrap_or_default();

    let mut chunk = Map::new();
    chunk.insert("id".to_string(), Value::String(message_id.clone()));
    chunk.insert(
        "object".to_string(),
        Value::String("chat.completion.chunk".to_string()),
    );
    chunk.insert("created".to_string(), json!(chrono::Utc::now().timestamp()));
    chunk.insert("model".to_string(), Value::String(model.clone()));

    match event {
        "message_start" => {
            *message_id = payload
                .pointer("/message/id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            *model = payload
                .pointer("/message/model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            chunk.insert("id".to_string(), Value::String(message_id.clone()));
            chunk.insert("model".to_string(), Value::String(model.clone()));
            chunk.insert(
                "choices".to_string(),
                json!([{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]),
            );
        }
        "content_block_start" => {
            let block = payload.get("content_block")?;
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return None;
            }
            let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0);
            chunk.insert(
                "choices".to_string(),
                json!([{"index": 0, "delta": {"tool_calls": [{
                    "index": index,
                    "id": block.get("id").cloned().unwrap_or(Value::String(String::new())),
                    "type": "function",
                    "function": {
                        "name": block.get("name").cloned().unwrap_or(Value::String(String::new())),
                        "arguments": ""
                    }
                }]}, "finish_reason": null}]),
            );
        }
        "content_block_delta" => {
            let delta = payload.get("delta")?;
            let standard_delta = match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => json!({
                    "content": delta.get("text").cloned().unwrap_or(Value::String(String::new()))
                }),
                Some("input_json_delta") => {
                    let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0);
                    json!({"tool_calls": [{
                        "index": index,
                        "function": {
                            "arguments": delta.get("partial_json").cloned().unwrap_or(Value::String(String::new()))
                        }
                    }]})
                }
                _ => return None,
            };
            chunk.insert(
                "choices".to_string(),
                json!([{"index": 0, "delta": standard_delta, "finish_reason": null}]),
            );
        }
        "message_delta" => {
            let finish_reason = match payload
                .pointer("/delta/stop_reason")
                .and_then(Value::as_str)
            {
                Some("tool_use") => "tool_calls",
                Some("max_tokens") => "length",
                _ => "stop",
            };
            chunk.insert(
                "choices".to_string(),
                json!([{"index": 0, "delta": {}, "finish_reason": finish_reason}]),
            );
            if let Some(output_tokens) = payload.pointer("/usage/output_tokens") {
                chunk.insert(
                    "usage".to_string(),
                    json!({"completion_tokens": output_tokens}),
                );
            }
        }
        "message_stop" => return Some("[DONE]".to_string()),
        "error" => {
            chunk.insert(
                "error".to_string(),
                json!({
                    "message": payload.pointer("/error/message").and_then(Value::as_str).unwrap_or("Anthropic stream error"),
                    "type": payload.pointer("/error/type").and_then(Value::as_str).unwrap_or("provider_error")
                }),
            );
            chunk.insert("choices".to_string(), json!([]));
        }
        _ => return None,
    }

    serde_json::to_string(&Value::Object(chunk)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> LLMSettings {
        LLMSettings {
            provider: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "sk-ant-test-key".to_string(),
            model: "claude-test".to_string(),
            max_tokens: 4096,
            ..LLMSettings::default()
        }
    }

    #[test]
    fn converts_canonical_tools_and_tool_results_to_anthropic() {
        let standard = json!({
            "model": "claude-test",
            "messages": [
                {"role": "system", "content": "Be precise."},
                {"role": "user", "content": "Read it"},
                {"role": "assistant", "content": null, "tool_calls": [{
                    "id": "call-1", "type": "function",
                    "function": {"name": "read_file", "arguments": "{\"path\":\"a.rs\"}"}
                }]},
                {"role": "tool", "tool_call_id": "call-1", "content": "contents"}
            ],
            "tools": [{"type": "function", "function": {
                "name": "read_file", "description": "Read a file",
                "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
            }}]
        });

        let (url, headers, body) = AnthropicUpstreamAdapter
            .prepare_request(&settings(), standard)
            .unwrap();

        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-test-key");
        assert_eq!(body["system"], "Be precise.");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn preserves_required_tool_choice_and_base64_images() {
        let standard = json!({
            "model": "claude-test",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "inspect"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
                ]
            }],
            "tools": [{"type": "function", "function": {
                "name": "inspect", "parameters": {"type": "object"}
            }}],
            "tool_choice": "required"
        });

        let (_, _, body) = AnthropicUpstreamAdapter
            .prepare_request(&settings(), standard)
            .unwrap();
        assert_eq!(body["tool_choice"], json!({"type": "any"}));
        assert_eq!(body["messages"][0]["content"][1]["type"], "image");
        assert_eq!(
            body["messages"][0]["content"][1]["source"]["media_type"],
            "image/png"
        );
        assert_eq!(body["messages"][0]["content"][1]["source"]["data"], "AAAA");
    }

    #[test]
    fn converts_anthropic_tool_use_and_usage_to_canonical_response() {
        let response = json!({
            "id": "msg-1",
            "model": "claude-test",
            "stop_reason": "tool_use",
            "content": [
                {"type": "text", "text": "I will inspect it."},
                {"type": "tool_use", "id": "call-1", "name": "read_file", "input": {"path": "a.rs"}}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 7}
        });

        let standard = AnthropicUpstreamAdapter
            .response_to_standard(response)
            .unwrap();
        assert_eq!(standard["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            standard["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "read_file"
        );
        assert_eq!(standard["usage"]["total_tokens"], 17);
    }

    #[tokio::test]
    async fn converts_split_anthropic_sse_tool_call_to_canonical_chunks() {
        let raw = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-1\",\"model\":\"claude-test\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":7}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let split = raw.len() / 2;
        let input = futures::stream::iter(vec![
            Ok(Bytes::copy_from_slice(&raw.as_bytes()[..split])),
            Ok(Bytes::copy_from_slice(&raw.as_bytes()[split..])),
        ]);

        let chunks = AnthropicUpstreamAdapter
            .response_stream_to_standard(Box::pin(input))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let rendered = chunks
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();

        assert!(rendered.contains("read_file"));
        assert!(rendered.contains("tool_calls"));
        assert!(rendered.contains("[DONE]"));
    }
}
