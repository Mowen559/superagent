use serde_json::{json, Value};
use uuid::Uuid;

pub(crate) fn response_usage(usage: Option<&Value>) -> Value {
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

pub(crate) fn canonical_response_to_responses(body: Value) -> Result<Value, anyhow::Error> {
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

