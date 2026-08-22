use std::{collections::BTreeMap, io, pin::Pin};

use bytes::Bytes;
use futures::{stream::Stream, StreamExt};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::{
    sse::{append_utf8_safe, strip_sse_field, take_sse_block},
    ProtocolConverter,
};

pub struct OpenAiResponsesConverter;

impl ProtocolConverter for OpenAiResponsesConverter {
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        responses_request_to_canonical(body)
    }

    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error> {
        canonical_response_to_responses(body)
    }

    fn standard_to_response_stream(
        &self,
        stream: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>> {
        Box::pin(canonical_stream_to_responses(stream))
    }
}

fn responses_request_to_canonical(body: Value) -> Result<Value, anyhow::Error> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Responses request requires a model"))?;
    if body.get("previous_response_id").is_some() || body.get("conversation").is_some() {
        anyhow::bail!(
            "previous_response_id/conversation requires a stateful Responses store, which this Canonical Adapter does not own"
        );
    }

    let mut messages = Vec::new();
    if let Some(instructions) = body.get("instructions").and_then(Value::as_str) {
        messages.push(json!({"role": "system", "content": instructions}));
    }
    if let Some(input) = body.get("input") {
        convert_input(input, &mut messages)?;
    }

    let mut canonical = json!({
        "model": model,
        "messages": messages,
        "stream": body.get("stream").and_then(Value::as_bool).unwrap_or(false),
    });
    copy_field(&body, &mut canonical, "temperature", "temperature");
    copy_field(&body, &mut canonical, "top_p", "top_p");
    copy_field(&body, &mut canonical, "max_output_tokens", "max_tokens");
    copy_field(
        &body,
        &mut canonical,
        "parallel_tool_calls",
        "parallel_tool_calls",
    );

    if canonical["stream"] == Value::Bool(true) {
        canonical["stream_options"] = json!({"include_usage": true});
    }
    if let Some(reasoning_effort) = body.pointer("/reasoning/effort") {
        canonical["reasoning_effort"] = reasoning_effort.clone();
    }
    if let Some(format) = body.pointer("/text/format") {
        match format.get("type").and_then(Value::as_str) {
            Some("json_schema") => {
                canonical["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": format.get("name").cloned().unwrap_or(Value::String("response".to_string())),
                        "description": format.get("description").cloned().unwrap_or(Value::Null),
                        "schema": format.get("schema").cloned().unwrap_or_else(|| json!({"type": "object"})),
                        "strict": format.get("strict").cloned().unwrap_or(Value::Bool(false)),
                    }
                })
            }
            Some("json_object") => canonical["response_format"] = json!({"type": "json_object"}),
            _ => {}
        }
    }
    if let Some(choice) = body.get("tool_choice") {
        canonical["tool_choice"] = convert_tool_choice(choice);
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        canonical["tools"] = Value::Array(convert_tools(tools)?);
    }

    Ok(canonical)
}

fn convert_input(input: &Value, messages: &mut Vec<Value>) -> Result<(), anyhow::Error> {
    match input {
        Value::String(text) => {
            messages.push(json!({"role": "user", "content": text}));
        }
        Value::Array(items) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => {
                        messages.push(json!({
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": item.get("call_id").or_else(|| item.get("id")).cloned().unwrap_or(Value::String(String::new())),
                                "type": "function",
                                "function": {
                                    "name": item.get("name").cloned().unwrap_or(Value::String(String::new())),
                                    "arguments": item.get("arguments").cloned().unwrap_or(Value::String("{}".to_string())),
                                }
                            }]
                        }));
                    }
                    Some("function_call_output") => {
                        let output = item.get("output").cloned().unwrap_or(Value::Null);
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": item.get("call_id").cloned().unwrap_or(Value::String(String::new())),
                            "content": value_as_text(&output),
                        }));
                    }
                    Some("reasoning") => {}
                    Some("message") | None => messages.push(convert_message(item)?),
                    Some(kind) => anyhow::bail!(
                        "Responses input item type '{kind}' cannot be represented by the Canonical Chat Interface"
                    ),
                }
            }
        }
        _ => anyhow::bail!("Responses input must be a string or an array of input items"),
    }
    Ok(())
}

fn convert_message(item: &Value) -> Result<Value, anyhow::Error> {
    let role = match item.get("role").and_then(Value::as_str).unwrap_or("user") {
        "developer" => "system",
        role => role,
    };
    let content = match item.get("content") {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(parts)) => {
            let mut canonical_parts = Vec::new();
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("output_text") | Some("text") => {
                        canonical_parts.push(json!({
                            "type": "text",
                            "text": part.get("text").cloned().unwrap_or(Value::String(String::new())),
                        }));
                    }
                    Some("input_image") => {
                        let url = part
                            .get("image_url")
                            .and_then(Value::as_str)
                            .ok_or_else(|| anyhow::anyhow!("input_image requires image_url"))?;
                        canonical_parts.push(json!({
                            "type": "image_url",
                            "image_url": {"url": url, "detail": part.get("detail").cloned().unwrap_or(Value::String("auto".to_string()))},
                        }));
                    }
                    Some(kind) => anyhow::bail!(
                        "Responses content part '{kind}' cannot be represented by the Canonical Chat Interface"
                    ),
                    None => {}
                }
            }
            if canonical_parts.len() == 1 && canonical_parts[0]["type"] == "text" {
                canonical_parts[0]["text"].clone()
            } else {
                Value::Array(canonical_parts)
            }
        }
        _ => Value::String(String::new()),
    };
    Ok(json!({"role": role, "content": content}))
}

fn convert_tools(tools: &[Value]) -> Result<Vec<Value>, anyhow::Error> {
    tools
        .iter()
        .map(|tool| {
            let kind = tool.get("type").and_then(Value::as_str).unwrap_or_default();
            if kind != "function" {
                anyhow::bail!(
                    "Responses built-in tool '{kind}' has no provider-independent Canonical Chat representation"
                );
            }
            Ok(json!({
                "type": "function",
                "function": {
                    "name": tool.get("name").cloned().unwrap_or(Value::String(String::new())),
                    "description": tool.get("description").cloned().unwrap_or(Value::String(String::new())),
                    "parameters": tool.get("parameters").cloned().unwrap_or_else(|| json!({"type": "object"})),
                    "strict": tool.get("strict").cloned().unwrap_or(Value::Bool(false)),
                }
            }))
        })
        .collect()
}

fn convert_tool_choice(choice: &Value) -> Value {
    if let Some(name) = choice.get("name") {
        json!({"type": "function", "function": {"name": name}})
    } else {
        choice.clone()
    }
}

fn copy_field(source: &Value, target: &mut Value, source_name: &str, target_name: &str) {
    if let Some(value) = source.get(source_name) {
        target[target_name] = value.clone();
    }
}

fn value_as_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
}

fn canonical_response_to_responses(body: Value) -> Result<Value, anyhow::Error> {
    if let Some(error) = body.get("error") {
        return Ok(json!({
            "id": response_id(&body),
            "object": "response",
            "created_at": body.get("created").cloned().unwrap_or_else(|| json!(chrono::Utc::now().timestamp())),
            "status": "failed",
            "error": error,
            "output": [],
        }));
    }
    let choice = body
        .pointer("/choices/0")
        .ok_or_else(|| anyhow::anyhow!("Canonical response is missing choices[0]"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow::anyhow!("Canonical response is missing choices[0].message"))?;
    let mut output = Vec::new();

    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            output.push(response_message(text));
        }
    }
    for tool_call in message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let function = tool_call.get("function").unwrap_or(&Value::Null);
        output.push(json!({
            "id": format!("fc_{}", Uuid::new_v4().simple()),
            "type": "function_call",
            "status": "completed",
            "call_id": tool_call.get("id").cloned().unwrap_or(Value::String(String::new())),
            "name": function.get("name").cloned().unwrap_or(Value::String(String::new())),
            "arguments": function.get("arguments").cloned().unwrap_or(Value::String("{}".to_string())),
        }));
    }

    let finish_reason = choice.get("finish_reason").and_then(Value::as_str);
    let incomplete = finish_reason == Some("length");
    let usage = response_usage(body.get("usage"));
    Ok(json!({
        "id": response_id(&body),
        "object": "response",
        "created_at": body.get("created").cloned().unwrap_or_else(|| json!(chrono::Utc::now().timestamp())),
        "completed_at": if incomplete { Value::Null } else { json!(chrono::Utc::now().timestamp()) },
        "status": if incomplete { "incomplete" } else { "completed" },
        "error": null,
        "incomplete_details": if incomplete { json!({"reason": "max_output_tokens"}) } else { Value::Null },
        "model": body.get("model").cloned().unwrap_or(Value::String(String::new())),
        "output": output,
        "parallel_tool_calls": true,
        "usage": usage,
    }))
}

fn response_id(body: &Value) -> String {
    body.get("id")
        .and_then(Value::as_str)
        .filter(|id| id.starts_with("resp_"))
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("resp_{}", Uuid::new_v4().simple()))
}

fn response_message(text: &str) -> Value {
    json!({
        "id": format!("msg_{}", Uuid::new_v4().simple()),
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": text, "annotations": []}],
    })
}

fn response_usage(usage: Option<&Value>) -> Value {
    let input = usage
        .and_then(|usage| usage.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|usage| usage.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({"input_tokens": input, "output_tokens": output, "total_tokens": input + output})
}

#[derive(Default)]
struct ResponsesStreamState {
    response_id: String,
    model: String,
    created_at: i64,
    sequence: u64,
    started: bool,
    terminal: bool,
    next_output_index: usize,
    message: Option<ResponseMessageState>,
    tools: BTreeMap<usize, ResponseToolState>,
    finish_reason: Option<String>,
    usage: Option<Value>,
}

struct ResponseMessageState {
    item_id: String,
    output_index: usize,
    text: String,
}

#[derive(Default)]
struct ResponseToolState {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    pending_arguments: String,
    output_index: usize,
    started: bool,
}

fn canonical_stream_to_responses(
    mut input: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>,
) -> impl Stream<Item = Result<Bytes, io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut remainder = Vec::new();
        let mut state = ResponsesStreamState {
            response_id: format!("resp_{}", Uuid::new_v4().simple()),
            created_at: chrono::Utc::now().timestamp(),
            ..ResponsesStreamState::default()
        };

        while let Some(chunk) = input.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(error);
                    continue;
                }
            };
            append_utf8_safe(&mut buffer, &mut remainder, &chunk);
            while let Some(block) = take_sse_block(&mut buffer) {
                let output = process_chat_sse_block(&block, &mut state);
                if !output.is_empty() {
                    yield Ok(Bytes::from(output));
                }
            }
        }

        if !remainder.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&remainder));
        }
        if !buffer.trim().is_empty() {
            let output = process_chat_sse_block(&buffer, &mut state);
            if !output.is_empty() {
                yield Ok(Bytes::from(output));
            }
        }
        if !state.terminal {
            let output = if state.finish_reason.is_some() {
                finalize_stream(&mut state)
            } else {
                fail_stream(&mut state, "upstream_stream_ended", "Canonical stream ended without a finish reason")
            };
            if !output.is_empty() {
                yield Ok(Bytes::from(output));
            }
        }
    }
}

fn process_chat_sse_block(block: &str, state: &mut ResponsesStreamState) -> String {
    let data = block
        .lines()
        .filter_map(|line| strip_sse_field(line, "data"))
        .collect::<Vec<_>>()
        .join("\n");
    if data.trim() == "[DONE]" {
        return if state.finish_reason.is_some() {
            finalize_stream(state)
        } else {
            fail_stream(
                state,
                "upstream_stream_incomplete",
                "Canonical stream ended without a finish reason",
            )
        };
    }
    let Ok(payload) = serde_json::from_str::<Value>(&data) else {
        return String::new();
    };
    if let Some(error) = payload.get("error") {
        return fail_stream(
            state,
            error
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("provider_error"),
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Provider stream error"),
        );
    }

    if let Some(id) = payload.get("id").and_then(Value::as_str) {
        if id.starts_with("resp_") {
            state.response_id = id.to_string();
        }
    }
    if let Some(model) = payload.get("model").and_then(Value::as_str) {
        state.model = model.to_string();
    }
    if let Some(created) = payload.get("created").and_then(Value::as_i64) {
        state.created_at = created;
    }
    if let Some(usage) = payload.get("usage") {
        state.usage = Some(response_usage(Some(usage)));
    }

    let mut output = ensure_stream_started(state);
    for choice in payload
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                output.push_str(&emit_text_delta(state, text));
            }
        }
        for tool_call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            output.push_str(&emit_tool_delta(state, tool_call));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            state.finish_reason = Some(reason.to_string());
        }
    }
    output
}

fn ensure_stream_started(state: &mut ResponsesStreamState) -> String {
    if state.started {
        return String::new();
    }
    state.started = true;
    let response = stream_response_snapshot(state, "in_progress", Vec::new(), Value::Null);
    let mut output = String::new();
    push_event(
        state,
        &mut output,
        "response.created",
        json!({"response": response}),
    );
    let response = stream_response_snapshot(state, "in_progress", Vec::new(), Value::Null);
    push_event(
        state,
        &mut output,
        "response.in_progress",
        json!({"response": response}),
    );
    output
}

fn emit_text_delta(state: &mut ResponsesStreamState, text: &str) -> String {
    let mut output = String::new();
    if state.message.is_none() {
        let message = ResponseMessageState {
            item_id: format!("msg_{}", Uuid::new_v4().simple()),
            output_index: state.next_output_index,
            text: String::new(),
        };
        state.next_output_index += 1;
        push_event(
            state,
            &mut output,
            "response.output_item.added",
            json!({
                "output_index": message.output_index,
                "item": {"id": message.item_id, "type": "message", "status": "in_progress", "role": "assistant", "content": []}
            }),
        );
        push_event(
            state,
            &mut output,
            "response.content_part.added",
            json!({
                "item_id": message.item_id,
                "output_index": message.output_index,
                "content_index": 0,
                "part": {"type": "output_text", "text": "", "annotations": []}
            }),
        );
        state.message = Some(message);
    }
    let message = state.message.as_mut().unwrap();
    message.text.push_str(text);
    let item_id = message.item_id.clone();
    let output_index = message.output_index;
    push_event(
        state,
        &mut output,
        "response.output_text.delta",
        json!({"item_id": item_id, "output_index": output_index, "content_index": 0, "delta": text}),
    );
    output
}

fn emit_tool_delta(state: &mut ResponsesStreamState, tool_call: &Value) -> String {
    let index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let function = tool_call.get("function").unwrap_or(&Value::Null);
    let tool = state.tools.entry(index).or_default();
    if let Some(call_id) = tool_call.get("id").and_then(Value::as_str) {
        tool.call_id.push_str(call_id);
    }
    if let Some(name) = function.get("name").and_then(Value::as_str) {
        tool.name.push_str(name);
    }
    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
        tool.pending_arguments.push_str(arguments);
    }

    let mut output = String::new();
    if !tool.started && !tool.call_id.is_empty() && !tool.name.is_empty() {
        tool.started = true;
        tool.item_id = format!("fc_{}", Uuid::new_v4().simple());
        tool.output_index = state.next_output_index;
        state.next_output_index += 1;
        let item_id = tool.item_id.clone();
        let call_id = tool.call_id.clone();
        let name = tool.name.clone();
        let output_index = tool.output_index;
        push_event(
            state,
            &mut output,
            "response.output_item.added",
            json!({
                "output_index": output_index,
                "item": {"id": item_id, "type": "function_call", "status": "in_progress", "call_id": call_id, "name": name, "arguments": ""}
            }),
        );
    }

    let tool = state.tools.get_mut(&index).unwrap();
    if tool.started && !tool.pending_arguments.is_empty() {
        let delta = std::mem::take(&mut tool.pending_arguments);
        tool.arguments.push_str(&delta);
        let item_id = tool.item_id.clone();
        let output_index = tool.output_index;
        push_event(
            state,
            &mut output,
            "response.function_call_arguments.delta",
            json!({"item_id": item_id, "output_index": output_index, "delta": delta}),
        );
    }
    output
}

fn finalize_stream(state: &mut ResponsesStreamState) -> String {
    if state.terminal {
        return String::new();
    }
    let mut output = ensure_stream_started(state);
    let mut items = Vec::new();

    if let Some(message) = state.message.take() {
        push_event(
            state,
            &mut output,
            "response.output_text.done",
            json!({"item_id": message.item_id, "output_index": message.output_index, "content_index": 0, "text": message.text}),
        );
        let part = json!({"type": "output_text", "text": message.text, "annotations": []});
        push_event(
            state,
            &mut output,
            "response.content_part.done",
            json!({"item_id": message.item_id, "output_index": message.output_index, "content_index": 0, "part": part}),
        );
        let item = json!({
            "id": message.item_id, "type": "message", "status": "completed", "role": "assistant",
            "content": [part]
        });
        push_event(
            state,
            &mut output,
            "response.output_item.done",
            json!({"output_index": message.output_index, "item": item}),
        );
        items.push((message.output_index, item));
    }

    let tool_indexes = state.tools.keys().copied().collect::<Vec<_>>();
    for index in tool_indexes {
        let mut tool = state.tools.remove(&index).unwrap();
        if !tool.started {
            tool.started = true;
            tool.item_id = format!("fc_{}", Uuid::new_v4().simple());
            tool.output_index = state.next_output_index;
            state.next_output_index += 1;
        }
        if !tool.pending_arguments.is_empty() {
            tool.arguments.push_str(&tool.pending_arguments);
        }
        push_event(
            state,
            &mut output,
            "response.function_call_arguments.done",
            json!({"item_id": tool.item_id, "output_index": tool.output_index, "arguments": tool.arguments}),
        );
        let item = json!({
            "id": tool.item_id, "type": "function_call", "status": "completed",
            "call_id": tool.call_id, "name": tool.name, "arguments": tool.arguments
        });
        push_event(
            state,
            &mut output,
            "response.output_item.done",
            json!({"output_index": tool.output_index, "item": item}),
        );
        items.push((tool.output_index, item));
    }
    items.sort_by_key(|(index, _)| *index);
    let output_items = items.into_iter().map(|(_, item)| item).collect();
    let incomplete = state.finish_reason.as_deref() == Some("length");
    let status = if incomplete {
        "incomplete"
    } else {
        "completed"
    };
    let usage = state.usage.clone().unwrap_or_else(|| response_usage(None));
    let response = stream_response_snapshot(state, status, output_items, usage);
    push_event(
        state,
        &mut output,
        if incomplete {
            "response.incomplete"
        } else {
            "response.completed"
        },
        json!({"response": response}),
    );
    state.terminal = true;
    output
}

fn fail_stream(state: &mut ResponsesStreamState, code: &str, message: &str) -> String {
    if state.terminal {
        return String::new();
    }
    let mut output = ensure_stream_started(state);
    let mut response = stream_response_snapshot(state, "failed", Vec::new(), Value::Null);
    response["error"] = json!({"code": code, "message": message});
    push_event(
        state,
        &mut output,
        "response.failed",
        json!({"response": response}),
    );
    state.terminal = true;
    output
}

fn stream_response_snapshot(
    state: &ResponsesStreamState,
    status: &str,
    output: Vec<Value>,
    usage: Value,
) -> Value {
    json!({
        "id": state.response_id,
        "object": "response",
        "created_at": state.created_at,
        "status": status,
        "error": null,
        "incomplete_details": if status == "incomplete" { json!({"reason": "max_output_tokens"}) } else { Value::Null },
        "model": state.model,
        "output": output,
        "parallel_tool_calls": true,
        "usage": usage,
    })
}

fn push_event(state: &mut ResponsesStreamState, output: &mut String, kind: &str, data: Value) {
    let mut data = data.as_object().cloned().unwrap_or_else(Map::new);
    data.insert("type".to_string(), Value::String(kind.to_string()));
    data.insert("sequence_number".to_string(), json!(state.sequence));
    state.sequence = state.sequence.saturating_add(1);
    output.push_str("event: ");
    output.push_str(kind);
    output.push_str("\ndata: ");
    output.push_str(&serde_json::to_string(&Value::Object(data)).unwrap_or_default());
    output.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_maps_messages_function_history_and_flat_tools() {
        let canonical = OpenAiResponsesConverter
            .request_to_standard(json!({
                "model": "gpt-test",
                "instructions": "Be precise",
                "input": [
                    {"role": "user", "content": [{"type": "input_text", "text": "Read a.rs"}]},
                    {"type": "function_call", "call_id": "call-1", "name": "read_file", "arguments": "{\"path\":\"a.rs\"}"},
                    {"type": "function_call_output", "call_id": "call-1", "output": "contents"}
                ],
                "tools": [{"type": "function", "name": "read_file", "description": "Read", "parameters": {"type": "object"}, "strict": true}],
                "max_output_tokens": 100,
                "stream": true
            }))
            .unwrap();

        assert_eq!(canonical["messages"][0]["role"], "system");
        assert_eq!(canonical["messages"][2]["tool_calls"][0]["id"], "call-1");
        assert_eq!(canonical["messages"][3]["role"], "tool");
        assert_eq!(canonical["tools"][0]["function"]["name"], "read_file");
        assert_eq!(canonical["max_tokens"], 100);
        assert_eq!(canonical["stream_options"]["include_usage"], true);
    }

    #[test]
    fn request_rejects_provider_specific_built_in_tools() {
        let error = OpenAiResponsesConverter
            .request_to_standard(json!({
                "model": "gpt-test",
                "input": "search",
                "tools": [{"type": "web_search"}]
            }))
            .unwrap_err();
        assert!(error.to_string().contains("no provider-independent"));

        let error = OpenAiResponsesConverter
            .request_to_standard(json!({
                "model": "gpt-test",
                "previous_response_id": "resp-prior",
                "input": "continue"
            }))
            .unwrap_err();
        assert!(error.to_string().contains("stateful Responses store"));
    }

    #[test]
    fn response_maps_text_tools_usage_and_incomplete_status() {
        let response = OpenAiResponsesConverter
            .standard_to_response(json!({
                "id": "chatcmpl-1", "created": 10, "model": "gpt-test",
                "choices": [{"message": {"role": "assistant", "content": "Checking", "tool_calls": [{
                    "id": "call-1", "type": "function", "function": {"name": "read_file", "arguments": "{}"}
                }]}, "finish_reason": "length"}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}
            }))
            .unwrap();

        assert_eq!(response["object"], "response");
        assert_eq!(response["status"], "incomplete");
        assert_eq!(response["output"][0]["type"], "message");
        assert_eq!(response["output"][1]["type"], "function_call");
        assert_eq!(response["usage"]["total_tokens"], 15);
    }

    #[tokio::test]
    async fn stream_emits_responses_text_lifecycle_and_terminal_usage() {
        let raw = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"created\":10,\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"delta\":{\"content\":\"你\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\n",
            "data: [DONE]\n\n"
        );
        let split = raw.find("你").unwrap() + 1;
        let input = futures::stream::iter(vec![
            Ok(Bytes::copy_from_slice(&raw.as_bytes()[..split])),
            Ok(Bytes::copy_from_slice(&raw.as_bytes()[split..])),
        ]);
        let rendered = OpenAiResponsesConverter
            .standard_to_response_stream(Box::pin(input))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();

        assert!(rendered.contains("response.created"));
        assert!(rendered.contains("response.output_text.delta"));
        assert!(rendered.contains("你"));
        assert!(rendered.contains("response.completed"));
        assert!(rendered.contains("\"total_tokens\":5"));
    }

    #[tokio::test]
    async fn stream_maps_function_call_argument_deltas() {
        let raw = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let input = futures::stream::iter(vec![Ok(Bytes::from_static(raw.as_bytes()))]);
        let rendered = OpenAiResponsesConverter
            .standard_to_response_stream(Box::pin(input))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();

        assert!(rendered.contains("response.function_call_arguments.delta"));
        assert!(rendered.contains("response.function_call_arguments.done"));
        assert!(rendered.contains("read_file"));
        assert!(rendered.contains("response.completed"));
    }

    #[tokio::test]
    async fn stream_without_finish_reason_emits_failed_not_completed() {
        let raw = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n"
        );
        let input = futures::stream::iter(vec![Ok(Bytes::from_static(raw.as_bytes()))]);
        let rendered = OpenAiResponsesConverter
            .standard_to_response_stream(Box::pin(input))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();

        assert!(rendered.contains("response.failed"));
        assert!(!rendered.contains("event: response.completed"));
    }
}
