use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardEntry {
    pub key: String,
    pub value: String,
    pub publisher: String,
    pub tag: String,
    pub timestamp: i64,
}

#[derive(Clone, Default)]
pub struct AgentBlackboard {
    entries: Arc<RwLock<HashMap<String, BlackboardEntry>>>,
    history: Arc<RwLock<Vec<BlackboardEntry>>>,
}

impl AgentBlackboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn post(&self, key: impl Into<String>, value: impl Into<String>, publisher: impl Into<String>, tag: impl Into<String>) {
        let entry = BlackboardEntry {
            key: key.into(),
            value: value.into(),
            publisher: publisher.into(),
            tag: tag.into(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        let mut entries_lock = self.entries.write().unwrap();
        entries_lock.insert(entry.key.clone(), entry.clone());

        let mut history_lock = self.history.write().unwrap();
        history_lock.push(entry);
    }

    pub fn get(&self, key: &str) -> Option<BlackboardEntry> {
        let lock = self.entries.read().unwrap();
        lock.get(key).cloned()
    }

    pub fn query_by_tag(&self, tag: &str) -> Vec<BlackboardEntry> {
        let lock = self.entries.read().unwrap();
        lock.values().filter(|e| e.tag == tag).cloned().collect()
    }

    pub fn get_all_history(&self) -> Vec<BlackboardEntry> {
        let lock = self.history.read().unwrap();
        lock.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blackboard_post_and_get() {
        let board = AgentBlackboard::new();
        board.post("spec", "Build auth module", "Planner", "Plan");
        let entry = board.get("spec").unwrap();
        assert_eq!(entry.publisher, "Planner");
        assert_eq!(entry.tag, "Plan");

        let plan_entries = board.query_by_tag("Plan");
        assert_eq!(plan_entries.len(), 1);
    }
}
