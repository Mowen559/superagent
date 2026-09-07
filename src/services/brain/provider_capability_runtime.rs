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

fn default_catalog_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDiscoveryRequest {
    pub account_id: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub profiles: Vec<CapabilityProfile>,
    #[serde(default = "default_catalog_enabled")]
    pub include_catalog: bool,
    #[serde(default)]
    pub run_probes: bool,
    #[serde(default)]
    pub confirm_network: bool,
    #[serde(default)]
    pub revalidate_expired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProfile {
    Basic,
    Tools,
    Vision,
    Streaming,
}

impl CapabilityProfile {
    fn metadata_key(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Tools => "tools",
            Self::Vision => "vision",
            Self::Streaming => "streaming",
        }
    }

    fn from_metadata_key(value: &str) -> Option<Self> {
        match value {
            "basic" => Some(Self::Basic),
            "tools" => Some(Self::Tools),
            "vision" => Some(Self::Vision),
            "streaming" => Some(Self::Streaming),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvidenceSource {
    Catalog,
    Probe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityObservationOutcome {
    Verified,
    Unknown,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityVerdict {
    Verified,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFreshness {
    Fresh,
    Expired,
    Unversioned,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityAssessment {
    pub verdict: CapabilityVerdict,
    pub freshness: CapabilityFreshness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<CapabilityEvidenceSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct CapabilityEvidencePolicy {
    pub catalog_ttl: ChronoDuration,
    pub probe_ttl: ChronoDuration,
}

impl Default for CapabilityEvidencePolicy {
    fn default() -> Self {
        Self {
            catalog_ttl: ChronoDuration::hours(ttl_hours_from_env(
                "HUB_CAPABILITY_CATALOG_TTL_HOURS",
                DEFAULT_CATALOG_TTL_HOURS,
            )),
            probe_ttl: ChronoDuration::hours(ttl_hours_from_env(
                "HUB_CAPABILITY_PROBE_TTL_HOURS",
                DEFAULT_PROBE_TTL_HOURS,
            )),
        }
    }
}

impl CapabilityEvidencePolicy {
    fn ttl_for(self, source: CapabilityEvidenceSource) -> ChronoDuration {
        match source {
            CapabilityEvidenceSource::Catalog => self.catalog_ttl,
            CapabilityEvidenceSource::Probe => self.probe_ttl,
        }
    }

    fn expires_at(self, observed_at: DateTime<Utc>, source: CapabilityEvidenceSource) -> String {
        (observed_at + self.ttl_for(source)).to_rfc3339()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityObservation {
    pub account_id: String,
    pub model: String,
    pub profile: CapabilityProfile,
    pub source: CapabilityEvidenceSource,
    pub outcome: CapabilityObservationOutcome,
    pub observed_at: String,
    pub expires_at: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogSnapshot {
    pub account_id: String,
    pub provider: String,
    pub observed_at: String,
    pub expires_at: String,
    pub models: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDiscoveryReport {
    pub account_id: String,
    pub provider: String,
    pub catalog: Option<ModelCatalogSnapshot>,
    pub observations: Vec<CapabilityObservation>,
    pub probe_requests: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recertification_plan: Option<RecertificationPlan>,
    #[serde(skip_serializing)]
    catalog_metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecertificationItem {
    pub model: String,
    pub profiles: Vec<CapabilityProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecertificationPlan {
    pub account_id: String,
    pub generated_at: String,
    pub items: Vec<RecertificationItem>,
    pub probe_requests: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDiscoveryErrorKind {
    InvalidRequest,
    AccountNotFound,
    UnsupportedProvider,
    CatalogRequestFailed,
    CatalogResponseInvalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDiscoveryError {
    pub kind: CapabilityDiscoveryErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
}

impl CapabilityDiscoveryError {
    fn new(kind: CapabilityDiscoveryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status_code: None,
        }
    }

    fn http(kind: CapabilityDiscoveryErrorKind, status_code: u16) -> Self {
        Self {
            kind,
            message: format!("provider returned HTTP {status_code}"),
            status_code: Some(status_code),
        }
    }
}

impl fmt::Display for CapabilityDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CapabilityDiscoveryError {}

/// Deep Module owning bounded model discovery, fixed active probes and safe
/// evidence normalization. Its Interface accepts only an existing account ID;
/// URLs, credentials and provider request bodies remain in the Implementation.
#[derive(Clone)]
pub struct ProviderCapabilityRuntime {
    client: reqwest::Client,
    evidence_policy: CapabilityEvidencePolicy,
}

impl ProviderCapabilityRuntime {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            evidence_policy: CapabilityEvidencePolicy::default(),
        }
    }

    #[cfg(test)]
    fn with_policy(client: reqwest::Client, evidence_policy: CapabilityEvidencePolicy) -> Self {
        Self {
            client,
            evidence_policy,
        }
    }

    pub async fn discover(
        &self,
        settings: &LLMSettings,
        request: CapabilityDiscoveryRequest,
    ) -> Result<CapabilityDiscoveryReport, CapabilityDiscoveryError> {
        let request = validate_request(request)?;
        let account = settings
            .accounts
            .iter()
            .find(|account| account.id == request.account_id)
            .ok_or_else(|| {
                CapabilityDiscoveryError::new(
                    CapabilityDiscoveryErrorKind::AccountNotFound,
                    "provider account was not found",
                )
            })?;
        validate_account(account)?;

        let mut report = CapabilityDiscoveryReport {
            account_id: account.id.clone(),
            provider: account.provider.clone(),
            catalog: None,
            observations: Vec::new(),
            probe_requests: 0,
            recertification_plan: None,
            catalog_metadata: HashMap::new(),
        };

        if request.include_catalog {
            let catalog = self.fetch_catalog(account).await?;
            report.observations.extend(catalog.observations);
            report.catalog_metadata = catalog.metadata;
            report.catalog = Some(catalog.snapshot);
        }

        if request.run_probes {
            let probe_pairs = if request.revalidate_expired {
                let plan =
                    plan_recertification_for_account_at(account, Utc::now(), self.evidence_policy);
                let pairs = plan
                    .items
                    .iter()
                    .flat_map(|item| {
                        item.profiles
                            .iter()
                            .map(|profile| (item.model.clone(), *profile))
                    })
                    .collect::<Vec<_>>();
                report.recertification_plan = Some(plan);
                pairs
            } else {
                let known_models = known_models(account, report.catalog.as_ref());
                for model in &request.models {
                    if !known_models.contains(model) {
                        return Err(CapabilityDiscoveryError::new(
                            CapabilityDiscoveryErrorKind::InvalidRequest,
                            format!("model '{model}' is not present in the configured or discovered catalog"),
                        ));
                    }
                }
                request
                    .models
                    .iter()
                    .flat_map(|model| {
                        request
                            .profiles
                            .iter()
                            .map(|profile| (model.clone(), *profile))
                    })
                    .collect::<Vec<_>>()
            };

            for (model, profile) in probe_pairs {
                report
                    .observations
                    .push(self.probe(account, &model, profile).await);
                report.probe_requests += 1;
            }
        }

        Ok(report)
    }

    async fn fetch_catalog(
        &self,
        account: &LLMAccount,
    ) -> Result<DiscoveredCatalog, CapabilityDiscoveryError> {
        tokio::time::timeout(CATALOG_TIMEOUT, self.fetch_catalog_inner(account))
            .await
            .map_err(|_| {
                CapabilityDiscoveryError::new(
                    CapabilityDiscoveryErrorKind::CatalogRequestFailed,
                    "model catalog request timed out",
                )
            })?
    }

    async fn fetch_catalog_inner(
        &self,
        account: &LLMAccount,
    ) -> Result<DiscoveredCatalog, CapabilityDiscoveryError> {
        let adapter = CatalogAdapter::for_account(account)?;
        let (url, headers) = adapter.prepare(account)?;
        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|_| {
                CapabilityDiscoveryError::new(
                    CapabilityDiscoveryErrorKind::CatalogRequestFailed,
                    "model catalog request failed",
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(CapabilityDiscoveryError::http(
                CapabilityDiscoveryErrorKind::CatalogRequestFailed,
                status.as_u16(),
            ));
        }
        let bytes = read_bounded(response, MAX_CATALOG_BYTES).await?;
        let payload: Value = serde_json::from_slice(&bytes).map_err(|_| {
            CapabilityDiscoveryError::new(
                CapabilityDiscoveryErrorKind::CatalogResponseInvalid,
                "model catalog was not valid JSON",
            )
        })?;
        normalize_catalog(account, payload, self.evidence_policy)
    }

    async fn probe(
        &self,
        account: &LLMAccount,
        model: &str,
        profile: CapabilityProfile,
    ) -> CapabilityObservation {
        let observed_at = Utc::now();
        let result =
            tokio::time::timeout(PROBE_TIMEOUT, self.probe_inner(account, model, profile)).await;

        match result {
            Ok(Ok(detail)) => observation(
                account,
                model,
                profile,
                CapabilityEvidenceSource::Probe,
                CapabilityObservationOutcome::Verified,
                observed_at,
                detail,
                Some(200),
                self.evidence_policy,
            ),
            Ok(Err(failure)) => observation(
                account,
                model,
                profile,
                CapabilityEvidenceSource::Probe,
                failure.outcome,
                observed_at,
                failure.detail,
                failure.status_code,
                self.evidence_policy,
            ),
            Err(_) => observation(
                account,
                model,
                profile,
                CapabilityEvidenceSource::Probe,
                CapabilityObservationOutcome::Failed,
                observed_at,
                "probe_timeout",
                None,
                self.evidence_policy,
            ),
        }
    }

    async fn probe_inner(
        &self,
        account: &LLMAccount,
        model: &str,
        profile: CapabilityProfile,
    ) -> Result<String, ProbeFailure> {
        if account.provider.eq_ignore_ascii_case("gemini_native") {
            return Err(ProbeFailure::failed("provider_probe_not_supported"));
        }

        let settings = account_settings(account, model);
        let canonical = probe_body(model, profile);
        let adapter = adapter_for_provider(&settings);
        let (url, headers, body) = adapter
            .prepare_request(&settings, canonical)
            .map_err(|_| ProbeFailure::failed("request_adaptation_failed"))?;
        validate_adapted_probe(&url, &body, profile)?;
        let response = self
            .client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|_| ProbeFailure::failed("provider_request_failed"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProbeFailure {
                // A status code without provider-body inspection is not strong
                // enough evidence to distinguish an unsupported capability
                // from a deployment/path/policy mismatch.
                outcome: CapabilityObservationOutcome::Failed,
                detail: "provider_http_failure".to_string(),
                status_code: Some(status.as_u16()),
            });
        }

        if profile == CapabilityProfile::Streaming {
            let source = response
                .bytes_stream()
                .map(|result| result.map_err(|error| io::Error::new(io::ErrorKind::Other, error)));
            let mut stream = adapter.response_stream_to_standard(Box::pin(source));
            let mut total = 0usize;
            let mut observed_data = false;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| ProbeFailure::failed("stream_decode_failed"))?;
                total = total.saturating_add(chunk.len());
                if total > MAX_PROBE_BYTES {
                    return Err(ProbeFailure::failed("probe_response_too_large"));
                }
                if String::from_utf8_lossy(&chunk).contains("data:") {
                    observed_data = true;
                    break;
                }
            }
            return if observed_data {
                Ok("canonical_stream_chunk_observed".to_string())
            } else {
                Err(ProbeFailure::failed("canonical_stream_chunk_missing"))
            };
        }

        let bytes = read_probe_body(response).await?;
        let upstream: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ProbeFailure::failed("provider_response_invalid"))?;
        let standard = adapter
            .response_to_standard(upstream)
            .map_err(|_| ProbeFailure::failed("response_adaptation_failed"))?;
        validate_probe_response(&standard, profile)
    }
}

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

fn plan_recertification_for_account_at(
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

fn assess_observation_evidence(
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

fn assess_evidence_timing(
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

fn unknown_assessment(
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

fn observation(
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

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn ttl_hours_from_env(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default)
        .clamp(MIN_EVIDENCE_TTL_HOURS, MAX_EVIDENCE_TTL_HOURS)
}

/// Merge a completed report into the latest settings snapshot. Transient probe
/// failures remain report-only and never downgrade previously verified facts.
pub fn apply_discovery_report(
    settings: &mut LLMSettings,
    report: &CapabilityDiscoveryReport,
) -> Result<(), String> {
    let account = settings
        .accounts
        .iter_mut()
        .find(|account| account.id == report.account_id)
        .ok_or_else(|| "provider account disappeared before capability commit".to_string())?;

    if let Some(catalog) = &report.catalog {
        let mut models = account.models.iter().cloned().collect::<HashSet<_>>();
        models.extend(catalog.models.iter().cloned());
        account.models = models.into_iter().collect();
        account.models.sort();
        let info = account.models_info.get_or_insert_with(HashMap::new);
        for (model, metadata) in &report.catalog_metadata {
            let model_info = info.entry(model.clone()).or_insert_with(|| json!({}));
            merge_metadata(model_info, metadata);
            let object = model_info
                .as_object_mut()
                .ok_or_else(|| format!("model metadata for '{model}' is not an object"))?;
            object.insert(
                "catalogEvidence".to_string(),
                json!({
                    "source": CapabilityEvidenceSource::Catalog,
                    "observedAt": catalog.observed_at,
                    "expiresAt": catalog.expires_at,
                }),
            );
        }
    }

    let info = account.models_info.get_or_insert_with(HashMap::new);
    for observation in &report.observations {
        if !matches!(
            observation.outcome,
            CapabilityObservationOutcome::Verified | CapabilityObservationOutcome::Unsupported
        ) {
            continue;
        }
        let model_info = info
            .entry(observation.model.clone())
            .or_insert_with(|| json!({}));
        let object = model_info.as_object_mut().ok_or_else(|| {
            format!(
                "model metadata for '{}' is not an object",
                observation.model
            )
        })?;
        let capability = object.entry("capabilities").or_insert_with(|| json!({}));
        let capability = capability
            .as_object_mut()
            .ok_or_else(|| "model capabilities metadata is not an object".to_string())?;
        capability.insert(
            observation.profile.metadata_key().to_string(),
            Value::Bool(observation.outcome == CapabilityObservationOutcome::Verified),
        );

        let evidence = object
            .entry("capabilityEvidence")
            .or_insert_with(|| json!({}));
        let evidence = evidence
            .as_object_mut()
            .ok_or_else(|| "model capability evidence is not an object".to_string())?;
        evidence.insert(
            observation.profile.metadata_key().to_string(),
            json!({
                "source": observation.source,
                "outcome": observation.outcome,
                "observedAt": observation.observed_at,
                "expiresAt": observation.expires_at,
                "detail": observation.detail,
                "statusCode": observation.status_code,
            }),
        );
    }
    Ok(())
}

#[derive(Debug)]
struct DiscoveredCatalog {
    snapshot: ModelCatalogSnapshot,
    metadata: HashMap<String, Value>,
    observations: Vec<CapabilityObservation>,
}

#[derive(Debug)]
struct ProbeFailure {
    outcome: CapabilityObservationOutcome,
    detail: String,
    status_code: Option<u16>,
}

impl ProbeFailure {
    fn failed(detail: impl Into<String>) -> Self {
        Self {
            outcome: CapabilityObservationOutcome::Failed,
            detail: detail.into(),
            status_code: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CatalogAdapter {
    OpenAiCompatible,
    Anthropic,
    Gemini,
}

impl CatalogAdapter {
    fn for_account(account: &LLMAccount) -> Result<Self, CapabilityDiscoveryError> {
        let provider = account.provider.to_ascii_lowercase();
        if provider == "anthropic" || provider == "claude" {
            Ok(Self::Anthropic)
        } else if provider == "gemini" || provider == "google" {
            Ok(Self::Gemini)
        } else if provider == "gemini_native" {
            Err(CapabilityDiscoveryError::new(
                CapabilityDiscoveryErrorKind::UnsupportedProvider,
                "gemini_native does not expose an account-scoped catalog endpoint",
            ))
        } else {
            Ok(Self::OpenAiCompatible)
        }
    }

    fn prepare(
        self,
        account: &LLMAccount,
    ) -> Result<(String, HeaderMap), CapabilityDiscoveryError> {
        let base = account.base_url.trim_end_matches('/');
        let mut headers = HeaderMap::new();
        if base.ends_with("/backend-api/codex") {
            headers.insert(
                USER_AGENT,
                HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36"),
            );
        } else {
            headers.insert(
                USER_AGENT,
                HeaderValue::from_static("hub-rs/capability-discovery"),
            );
        }
        match self {
            Self::Anthropic => {
                headers.insert(
                    "x-api-key",
                    header_value(&account.api_key, "invalid provider API key")?,
                );
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
                let url = if base.ends_with("/v1") {
                    format!("{base}/models")
                } else {
                    format!("{base}/v1/models")
                };
                Ok((url, headers))
            }
            Self::Gemini => {
                if account.api_key.starts_with("ya29.") {
                    headers.insert(
                        AUTHORIZATION,
                        header_value(
                            &format!("Bearer {}", account.api_key),
                            "invalid provider access token",
                        )?,
                    );
                } else {
                    headers.insert(
                        "x-goog-api-key",
                        header_value(&account.api_key, "invalid provider API key")?,
                    );
                }
                let native_base = base.strip_suffix("/openai").unwrap_or(base);
                let url = if native_base.ends_with("/v1beta") {
                    format!("{native_base}/models")
                } else {
                    format!("{native_base}/v1beta/models")
                };
                Ok((url, headers))
            }
            Self::OpenAiCompatible => {
                headers.insert(
                    AUTHORIZATION,
                    header_value(
                        &format!("Bearer {}", account.api_key),
                        "invalid provider API key",
                    )?,
                );

                let url = if base.ends_with("/backend-api/codex") {
                    base.replace("/backend-api/codex", "/backend-api/models")
                } else if base.ends_with("/v1") {
                    format!("{base}/models")
                } else {
                    format!("{base}/v1/models")
                };
                Ok((url, headers))
            }
        }
    }
}

fn validate_request(
    mut request: CapabilityDiscoveryRequest,
) -> Result<CapabilityDiscoveryRequest, CapabilityDiscoveryError> {
    request.account_id = request.account_id.trim().to_string();
    request.models = deduplicate_trimmed(request.models);
    let mut seen_profiles = HashSet::new();
    request
        .profiles
        .retain(|profile| seen_profiles.insert(*profile));

    if !request.confirm_network {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "confirmNetwork must be true for external capability discovery",
        ));
    }
    if request.account_id.is_empty() {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "accountId is required",
        ));
    }
    if !request.include_catalog && !request.run_probes {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "includeCatalog or runProbes must be enabled",
        ));
    }
    if request.models.len() > MAX_SELECTED_MODELS
        || request
            .models
            .iter()
            .any(|model| model.is_empty() || model.chars().count() > MAX_MODEL_ID_CHARS)
    {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            format!("at most {MAX_SELECTED_MODELS} bounded model IDs may be selected"),
        ));
    }
    if request.profiles.len() > MAX_PROFILES {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            format!("at most {MAX_PROFILES} capability profiles may be selected"),
        ));
    }
    if request.revalidate_expired && !request.run_probes {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "revalidateExpired requires runProbes",
        ));
    }
    if request.revalidate_expired && (!request.models.is_empty() || !request.profiles.is_empty()) {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "revalidateExpired selects models and profiles automatically",
        ));
    }
    if request.run_probes
        && !request.revalidate_expired
        && (request.models.is_empty() || request.profiles.is_empty())
    {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "runProbes requires explicit models and profiles",
        ));
    }
    if request.models.len().saturating_mul(request.profiles.len()) > MAX_PROBE_REQUESTS {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            format!("a discovery request may issue at most {MAX_PROBE_REQUESTS} probes"),
        ));
    }
    Ok(request)
}

fn validate_account(account: &LLMAccount) -> Result<(), CapabilityDiscoveryError> {
    if account.base_url.trim().is_empty() || account.api_key.trim().is_empty() {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "provider account is missing its base URL or credential",
        ));
    }
    if !(account.base_url.starts_with("https://") || account.base_url.starts_with("http://")) {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::InvalidRequest,
            "provider account base URL must use HTTP or HTTPS",
        ));
    }
    Ok(())
}

async fn read_bounded(
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

async fn read_probe_body(response: reqwest::Response) -> Result<Vec<u8>, ProbeFailure> {
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

fn normalize_catalog(
    account: &LLMAccount,
    payload: Value,
    evidence_policy: CapabilityEvidencePolicy,
) -> Result<DiscoveredCatalog, CapabilityDiscoveryError> {
    let items = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CapabilityDiscoveryError::new(
                CapabilityDiscoveryErrorKind::CatalogResponseInvalid,
                "model catalog did not contain a data or models array",
            )
        })?;
    let observed_at = Utc::now();
    let observed_at_text = observed_at.to_rfc3339();
    let expires_at = evidence_policy.expires_at(observed_at, CapabilityEvidenceSource::Catalog);
    let mut metadata = HashMap::new();
    let mut models = Vec::new();
    let mut observations = Vec::new();
    let mut seen = HashSet::new();

    for item in items.iter().take(MAX_CATALOG_MODELS) {
        let Some(id) = catalog_model_id(item) else {
            continue;
        };
        if id.is_empty() || id.chars().count() > MAX_MODEL_ID_CHARS || !seen.insert(id.clone()) {
            continue;
        }
        let safe = safe_catalog_metadata(item, &id);
        observations.extend(explicit_catalog_observations(
            account,
            &id,
            &safe,
            observed_at,
            evidence_policy,
        ));
        metadata.insert(id.clone(), safe);
        models.push(id);
    }
    models.sort();

    if models.is_empty() && !items.is_empty() {
        return Err(CapabilityDiscoveryError::new(
            CapabilityDiscoveryErrorKind::CatalogResponseInvalid,
            "model catalog contained no valid model IDs",
        ));
    }

    Ok(DiscoveredCatalog {
        snapshot: ModelCatalogSnapshot {
            account_id: account.id.clone(),
            provider: account.provider.clone(),
            observed_at: observed_at_text,
            expires_at,
            models,
            truncated: items.len() > MAX_CATALOG_MODELS,
        },
        metadata,
        observations,
    })
}

fn catalog_model_id(item: &Value) -> Option<String> {
    item.get("id")
        .or_else(|| item.get("slug"))
        .or_else(|| item.get("name"))
        .and_then(Value::as_str)
        .map(|id| id.trim_start_matches("models/").trim().to_string())
}

fn safe_catalog_metadata(item: &Value, id: &str) -> Value {
    const SAFE_KEYS: &[&str] = &[
        "object",
        "created",
        "owned_by",
        "displayName",
        "display_name",
        "description",
        "inputTokenLimit",
        "outputTokenLimit",
        "context_length",
        "context_window",
        "supportedGenerationMethods",
        "supportsTools",
        "supportsVision",
        "supportsStreaming",
        "capabilities",
    ];
    let mut safe = Map::new();
    safe.insert("id".to_string(), Value::String(id.to_string()));
    if let Some(object) = item.as_object() {
        for key in SAFE_KEYS {
            if let Some(value) = object.get(*key) {
                let bounded = bound_metadata_value(value, 0);
                safe.insert((*key).to_string(), bounded);
            }
        }
    }
    Value::Object(safe)
}

fn bound_metadata_value(value: &Value, depth: usize) -> Value {
    if depth >= 3 {
        return Value::Null;
    }
    match value {
        Value::String(text) => Value::String(text.chars().take(1024).collect()),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(32)
                .map(|value| bound_metadata_value(value, depth + 1))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .take(32)
                .map(|(key, value)| {
                    (
                        key.chars().take(128).collect(),
                        bound_metadata_value(value, depth + 1),
                    )
                })
                .collect(),
        ),
        scalar => scalar.clone(),
    }
}

fn explicit_catalog_observations(
    account: &LLMAccount,
    model: &str,
    metadata: &Value,
    observed_at: DateTime<Utc>,
    evidence_policy: CapabilityEvidencePolicy,
) -> Vec<CapabilityObservation> {
    let mut observations = Vec::new();
    for profile in [
        CapabilityProfile::Basic,
        CapabilityProfile::Tools,
        CapabilityProfile::Vision,
        CapabilityProfile::Streaming,
    ] {
        if let Some(supported) = explicit_capability(metadata, profile) {
            observations.push(observation(
                account,
                model,
                profile,
                CapabilityEvidenceSource::Catalog,
                if supported {
                    CapabilityObservationOutcome::Verified
                } else {
                    CapabilityObservationOutcome::Unsupported
                },
                observed_at,
                "explicit_provider_metadata",
                None,
                evidence_policy,
            ));
        }
    }
    observations
}

fn explicit_capability(metadata: &Value, profile: CapabilityProfile) -> Option<bool> {
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

fn capability_bool(metadata: &Value, keys: &[&str], nested_key: &str) -> Option<bool> {
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

fn probe_body(model: &str, profile: CapabilityProfile) -> Value {
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

fn validate_probe_response(
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

fn validate_adapted_probe(
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

fn contains_key(value: &Value, keys: &[&str]) -> bool {
    match value {
        Value::Array(values) => values.iter().any(|value| contains_key(value, keys)),
        Value::Object(values) => {
            values.keys().any(|key| keys.contains(&key.as_str()))
                || values.values().any(|value| contains_key(value, keys))
        }
        _ => false,
    }
}

fn account_settings(account: &LLMAccount, model: &str) -> LLMSettings {
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

fn known_models(account: &LLMAccount, catalog: Option<&ModelCatalogSnapshot>) -> HashSet<String> {
    account
        .models
        .iter()
        .chain(
            catalog
                .into_iter()
                .flat_map(|snapshot| snapshot.models.iter()),
        )
        .cloned()
        .collect()
}

fn deduplicate_trimmed(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn header_value(
    value: &str,
    message: &'static str,
) -> Result<HeaderValue, CapabilityDiscoveryError> {
    HeaderValue::from_str(value).map_err(|_| {
        CapabilityDiscoveryError::new(CapabilityDiscoveryErrorKind::InvalidRequest, message)
    })
}

fn merge_metadata(target: &mut Value, discovered: &Value) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Response, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};

    fn account() -> LLMAccount {
        LLMAccount {
            id: "account-1".to_string(),
            name: "Account".to_string(),
            provider: "openai".to_string(),
            base_url: "https://provider.test".to_string(),
            api_key: "secret".to_string(),
            models: vec!["existing".to_string()],
            ..LLMAccount::default()
        }
    }

    #[test]
    fn catalog_is_bounded_sanitized_and_preserves_explicit_evidence() {
        let account = account();
        let catalog = normalize_catalog(
            &account,
            json!({
                "data": [{
                    "id": "model-a",
                    "supportsTools": true,
                    "supportsVision": false,
                    "secretField": "must-not-survive",
                    "context_window": 32000
                }]
            }),
            CapabilityEvidencePolicy::default(),
        )
        .unwrap();

        assert_eq!(catalog.snapshot.models, vec!["model-a"]);
        assert!(catalog.metadata["model-a"].get("secretField").is_none());
        assert!(catalog.observations.iter().any(|observation| {
            observation.profile == CapabilityProfile::Tools
                && observation.outcome == CapabilityObservationOutcome::Verified
        }));
        assert!(catalog.observations.iter().any(|observation| {
            observation.profile == CapabilityProfile::Vision
                && observation.outcome == CapabilityObservationOutcome::Unsupported
        }));
    }

    #[test]
    fn applying_report_merges_models_and_never_persists_failed_downgrades() {
        let mut account = account();
        account.models_info = Some(HashMap::from([(
            "model-a".to_string(),
            json!({"capabilities": {"tools": true}}),
        )]));
        let mut settings = LLMSettings {
            accounts: vec![account],
            ..LLMSettings::default()
        };
        let report = CapabilityDiscoveryReport {
            account_id: "account-1".to_string(),
            provider: "openai".to_string(),
            catalog: Some(ModelCatalogSnapshot {
                account_id: "account-1".to_string(),
                provider: "openai".to_string(),
                observed_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: "2026-01-02T00:00:00Z".to_string(),
                models: vec!["model-a".to_string()],
                truncated: false,
            }),
            observations: vec![CapabilityObservation {
                account_id: "account-1".to_string(),
                model: "model-a".to_string(),
                profile: CapabilityProfile::Tools,
                source: CapabilityEvidenceSource::Probe,
                outcome: CapabilityObservationOutcome::Failed,
                observed_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: "2026-01-08T00:00:00Z".to_string(),
                detail: "transient".to_string(),
                status_code: Some(503),
            }],
            probe_requests: 1,
            recertification_plan: None,
            catalog_metadata: HashMap::from([(
                "model-a".to_string(),
                json!({"id": "model-a", "context_window": 32000}),
            )]),
        };

        apply_discovery_report(&mut settings, &report).unwrap();
        let info = &settings.accounts[0].models_info.as_ref().unwrap()["model-a"];
        assert_eq!(
            info.pointer("/capabilities/tools"),
            Some(&Value::Bool(true))
        );
        assert_eq!(info["context_window"], 32000);
        assert!(settings.accounts[0]
            .models
            .contains(&"existing".to_string()));
    }

    #[test]
    fn request_requires_explicit_network_confirmation_and_bounded_probe_product() {
        let request = CapabilityDiscoveryRequest {
            account_id: "account-1".to_string(),
            models: vec!["m1".to_string()],
            profiles: vec![CapabilityProfile::Basic],
            include_catalog: false,
            run_probes: true,
            confirm_network: false,
            revalidate_expired: false,
        };
        assert_eq!(
            validate_request(request).unwrap_err().kind,
            CapabilityDiscoveryErrorKind::InvalidRequest
        );

        let request = CapabilityDiscoveryRequest {
            account_id: "account-1".to_string(),
            models: vec!["m1".into(), "m2".into(), "m3".into()],
            profiles: vec![
                CapabilityProfile::Basic,
                CapabilityProfile::Tools,
                CapabilityProfile::Vision,
            ],
            include_catalog: false,
            run_probes: true,
            confirm_network: true,
            revalidate_expired: false,
        };
        assert_eq!(
            validate_request(request).unwrap_err().kind,
            CapabilityDiscoveryErrorKind::InvalidRequest
        );
    }

    #[test]
    fn evidence_expires_to_unknown_at_the_exact_ttl_boundary() {
        let now = parse_timestamp("2026-08-22T00:00:00Z").unwrap();
        let policy = CapabilityEvidencePolicy {
            catalog_ttl: ChronoDuration::hours(24),
            probe_ttl: ChronoDuration::hours(168),
        };
        let info = json!({
            "capabilities": {"tools": false},
            "capabilityEvidence": {
                "tools": {
                    "source": "probe",
                    "outcome": "unsupported",
                    "observedAt": "2026-08-15T00:00:00Z",
                    "expiresAt": "2026-08-22T00:00:00Z"
                }
            }
        });

        let fresh = assess_model_profile_at(
            &info,
            CapabilityProfile::Tools,
            now - ChronoDuration::seconds(1),
            policy,
        );
        assert_eq!(fresh.verdict, CapabilityVerdict::Unsupported);
        assert_eq!(fresh.freshness, CapabilityFreshness::Fresh);

        let expired = assess_model_profile_at(&info, CapabilityProfile::Tools, now, policy);
        assert_eq!(expired.verdict, CapabilityVerdict::Unknown);
        assert_eq!(expired.freshness, CapabilityFreshness::Expired);
    }

    #[test]
    fn invalid_timestamps_fail_closed_to_unknown_and_legacy_metadata_stays_unversioned() {
        let policy = CapabilityEvidencePolicy::default();
        let now = parse_timestamp("2026-08-22T00:00:00Z").unwrap();
        let invalid = json!({
            "capabilities": {"vision": true},
            "capabilityEvidence": {
                "vision": {
                    "source": "catalog",
                    "outcome": "verified",
                    "observedAt": "not-a-time"
                }
            }
        });
        let assessment = assess_model_profile_at(&invalid, CapabilityProfile::Vision, now, policy);
        assert_eq!(assessment.verdict, CapabilityVerdict::Unknown);
        assert_eq!(assessment.freshness, CapabilityFreshness::Invalid);

        let legacy = assess_model_profile_at(
            &json!({"supportsVision": true}),
            CapabilityProfile::Vision,
            now,
            policy,
        );
        assert_eq!(legacy.verdict, CapabilityVerdict::Verified);
        assert_eq!(legacy.freshness, CapabilityFreshness::Unversioned);
    }

    #[test]
    fn recertification_plan_is_deterministic_bounded_and_expired_only() {
        let now = parse_timestamp("2026-08-22T00:00:00Z").unwrap();
        let policy = CapabilityEvidencePolicy {
            catalog_ttl: ChronoDuration::hours(24),
            probe_ttl: ChronoDuration::hours(168),
        };
        let mut account = account();
        account.models.clear();
        let mut models_info = HashMap::new();
        for index in 0..5 {
            let model = format!("model-{index}");
            account.models.push(model.clone());
            models_info.insert(
                model,
                json!({
                    "capabilities": {"tools": true, "vision": true},
                    "capabilityEvidence": {
                        "tools": {
                            "source": "probe", "outcome": "verified",
                            "observedAt": "2026-08-01T00:00:00Z",
                            "expiresAt": "2026-08-08T00:00:00Z"
                        },
                        "vision": {
                            "source": "catalog", "outcome": "verified",
                            "observedAt": "2026-08-01T00:00:00Z",
                            "expiresAt": "2026-08-02T00:00:00Z"
                        },
                        "streaming": {
                            "source": "probe", "outcome": "verified",
                            "observedAt": "2026-08-21T00:00:00Z",
                            "expiresAt": "2026-08-28T00:00:00Z"
                        }
                    }
                }),
            );
        }
        account.models_info = Some(models_info);

        let plan = plan_recertification_for_account_at(&account, now, policy);
        assert_eq!(plan.probe_requests, MAX_PROBE_REQUESTS);
        assert_eq!(plan.items.len(), MAX_SELECTED_MODELS);
        assert!(plan.truncated);
        assert_eq!(plan.items[0].model, "model-0");
        assert!(plan.items.iter().all(|item| {
            item.profiles == vec![CapabilityProfile::Tools, CapabilityProfile::Vision]
        }));
    }

    #[tokio::test]
    async fn controlled_recertification_executes_only_the_expired_plan() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["tool_choice"], "required");
                Json(json!({
                    "choices": [{"message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call-1", "type": "function",
                            "function": {"name": "hub_capability_probe", "arguments": "{\"value\":\"OK\"}"}
                        }]
                    }}]
                }))
            }),
        );
        let mut configured = account();
        configured.base_url = spawn_test_server(app).await;
        configured.models = vec!["probe-model".to_string()];
        configured.models_info = Some(HashMap::from([(
            "probe-model".to_string(),
            json!({
                "capabilities": {"tools": true, "streaming": true},
                "capabilityEvidence": {
                    "tools": {
                        "source": "probe", "outcome": "verified",
                        "observedAt": "2020-01-01T00:00:00Z",
                        "expiresAt": "2020-01-08T00:00:00Z"
                    },
                    "streaming": {
                        "source": "probe", "outcome": "verified",
                        "observedAt": "2099-01-01T00:00:00Z",
                        "expiresAt": "2099-01-08T00:00:00Z"
                    }
                }
            }),
        )]));
        let settings = LLMSettings {
            accounts: vec![configured],
            ..LLMSettings::default()
        };
        let report = ProviderCapabilityRuntime::with_policy(
            reqwest::Client::new(),
            CapabilityEvidencePolicy::default(),
        )
        .discover(
            &settings,
            CapabilityDiscoveryRequest {
                account_id: "account-1".to_string(),
                models: Vec::new(),
                profiles: Vec::new(),
                include_catalog: false,
                run_probes: true,
                confirm_network: true,
                revalidate_expired: true,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.probe_requests, 1);
        let plan = report.recertification_plan.unwrap();
        assert_eq!(plan.items[0].profiles, vec![CapabilityProfile::Tools]);
        assert_eq!(
            report.observations[0].outcome,
            CapabilityObservationOutcome::Verified
        );
    }

    #[tokio::test]
    async fn discovery_calls_only_the_configured_catalog_and_redacts_credentials() {
        let app = Router::new().route(
            "/v1/models",
            get(|headers: HeaderMap| async move {
                assert_eq!(
                    headers
                        .get(AUTHORIZATION)
                        .and_then(|value| value.to_str().ok()),
                    Some("Bearer secret")
                );
                Json(json!({
                    "data": [{"id": "discovered-model", "supportsTools": true}]
                }))
            }),
        );
        let base_url = spawn_test_server(app).await;
        let mut configured = account();
        configured.base_url = base_url;
        let settings = LLMSettings {
            accounts: vec![configured],
            ..LLMSettings::default()
        };
        let report = ProviderCapabilityRuntime::new(reqwest::Client::new())
            .discover(
                &settings,
                CapabilityDiscoveryRequest {
                    account_id: "account-1".to_string(),
                    models: Vec::new(),
                    profiles: Vec::new(),
                    include_catalog: true,
                    run_probes: false,
                    confirm_network: true,
                    revalidate_expired: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(
            report.catalog.as_ref().unwrap().models,
            vec!["discovered-model"]
        );
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains(&settings.accounts[0].base_url));
    }

    #[tokio::test]
    async fn fixed_tools_and_streaming_probes_require_canonical_evidence() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                if body.get("stream").and_then(Value::as_bool) == Some(true) {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .body(Body::from(
                            "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\n",
                        ))
                        .unwrap()
                } else {
                    assert_eq!(body["max_tokens"], PROBE_MAX_TOKENS);
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&json!({
                                "choices": [{
                                    "message": {
                                        "role": "assistant",
                                        "content": null,
                                        "tool_calls": [{
                                            "id": "call-1",
                                            "type": "function",
                                            "function": {
                                                "name": "hub_capability_probe",
                                                "arguments": "{\"value\":\"OK\"}"
                                            }
                                        }]
                                    }
                                }]
                            }))
                            .unwrap(),
                        ))
                        .unwrap()
                }
            }),
        );
        let base_url = spawn_test_server(app).await;
        let mut configured = account();
        configured.base_url = base_url;
        configured.models = vec!["probe-model".to_string()];
        let settings = LLMSettings {
            accounts: vec![configured],
            ..LLMSettings::default()
        };
        let report = ProviderCapabilityRuntime::new(reqwest::Client::new())
            .discover(
                &settings,
                CapabilityDiscoveryRequest {
                    account_id: "account-1".to_string(),
                    models: vec!["probe-model".to_string()],
                    profiles: vec![CapabilityProfile::Tools, CapabilityProfile::Streaming],
                    include_catalog: false,
                    run_probes: true,
                    confirm_network: true,
                    revalidate_expired: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(report.probe_requests, 2);
        assert!(report
            .observations
            .iter()
            .all(|observation| { observation.outcome == CapabilityObservationOutcome::Verified }));
    }

    async fn spawn_test_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{address}")
    }
}
