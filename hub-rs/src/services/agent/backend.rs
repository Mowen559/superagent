//! The `AgentBackend` trait: the single abstraction every CLI adapter
//! implements. Mirrors multica's one-method `agent.Backend` interface — one
//! `execute` call returns an [`AgentSession`] the caller drains for streaming
//! events and the final result.

use async_trait::async_trait;

use super::models::Model;
use super::types::{AgentSession, ExecOptions};

/// Result of attempting to start a run. `Err` means the backend could not even
/// launch (CLI missing, spawn failure, not-yet-implemented protocol); once an
/// [`AgentSession`] is returned, all further outcomes flow through it.
pub type BackendResult<T> = Result<T, BackendError>;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("agent CLI not found: {0}")]
    NotFound(String),
    #[error("failed to launch agent: {0}")]
    Launch(String),
    #[error("agent backend not implemented yet: {0}")]
    NotImplemented(String),
}

/// A coding-agent adapter. Implementations translate a prompt + options into a
/// spawned CLI subprocess and stream its protocol back as unified events.
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Start a run. The prompt is delivered per the backend's protocol (stdin
    /// for stream-json, argv for json). Returns a session to stream from.
    async fn execute(
        &self,
        prompt: String,
        opts: ExecOptions,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        control_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::services::task_manager::TaskControl>,
        >,
    ) -> BackendResult<AgentSession>;

    /// Returns the models supported by the given agent provider.
    async fn list_models(&self, executable_path: Option<&str>) -> BackendResult<Vec<Model>> {
        let _ = executable_path; // Default unused
        Ok(vec![])
    }

    /// Reports whether setting `agent.model` has any effect for the given provider.
    fn model_selection_supported(&self) -> bool {
        true
    }

    /// Receives unified configuration from the frontend and performs agent-specific local writes
    /// (e.g. updating ~/.claude/settings.json or ~/.codex/config.toml).
    async fn sync_config(
        &self,
        config: &super::types::SyncConfigPayload,
        executable_path: Option<&str>,
    ) -> BackendResult<()> {
        let _ = config;
        let _ = executable_path;
        Err(BackendError::NotImplemented(
            "sync_config not implemented for this backend".to_string(),
        ))
    }

    /// Reads the agent-specific local configuration file and returns its content as a string.
    async fn get_config_content(
        &self,
        executable_path: Option<&str>,
    ) -> BackendResult<Option<String>> {
        let _ = executable_path;
        Ok(None)
    }
}
