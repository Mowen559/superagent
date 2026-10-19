//! 蜂群集体意识流与共享认知黑板 (Hive-Mind & Consciousness Stream)
//! 提供多智能体透明思维流追踪、经验微反思与全局黑板同步。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThoughtType {
    Observation,
    Hypothesis,
    Reflection,
    Decision,
    Warning,
    Broadcast,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtEntry {
    pub id: String,
    pub source_node: String,
    pub thought_type: ThoughtType,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardFact {
    pub key: String,
    pub value: serde_json::Value,
    pub author: String,
    pub updated_at: DateTime<Utc>,
    pub version: u64,
}

pub struct ConsciousnessEngine {
    stream: Vec<ThoughtEntry>,
    blackboard: HashMap<String, BlackboardFact>,
    max_history: usize,
}

impl ConsciousnessEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            stream: Vec::with_capacity(1000),
            blackboard: HashMap::new(),
            max_history: 1000,
        };

        // 注入初始意识种子
        engine.record_thought(
            "system",
            ThoughtType::Reflection,
            "Collective consciousness online. Hive-mind protocol engaged.",
            vec!["bootstrap".into(), "swarm".into()],
        );

        engine
    }

    pub fn record_thought(
        &mut self,
        source: &str,
        thought_type: ThoughtType,
        content: &str,
        tags: Vec<String>,
    ) -> String {
        let id = format!("th-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let entry = ThoughtEntry {
            id: id.clone(),
            source_node: source.to_string(),
            thought_type,
            content: content.to_string(),
            timestamp: Utc::now(),
            tags,
        };
        self.stream.push(entry);
        if self.stream.len() > self.max_history {
            self.stream.remove(0);
        }
        id
    }

    pub fn get_recent_stream(&self, limit: usize) -> Vec<ThoughtEntry> {
        let count = limit.min(self.stream.len());
        self.stream.iter().rev().take(count).cloned().collect()
    }

    pub fn set_fact(&mut self, key: &str, value: serde_json::Value, author: &str) {
        let version = self.blackboard.get(key).map(|f| f.version + 1).unwrap_or(1);
        self.blackboard.insert(
            key.to_string(),
            BlackboardFact {
                key: key.to_string(),
                value,
                author: author.to_string(),
                updated_at: Utc::now(),
                version,
            },
        );
    }

    pub fn get_fact(&self, key: &str) -> Option<&BlackboardFact> {
        self.blackboard.get(key)
    }

    pub fn all_facts(&self) -> Vec<BlackboardFact> {
        self.blackboard.values().cloned().collect()
    }
}

lazy_static::lazy_static! {
    pub static ref GLOBAL_CONSCIOUSNESS: Arc<RwLock<ConsciousnessEngine>> = Arc::new(RwLock::new(ConsciousnessEngine::new()));
}
