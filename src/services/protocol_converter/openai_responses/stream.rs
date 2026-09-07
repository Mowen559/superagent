use std::{collections::BTreeMap, io, pin::Pin};
use bytes::Bytes;
use futures::{stream::Stream, StreamExt};
use serde_json::{json, Map, Value};
use uuid::Uuid;
use super::super::sse::{append_utf8_safe, strip_sse_field, take_sse_block};
use super::response::response_usage;

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

pub(crate) fn canonical_stream_to_responses(
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
