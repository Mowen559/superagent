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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProfile {
    Basic,
    Tools,
    Vision,
    Streaming,
}

impl CapabilityProfile {
    pub(crate) fn metadata_key(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Tools => "tools",
            Self::Vision => "vision",
            Self::Streaming => "streaming",
        }
    }

    pub(crate) fn from_metadata_key(value: &str) -> Option<Self> {
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

