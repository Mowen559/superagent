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


pub mod types;
pub use types::*;
pub mod discovery;
pub use discovery::*;
pub mod assessment;
pub use assessment::*;
pub mod catalog;
pub use catalog::*;
pub mod probe;
pub use probe::*;
pub mod utils;
pub use utils::*;

#[cfg(test)]
pub mod tests;

/// Deep Module owning bounded model discovery, fixed active probes and safe
/// evidence normalization. Its Interface accepts only an existing account ID;
/// URLs, credentials and provider request bodies remain in the Implementation.
#[derive(Clone)]
pub struct ProviderCapabilityRuntime {
    pub(crate) client: reqwest::Client,
    pub(crate) evidence_policy: CapabilityEvidencePolicy,
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

