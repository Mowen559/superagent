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

/// Resolve one capability fact through its evidence timestamp. Callers do not
/// parse storage fields or TTL configuration; that knowledge stays local to
/// the Provider Capability Runtime.
pub fn assess_model_profile(info: &Value, profile: CapabilityProfile) -> CapabilityAssessment {
    assess_model_profile_at(
        info,
        profile,
        Utc::now(),
        CapabilityEvidencePolicy::default(),
    )
}

pub fn assess_model_profile_at(
    info: &Value,
    profile: CapabilityProfile,
    now: DateTime<Utc>,
    policy: CapabilityEvidencePolicy,
) -> CapabilityAssessment {
    if let Some(evidence) = info.pointer(&format!("/capabilityEvidence/{}", profile.metadata_key()))
    {
        return assess_observation_evidence(evidence, now, policy);
    }

    match explicit_capability(info, profile) {
        Some(true) => CapabilityAssessment {
            verdict: CapabilityVerdict::Verified,
            freshness: CapabilityFreshness::Unversioned,
            source: None,
            expires_at: None,
        },
        Some(false) => CapabilityAssessment {
            verdict: CapabilityVerdict::Unsupported,
            freshness: CapabilityFreshness::Unversioned,
            source: None,
            expires_at: None,
        },
        None => unknown_assessment(CapabilityFreshness::Missing, None, None),
    }
}

pub fn assess_context_limit(info: &Value, min_context_tokens: usize) -> CapabilityAssessment {
    assess_context_limit_at(
        info,
        min_context_tokens,
        Utc::now(),
        CapabilityEvidencePolicy::default(),
    )
}

pub fn assess_context_limit_at(
    info: &Value,
    min_context_tokens: usize,
    now: DateTime<Utc>,
    policy: CapabilityEvidencePolicy,
) -> CapabilityAssessment {
    let context_limit = info
        .get("inputTokenLimit")
        .or_else(|| info.get("context_length"))
        .or_else(|| info.get("context_window"))
        .or_else(|| info.get("max_position_embeddings"))
        .and_then(Value::as_u64)
        .map(|limit| limit.min(usize::MAX as u64) as usize);

    let (freshness, source, expires_at) = if let Some(evidence) = info.get("catalogEvidence") {
        match assess_evidence_timing(evidence, CapabilityEvidenceSource::Catalog, now, policy) {
            Ok(timing) => timing,
            Err(()) => {
                return unknown_assessment(
                    CapabilityFreshness::Invalid,
                    Some(CapabilityEvidenceSource::Catalog),
                    None,
                )
            }
        }
    } else {
        (CapabilityFreshness::Unversioned, None, None)
    };
    if freshness == CapabilityFreshness::Expired {
        return unknown_assessment(freshness, source, expires_at);
    }

    match context_limit {
        Some(limit) if limit < min_context_tokens => CapabilityAssessment {
            verdict: CapabilityVerdict::Unsupported,
            freshness,
            source,
            expires_at,
        },
        Some(_) => CapabilityAssessment {
            verdict: CapabilityVerdict::Verified,
            freshness,
            source,
            expires_at,
        },
        None => unknown_assessment(CapabilityFreshness::Missing, source, expires_at),
    }
}

pub fn plan_recertification_for_account(account: &LLMAccount) -> RecertificationPlan {
    plan_recertification_for_account_at(account, Utc::now(), CapabilityEvidencePolicy::default())
}

pub(crate) fn plan_recertification_for_account_at(
    account: &LLMAccount,
    now: DateTime<Utc>,
    policy: CapabilityEvidencePolicy,
) -> RecertificationPlan {
    let mut model_ids = account
        .models_info
        .as_ref()
        .map(|models| models.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    model_ids.sort();
    let known_models = account.models.iter().collect::<HashSet<_>>();
    let mut items = Vec::new();
    let mut probe_requests = 0usize;
    let mut truncated = false;

    for model in model_ids {
        if !known_models.contains(&model) {
            continue;
        }
        let info = &account.models_info.as_ref().expect("checked above")[&model];
        let mut profiles = Vec::new();
        for profile in [
            CapabilityProfile::Basic,
            CapabilityProfile::Tools,
            CapabilityProfile::Vision,
            CapabilityProfile::Streaming,
        ] {
            let has_evidence = info
                .pointer(&format!("/capabilityEvidence/{}", profile.metadata_key()))
                .is_some();
            if !has_evidence
                || assess_model_profile_at(info, profile, now, policy).freshness
                    != CapabilityFreshness::Expired
            {
                continue;
            }
            if items.len() >= MAX_SELECTED_MODELS || probe_requests >= MAX_PROBE_REQUESTS {
                truncated = true;
                continue;
            }
            profiles.push(profile);
            probe_requests += 1;
        }
        if !profiles.is_empty() {
            items.push(RecertificationItem { model, profiles });
        }
    }

    RecertificationPlan {
        account_id: account.id.clone(),
        generated_at: now.to_rfc3339(),
        items,
        probe_requests,
        truncated,
    }
}

pub(crate) fn assess_observation_evidence(
    evidence: &Value,
    now: DateTime<Utc>,
    policy: CapabilityEvidencePolicy,
) -> CapabilityAssessment {
    let source = evidence
        .get("source")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    let outcome = evidence
        .get("outcome")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    let (Some(source), Some(outcome)) = (source, outcome) else {
        return unknown_assessment(CapabilityFreshness::Invalid, source, None);
    };
    let Ok((freshness, _, expires_at)) = assess_evidence_timing(evidence, source, now, policy)
    else {
        return unknown_assessment(CapabilityFreshness::Invalid, Some(source), None);
    };
    if freshness == CapabilityFreshness::Expired {
        return unknown_assessment(freshness, Some(source), expires_at);
    }

    let verdict = match outcome {
        CapabilityObservationOutcome::Verified => CapabilityVerdict::Verified,
        CapabilityObservationOutcome::Unsupported => CapabilityVerdict::Unsupported,
        CapabilityObservationOutcome::Unknown | CapabilityObservationOutcome::Failed => {
            CapabilityVerdict::Unknown
        }
    };
    CapabilityAssessment {
        verdict,
        freshness,
        source: Some(source),
        expires_at,
    }
}

pub(crate) fn assess_evidence_timing(
    evidence: &Value,
    source: CapabilityEvidenceSource,
    now: DateTime<Utc>,
    policy: CapabilityEvidencePolicy,
) -> Result<
    (
        CapabilityFreshness,
        Option<CapabilityEvidenceSource>,
        Option<String>,
    ),
    (),
> {
    let observed_at = evidence
        .get("observedAt")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
        .ok_or(())?;
    let expires_at = match evidence.get("expiresAt").and_then(Value::as_str) {
        Some(value) => parse_timestamp(value).ok_or(())?,
        None => observed_at + policy.ttl_for(source),
    };
    Ok((
        if now >= expires_at {
            CapabilityFreshness::Expired
        } else {
            CapabilityFreshness::Fresh
        },
        Some(source),
        Some(expires_at.to_rfc3339()),
    ))
}

pub(crate) fn unknown_assessment(
    freshness: CapabilityFreshness,
    source: Option<CapabilityEvidenceSource>,
    expires_at: Option<String>,
) -> CapabilityAssessment {
    CapabilityAssessment {
        verdict: CapabilityVerdict::Unknown,
        freshness,
        source,
        expires_at,
    }
}

pub(crate) fn observation(
    account: &LLMAccount,
    model: &str,
    profile: CapabilityProfile,
    source: CapabilityEvidenceSource,
    outcome: CapabilityObservationOutcome,
    observed_at: DateTime<Utc>,
    detail: impl Into<String>,
    status_code: Option<u16>,
    policy: CapabilityEvidencePolicy,
) -> CapabilityObservation {
    CapabilityObservation {
        account_id: account.id.clone(),
        model: model.to_string(),
        profile,
        source,
        outcome,
        observed_at: observed_at.to_rfc3339(),
        expires_at: policy.expires_at(observed_at, source),
        detail: detail.into(),
        status_code,
    }
}

pub(crate) fn explicit_capability(metadata: &Value, profile: CapabilityProfile) -> Option<bool> {
    let direct = match profile {
        CapabilityProfile::Basic => capability_bool(metadata, &["supportsBasic"], "basic"),
        CapabilityProfile::Tools => capability_bool(
            metadata,
            &[
                "supportsTools",
                "supports_tools",
                "toolCalling",
                "tool_calling",
            ],
            "tools",
        ),
        CapabilityProfile::Vision => capability_bool(
            metadata,
            &["supportsVision", "supports_vision", "vision"],
            "vision",
        ),
        CapabilityProfile::Streaming => capability_bool(
            metadata,
            &["supportsStreaming", "supports_streaming", "streaming"],
            "streaming",
        ),
    };
    if direct.is_some() {
        return direct;
    }
    let methods = metadata
        .get("supportedGenerationMethods")
        .and_then(Value::as_array)?;
    match profile {
        CapabilityProfile::Basic => {
            Some(methods.iter().any(|method| {
                matches!(method.as_str(), Some("generateContent" | "chatCompletions"))
            }))
        }
        CapabilityProfile::Streaming => Some(
            methods
                .iter()
                .any(|method| method.as_str() == Some("streamGenerateContent")),
        ),
        _ => None,
    }
}

pub(crate) fn capability_bool(metadata: &Value, keys: &[&str], nested_key: &str) -> Option<bool> {
    let values = keys
        .iter()
        .filter_map(|key| metadata.get(*key))
        .chain(
            metadata
                .pointer(&format!("/capabilities/{nested_key}"))
                .into_iter(),
        )
        .filter_map(Value::as_bool)
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value) {
        Some(false)
    } else if values.iter().any(|value| *value) {
        Some(true)
    } else {
        None
    }
}

pub(crate) fn contains_key(value: &Value, keys: &[&str]) -> bool {
    match value {
        Value::Array(values) => values.iter().any(|value| contains_key(value, keys)),
        Value::Object(values) => {
            values.keys().any(|key| keys.contains(&key.as_str()))
                || values.values().any(|value| contains_key(value, keys))
        }
        _ => false,
    }
}

