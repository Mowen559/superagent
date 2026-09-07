use std::collections::HashSet;
use chrono::{DateTime, Utc};
use reqwest::header::HeaderValue;
use serde_json::Value;

use crate::config::llm_settings::{LLMAccount, LLMSettings};
use super::policy::{MIN_EVIDENCE_TTL_HOURS, MAX_EVIDENCE_TTL_HOURS, PROBE_MAX_TOKENS};
use super::discovery::{CapabilityDiscoveryError, CapabilityDiscoveryErrorKind};

pub(crate) fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

pub(crate) fn ttl_hours_from_env(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default)
        .clamp(MIN_EVIDENCE_TTL_HOURS, MAX_EVIDENCE_TTL_HOURS)
}

/// Merge a completed report into the latest settings snapshot. Transient probe
/// failures remain report-only and never downgrade previously verified facts.
pub(crate) fn account_settings(account: &LLMAccount, model: &str) -> LLMSettings {
    LLMSettings {
        enabled: true,
        active_account_id: Some(account.id.clone()),
        accounts: vec![account.clone()],
        provider: account.provider.clone(),
        base_url: account.base_url.clone(),
        api_key: account.api_key.clone(),
        project_id: account.project_id.clone(),
        model: model.to_string(),
        temperature: 0.0,
        max_tokens: PROBE_MAX_TOKENS,
        ..LLMSettings::default()
    }
}

pub(crate) fn deduplicate_trimmed(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(crate) fn header_value(
    value: &str,
    message: &'static str,
) -> Result<HeaderValue, CapabilityDiscoveryError> {
    HeaderValue::from_str(value).map_err(|_| {
        CapabilityDiscoveryError::new(CapabilityDiscoveryErrorKind::InvalidRequest, message)
    })
}

pub(crate) fn merge_metadata(target: &mut Value, discovered: &Value) {
    let Some(target) = target.as_object_mut() else {
        *target = discovered.clone();
        return;
    };
    if let Some(discovered) = discovered.as_object() {
        for (key, value) in discovered {
            target.insert(key.clone(), value.clone());
        }
    }
}
