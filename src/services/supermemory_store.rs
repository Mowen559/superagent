use std::path::PathBuf;
use crate::config::runtime::RuntimeConfig;
use crate::models::memory::{SupermemoryStoreData, SupermemoryRecord, SupermemoryLogEntry};

/// Supermemory store - local JSON-based memory storage
pub struct SupermemoryStore {
    store_path: PathBuf,
}

impl SupermemoryStore {
    pub fn new(runtime_config: &RuntimeConfig, project_path: Option<&str>) -> Self {
        Self {
            store_path: runtime_config.supermemory_store_path(project_path),
        }
    }

    pub fn load(&self) -> SupermemoryStoreData {
        if !self.store_path.exists() {
            return SupermemoryStoreData::default();
        }
        std::fs::read_to_string(&self.store_path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data: &SupermemoryStoreData) {
        if let Some(parent) = self.store_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(json) = serde_json::to_string_pretty(data) {
            std::fs::write(&self.store_path, json).ok();
        }
    }

    /// Search memories by keyword (token-based: any token hit counts)
    pub fn search(&self, query: &str, limit: usize) -> Vec<SupermemoryRecord> {
        let data = self.load();
        let tokens: Vec<String> = query.to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();

        let results: Vec<SupermemoryRecord> = data.memories
            .into_iter()
            .filter(|m| {
                if tokens.is_empty() {
                    return true;
                }
                let haystack = format!(
                    "{} {} {} {}",
                    m.name.to_lowercase(),
                    m.summary.to_lowercase(),
                    m.tags.join(" ").to_lowercase(),
                    m.file_path.to_lowercase()
                );
                tokens.iter().any(|t| haystack.contains(t))
            })
            .take(limit)
            .collect();

        results
    }

    /// Write a new memory record
    pub fn write_memory(&self, record: SupermemoryRecord) {
        let mut data = self.load();
        // Replace if exists, otherwise push
        if let Some(existing) = data.memories.iter_mut().find(|m| m.id == record.id) {
            *existing = record;
        } else {
            data.memories.push(record);
        }
        data.stats.total_memories = data.memories.len() as u32;
        data.stats.last_sync = chrono::Utc::now().to_rfc3339();
        self.save(&data);
    }

    /// Add a log entry
    pub fn add_log(&self, log_type: &str, message: Option<String>, query: Option<String>) {
        let mut data = self.load();
        let entry = SupermemoryLogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            log_type: log_type.to_string(),
            message,
            query,
            response: None,
            tokens_used: None,
            matched_memory_ids: None,
        };
        data.logs.push(entry);
        if data.logs.len() > 500 {
            data.logs = data.logs.split_off(data.logs.len() - 500);
        }
        if log_type == "query" {
            data.stats.total_queries += 1;
        }
        self.save(&data);
    }

    /// Get recent logs
    pub fn get_logs(&self, limit: usize) -> Vec<SupermemoryLogEntry> {
        let data = self.load();
        data.logs.into_iter().rev().take(limit).collect()
    }
}
