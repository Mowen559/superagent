use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::services::agent::AgentKind;

#[derive(Debug, Clone)]
pub struct CliInstallation {
    pub executable: String,
    pub version: String,
    pub config_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagementMode {
    Exclusive,
    Additive,
    RuntimeOnly,
}

#[derive(Debug, Clone)]
pub struct CliConfigCapabilities {
    pub management_mode: ManagementMode,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveCliConfig {
    pub requested_model: Option<String>,
    pub persisted_model: Option<String>,
    pub effective_model: Option<String>,
    pub model_source: String,
    pub effective_provider: Option<String>,
    pub effective_base_url: Option<String>,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequestModel {
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSyncRequest {
    pub provider_id: String,
    pub model: SyncRequestModel,
    #[serde(default = "default_persist_default")]
    pub persist_default: bool,
    pub expected_revision: Option<String>,
    pub validation_level: Option<String>,
    #[serde(default)]
    pub allow_network_probe: bool,
}

fn default_persist_default() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct ConfigMutationPlan {
    /// Map of absolute path to the intended new contents
    pub files_to_write: HashMap<String, String>,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub syntax: String,
    pub semantic: String,
    pub read_back: String,
    pub cli_probe: String,
    pub network_probe: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("CLI not installed")]
    CliNotInstalled,
    #[error("CLI version unsupported")]
    CliVersionUnsupported,
    #[error("Config not readable: {0}")]
    ConfigNotReadable(String),
    #[error("Config parse failed: {0}")]
    ConfigParseFailed(String),
    #[error("Config conflict")]
    ConfigConflict,
    #[error("Config locked")]
    ConfigLocked,
    #[error("Schema validation failed: {0}")]
    SchemaValidationFailed(String),
    #[error("Protocol incompatible")]
    ProtocolIncompatible,
    #[error("Credential unavailable")]
    CredentialUnavailable,
    #[error("Atomic write failed: {0}")]
    AtomicWriteFailed(String),
    #[error("Read back mismatch")]
    ReadBackMismatch,
    #[error("CLI probe failed: {0}")]
    CliProbeFailed(String),
    #[error("Network probe failed: {0}")]
    NetworkProbeFailed(String),
    #[error("Rollback failed: {0}")]
    RollbackFailed(String),
}

#[async_trait]
pub trait CliConfigAdapter: Send + Sync {
    fn kind(&self) -> AgentKind;
    async fn detect(&self) -> Result<CliInstallation, SyncError>;
    async fn capabilities(
        &self,
        install: &CliInstallation,
    ) -> Result<CliConfigCapabilities, SyncError>;
    async fn inspect(&self, install: &CliInstallation) -> Result<EffectiveCliConfig, SyncError>;
    async fn plan(
        &self,
        install: &CliInstallation,
        request: &ResolvedSyncRequest,
        hub_base_url: &str,
    ) -> Result<ConfigMutationPlan, SyncError>;
    fn validate_rendered(&self, plan: &ConfigMutationPlan) -> Result<(), SyncError>;
    async fn probe(&self, install: &CliInstallation, expected: &EffectiveCliConfig) -> Result<ProbeResult, SyncError>;
}
