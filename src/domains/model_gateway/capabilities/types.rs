use serde::{Deserialize, Serialize};

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
