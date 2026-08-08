use crate::services::agent::AgentKind;
use adapter::CliConfigAdapter;
use adapters::claude::ClaudeConfigAdapter;
use adapters::codex::CodexConfigAdapter;
use std::sync::Arc;

pub mod adapter;
pub mod adapters;
pub mod coordinator;
pub mod transaction;
pub mod history;

pub fn get_adapter(kind: AgentKind) -> Option<Arc<dyn CliConfigAdapter>> {
    match kind {
        AgentKind::Claude => Some(Arc::new(ClaudeConfigAdapter)),
        AgentKind::Codex => Some(Arc::new(CodexConfigAdapter)),
        _ => None,
    }
}
