use std::collections::{HashMap, HashSet};
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::config::llm_settings::LLMAccount;
use super::policy::{CapabilityEvidencePolicy, MAX_CATALOG_MODELS, MAX_MODEL_ID_CHARS};
use super::types::{CapabilityEvidenceSource, CapabilityObservation, CapabilityObservationOutcome, CapabilityProfile};
use super::discovery::{CapabilityDiscoveryError, CapabilityDiscoveryErrorKind};
use super::assessment::{explicit_capability, observation};
use super::utils::{deduplicate_trimmed, header_value, parse_timestamp};

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

#[derive(Debug)]
pub(crate) struct DiscoveredCatalog {
    pub(crate) snapshot: ModelCatalogSnapshot,
    pub(crate) metadata: HashMap<String, Value>,
    pub(crate) observations: Vec<CapabilityObservation>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CatalogAdapter {
    OpenAiCompatible,
    Anthropic,
    Gemini,
    GeminiNative,
}

impl CatalogAdapter {
    pub(crate) fn for_account(account: &LLMAccount) -> Result<Self, CapabilityDiscoveryError> {
        let clean_provider = account.provider.to_ascii_lowercase().replace([' ', '-'], "_");
        if clean_provider == "anthropic" || clean_provider == "claude" {
            Ok(Self::Anthropic)
        } else if clean_provider == "gemini_native" || clean_provider == "google_gemini_native" {
            Ok(Self::GeminiNative)
        } else if clean_provider == "gemini" || clean_provider == "google" {
            Ok(Self::Gemini)
        } else {
            Ok(Self::OpenAiCompatible)
        }
    }

    pub(crate) fn prepare(
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
            Self::GeminiNative => {
                if account.api_key.starts_with("ya29.") {
                    headers.insert(
                        AUTHORIZATION,
                        header_value(
                            &format!("Bearer {}", account.api_key),
                            "invalid provider access token",
                        )?,
                    );
                } else if !account.api_key.is_empty() {
                    let bearer = if account.api_key.starts_with("Bearer ") {
                        account.api_key.clone()
                    } else {
                        format!("Bearer {}", account.api_key)
                    };
                    if let Ok(v) = HeaderValue::from_str(&bearer) {
                        headers.insert(AUTHORIZATION, v);
                    }
                    if let Ok(v) = HeaderValue::from_str(&account.api_key) {
                        headers.insert("x-goog-api-key", v);
                    }
                }

                let url = if base.ends_with("/models") {
                    base.to_string()
                } else if base.contains("openapi") || base.ends_with("/v1") {
                    format!("{base}/models")
                } else if base.ends_with("/v1beta") || base.ends_with("/v1beta1") {
                    format!("{base}/models")
                } else if base.is_empty() {
                    "https://generativelanguage.googleapis.com/v1beta/models".to_string()
                } else {
                    format!("{base}/v1beta/models")
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

pub(crate) fn normalize_catalog(
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

pub(crate) fn catalog_model_id(item: &Value) -> Option<String> {
    item.get("id")
        .or_else(|| item.get("slug"))
        .or_else(|| item.get("name"))
        .and_then(Value::as_str)
        .map(|id| id.trim_start_matches("models/").trim().to_string())
}

pub(crate) fn safe_catalog_metadata(item: &Value, id: &str) -> Value {
    pub(crate) const SAFE_KEYS: &[&str] = &[
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

pub(crate) fn bound_metadata_value(value: &Value, depth: usize) -> Value {
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

pub(crate) fn explicit_catalog_observations(
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

pub(crate) fn known_models(account: &LLMAccount, catalog: Option<&ModelCatalogSnapshot>) -> HashSet<String> {
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



pub(crate) fn static_gemini_native_catalog(
    account: &LLMAccount,
    evidence_policy: CapabilityEvidencePolicy,
) -> DiscoveredCatalog {
    let observed_at = Utc::now();
    let observed_at_text = observed_at.to_rfc3339();
    let expires_at = evidence_policy.expires_at(observed_at, CapabilityEvidenceSource::Catalog);
    let models = vec![
        "gemini-2.5-flash".to_string(),
        "gemini-2.5-pro".to_string(),
        "gemini-2.5-flash-thinking".to_string(),
        "gemini-2.0-flash".to_string(),
        "gemini-2.0-flash-lite".to_string(),
        "gemini-2.0-pro-exp".to_string(),
        "gemini-1.5-pro".to_string(),
        "gemini-1.5-flash".to_string(),
        "gemini-1.5-flash-8b".to_string(),
        "gemini-exp-1206".to_string(),
        "gemini-3-flash".to_string(),
        "gemini-3.8-flash-high".to_string(),
        "gemini-3.1-pro-high".to_string(),
    ];
    let mut metadata = HashMap::new();
    let mut observations = Vec::new();
    for model_id in &models {
        let safe = json!({
            "id": model_id,
            "displayName": model_id,
            "supportsTools": true,
            "supportsVision": true,
            "supportsStreaming": true,
        });
        observations.extend(explicit_catalog_observations(
            account,
            model_id,
            &safe,
            observed_at,
            evidence_policy,
        ));
        metadata.insert(model_id.clone(), safe);
    }
    DiscoveredCatalog {
        snapshot: ModelCatalogSnapshot {
            account_id: account.id.clone(),
            provider: account.provider.clone(),
            observed_at: observed_at_text,
            expires_at,
            models,
            truncated: false,
        },
        metadata,
        observations,
    }
}
