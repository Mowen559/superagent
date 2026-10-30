use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityState {
    Certified,
    Experimental,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityMetadata {
    pub name: String,
    pub description: String,
    pub state: CapabilityState,
    #[serde(default)]
    pub evidence: Vec<String>,
}

pub struct CertificationGatekeeper {
    capabilities: RwLock<HashMap<String, CapabilityMetadata>>,
}

impl CertificationGatekeeper {
    pub fn with_builtins() -> Self {
        let gatekeeper = Self::new();
        let builtins = [
            (
                "agent.stream-json",
                "Stream-JSON Agent adapters",
                CapabilityState::Certified,
                "Shared stream driver with cancellation",
            ),
            (
                "agent.json",
                "JSON/JSONL Agent adapters",
                CapabilityState::Certified,
                "Shared JSON driver with bounded execution",
            ),
            (
                "agent.acp",
                "Agent Client Protocol adapters",
                CapabilityState::Experimental,
                "ACP session lifecycle and permission events are wired",
            ),
            (
                "agent.app-server",
                "Codex App Server adapter",
                CapabilityState::Experimental,
                "App-server adapter is wired; needs a real CLI fixture",
            ),
            (
                "mcp.server",
                "Model Context Protocol stdio server",
                CapabilityState::Certified,
                "initialize, tools/list and tools/call are implemented",
            ),
            (
                "harness.acp",
                "DeepSeek Harness ACP bridge",
                CapabilityState::Experimental,
                "Harness ACP client is available as a native tool",
            ),
            (
                "memory.local",
                "Project Supermemory",
                CapabilityState::Certified,
                "Local search, write and event logging",
            ),
            (
                "code-intelligence.graph",
                "Code graph and hybrid retrieval",
                CapabilityState::Experimental,
                "SQLite schema validation exists; external indexer fixture pending",
            ),
            (
                "workspace.guard",
                "Workspace capability guard",
                CapabilityState::Certified,
                "Canonical path validation for destructive operations",
            ),
            (
                "plugins.dynamic",
                "Dynamic script tools",
                CapabilityState::Experimental,
                "Manifest-scanned tools with bounded execution",
            ),
            (
                "agent.evolution",
                "Evidence-gated strategy evolution",
                CapabilityState::Experimental,
                "Candidate strategies require repeated successful Runs before promotion",
            ),
            (
                "agent.task-understanding",
                "Context-first requirement understanding and dependency-aware task contracts",
                CapabilityState::Experimental,
                "Run-local receipt gates, atomic versioned additive revisions with retained evidence, and bounded synthetic planning evaluations; real implementation success remains unmeasured",
            ),
        ];
        for (name, description, state, evidence) in builtins {
            gatekeeper.register(CapabilityMetadata {
                name: name.to_string(),
                description: description.to_string(),
                state,
                evidence: vec![evidence.to_string()],
            });
        }
        gatekeeper
    }

    pub fn new() -> Self {
        Self {
            capabilities: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, metadata: CapabilityMetadata) {
        let mut caps = self.capabilities.write().unwrap();
        caps.insert(metadata.name.clone(), metadata);
    }

    pub fn get_state(&self, name: &str) -> Option<CapabilityState> {
        let caps = self.capabilities.read().unwrap();
        caps.get(name).map(|m| m.state)
    }

    pub fn is_allowed(&self, name: &str) -> bool {
        match self.get_state(name) {
            Some(CapabilityState::Certified) => true,
            Some(CapabilityState::Experimental) => true, // Might require user consent in the future
            Some(CapabilityState::Disabled) => false,
            None => false, // Unregistered capabilities are not allowed
        }
    }

    pub fn list_capabilities(&self) -> Vec<CapabilityMetadata> {
        let caps = self.capabilities.read().unwrap();
        let mut result: Vec<_> = caps.values().cloned().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }
}

impl Default for CertificationGatekeeper {
    fn default() -> Self {
        Self::with_builtins()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_are_discoverable_and_sorted() {
        let catalog = CertificationGatekeeper::with_builtins();
        let caps = catalog.list_capabilities();
        assert!(caps.len() >= 8);
        assert!(caps.windows(2).all(|pair| pair[0].name <= pair[1].name));
        assert_eq!(
            catalog.get_state("mcp.server"),
            Some(CapabilityState::Certified)
        );
        assert!(!catalog.is_allowed("unknown.capability"));
        assert_eq!(
            catalog.get_state("agent.task-understanding"),
            Some(CapabilityState::Experimental)
        );
    }
}
