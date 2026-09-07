use chrono::Utc;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use crate::config::llm_settings::LLMAccount;
use crate::domains::model_gateway::protocols::upstream::adapter_for_provider;
use super::policy::{CapabilityEvidencePolicy, MAX_PROBE_BYTES, PROBE_MAX_TOKENS, TINY_PNG};
use super::types::{CapabilityEvidenceSource, CapabilityObservation, CapabilityObservationOutcome, CapabilityProfile};
use super::discovery::{CapabilityDiscoveryError, CapabilityDiscoveryErrorKind};
use super::assessment::contains_key;
use super::utils::{account_settings, header_value};

#[derive(Debug)]
pub(crate) struct ProbeFailure {
    pub(crate) outcome: CapabilityObservationOutcome,
    pub(crate) detail: String,
    pub(crate) status_code: Option<u16>,
}

impl ProbeFailure {
    pub(crate) fn failed(detail: impl Into<String>) -> Self {
        Self {
            outcome: CapabilityObservationOutcome::Failed,
            detail: detail.into(),
            status_code: None,
        }
    }
}

pub(crate) async fn read_bounded(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, CapabilityDiscoveryError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            CapabilityDiscoveryError::new(
                CapabilityDiscoveryErrorKind::CatalogRequestFailed,
                "model catalog response stream failed",
            )
        })?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(CapabilityDiscoveryError::new(
                CapabilityDiscoveryErrorKind::CatalogResponseInvalid,
                "model catalog response exceeded the byte limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) async fn read_probe_body(response: reqwest::Response) -> Result<Vec<u8>, ProbeFailure> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ProbeFailure::failed("probe_response_failed"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_PROBE_BYTES {
            return Err(ProbeFailure::failed("probe_response_too_large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn probe_body(model: &str, profile: CapabilityProfile) -> Value {
    let mut body = json!({
        "model": model,
        "messages": [{"role": "user", "content": "Reply with OK."}],
        "temperature": 0,
        "max_tokens": PROBE_MAX_TOKENS,
        "stream": profile == CapabilityProfile::Streaming,
    });
    if profile == CapabilityProfile::Tools {
        body["messages"] = json!([{
            "role": "user",
            "content": "Call the hub_capability_probe tool exactly once with value OK."
        }]);
        body["tools"] = json!([{
            "type": "function",
            "function": {
                "name": "hub_capability_probe",
                "description": "Return the fixed capability probe value.",
                "parameters": {
                    "type": "object",
                    "properties": {"value": {"type": "string", "enum": ["OK"]}},
                    "required": ["value"],
                    "additionalProperties": false
                }
            }
        }]);
        body["tool_choice"] = json!("required");
    } else if profile == CapabilityProfile::Vision {
        body["messages"] = json!([{
            "role": "user",
            "content": [
                {"type": "text", "text": "Reply with OK after inspecting this image."},
                {"type": "image_url", "image_url": {"url": TINY_PNG}}
            ]
        }]);
    }
    body
}

pub(crate) fn validate_probe_response(
    response: &Value,
    profile: CapabilityProfile,
) -> Result<String, ProbeFailure> {
    let message = response
        .pointer("/choices/0/message")
        .ok_or_else(|| ProbeFailure::failed("canonical_response_message_missing"))?;
    if profile == CapabilityProfile::Tools {
        let tool_seen = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| !calls.is_empty());
        return if tool_seen {
            Ok("canonical_tool_call_observed".to_string())
        } else {
            Err(ProbeFailure::failed("canonical_tool_call_missing"))
        };
    }
    let content_seen = message.get("content").is_some_and(|content| match content {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => false,
    });
    if content_seen {
        Ok(if profile == CapabilityProfile::Vision {
            "canonical_vision_response_observed"
        } else {
            "canonical_text_response_observed"
        }
        .to_string())
    } else {
        Err(ProbeFailure::failed("canonical_response_content_missing"))
    }
}

pub(crate) fn validate_adapted_probe(
    url: &str,
    body: &Value,
    profile: CapabilityProfile,
) -> Result<(), ProbeFailure> {
    let preserved = match profile {
        CapabilityProfile::Basic => true,
        CapabilityProfile::Tools => contains_key(body, &["tools", "functionDeclarations"]),
        CapabilityProfile::Vision => {
            contains_key(body, &["image_url", "image", "inlineData", "inline_data"])
        }
        CapabilityProfile::Streaming => {
            body.get("stream").and_then(Value::as_bool) == Some(true) || url.contains("alt=sse")
        }
    };
    if preserved {
        Ok(())
    } else {
        Err(ProbeFailure::failed("capability_lost_during_adaptation"))
    }
}

