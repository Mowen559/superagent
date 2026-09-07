use std::time::Duration;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use super::types::CapabilityEvidenceSource;
use super::utils::ttl_hours_from_env;

pub const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CATALOG_MODELS: usize = 256;
pub const MAX_MODEL_ID_CHARS: usize = 256;
pub const MAX_SELECTED_MODELS: usize = 4;
pub const MAX_PROFILES: usize = 4;
pub const MAX_PROBE_REQUESTS: usize = 8;
pub const MAX_PROBE_BYTES: usize = 128 * 1024;
pub const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
pub const PROBE_MAX_TOKENS: u32 = 8;
pub const DEFAULT_CATALOG_TTL_HOURS: i64 = 24;
pub const DEFAULT_PROBE_TTL_HOURS: i64 = 7 * 24;
pub const MIN_EVIDENCE_TTL_HOURS: i64 = 1;
pub const MAX_EVIDENCE_TTL_HOURS: i64 = 90 * 24;
pub const TINY_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityPolicy {
    pub max_catalog_bytes: usize,
    pub max_catalog_models: usize,
    pub max_selected_models: usize,
    pub max_profiles: usize,
    pub max_probe_requests: usize,
    pub max_probe_bytes: usize,
    pub catalog_timeout: Duration,
    pub probe_timeout: Duration,
    pub probe_max_tokens: u32,
    pub default_catalog_ttl_hours: i64,
    pub default_probe_ttl_hours: i64,
}

impl Default for CapabilityPolicy {
    fn default() -> Self {
        Self {
            max_catalog_bytes: MAX_CATALOG_BYTES,
            max_catalog_models: MAX_CATALOG_MODELS,
            max_selected_models: MAX_SELECTED_MODELS,
            max_profiles: MAX_PROFILES,
            max_probe_requests: MAX_PROBE_REQUESTS,
            max_probe_bytes: MAX_PROBE_BYTES,
            catalog_timeout: CATALOG_TIMEOUT,
            probe_timeout: PROBE_TIMEOUT,
            probe_max_tokens: PROBE_MAX_TOKENS,
            default_catalog_ttl_hours: DEFAULT_CATALOG_TTL_HOURS,
            default_probe_ttl_hours: DEFAULT_PROBE_TTL_HOURS,
        }
    }
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
    pub(crate) fn ttl_for(self, source: CapabilityEvidenceSource) -> ChronoDuration {
        match source {
            CapabilityEvidenceSource::Catalog => self.catalog_ttl,
            CapabilityEvidenceSource::Probe => self.probe_ttl,
        }
    }

    pub(crate) fn expires_at(self, observed_at: DateTime<Utc>, source: CapabilityEvidenceSource) -> String {
        (observed_at + self.ttl_for(source)).to_rfc3339()
    }
}
