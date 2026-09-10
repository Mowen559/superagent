use std::collections::{HashMap, HashSet};
use std::fmt;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::llm_settings::{LLMAccount, LLMSettings};
use super::policy::{CapabilityEvidencePolicy, MAX_MODEL_ID_CHARS, MAX_PROFILES, MAX_PROBE_REQUESTS, MAX_SELECTED_MODELS};
use super::types::{
    CapabilityEvidenceSource, CapabilityObservation, CapabilityObservationOutcome,
    CapabilityProfile, RecertificationPlan,
};
use super::catalog::ModelCatalogSnapshot;
use super::assessment::plan_recertification_for_account_at;
use super::utils::{account_settings, deduplicate_trimmed, merge_metadata};

pub(crate) fn default_catalog_enabled() -> bool {
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
    pub(crate) catalog_metadata: HashMap<String, Value>,
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
    pub(crate) fn new(kind: CapabilityDiscoveryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status_code: None,
        }
    }

    pub(crate) fn http(kind: CapabilityDiscoveryErrorKind, status_code: u16) -> Self {
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
pub(crate) fn validate_request(
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

pub(crate) fn validate_account(account: &LLMAccount) -> Result<(), CapabilityDiscoveryError> {
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

