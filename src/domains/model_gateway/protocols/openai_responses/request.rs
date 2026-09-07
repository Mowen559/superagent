use serde_json::{json, Value};

pub(crate) fn responses_request_to_canonical(body: Value) -> Result<Value, anyhow::Error> {
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
