use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::config::llm_settings::{LLMAccount, LLMSettings};
use crate::services::protocol_converter::upstream::adapter_for_provider;

const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const MAX_CATALOG_MODELS: usize = 256;
const MAX_MODEL_ID_CHARS: usize = 256;
const MAX_SELECTED_MODELS: usize = 4;
const MAX_PROFILES: usize = 4;
const MAX_PROBE_REQUESTS: usize = 8;
const MAX_PROBE_BYTES: usize = 128 * 1024;
const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
const PROBE_MAX_TOKENS: u32 = 8;
const DEFAULT_CATALOG_TTL_HOURS: i64 = 24;
const DEFAULT_PROBE_TTL_HOURS: i64 = 7 * 24;
const MIN_EVIDENCE_TTL_HOURS: i64 = 1;
const MAX_EVIDENCE_TTL_HOURS: i64 = 90 * 24;
const TINY_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";


use super::*;

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

