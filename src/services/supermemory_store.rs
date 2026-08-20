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

    /// Search memories using Hybrid Search (BM25 Keyword + Dense Vector Embedding)
    pub fn search(&self, query: &str, limit: usize) -> Vec<SupermemoryRecord> {
        let data = self.load();
        
        let mut query_embedding = None;
        if let Ok(svc) = crate::services::brain::embedding_service::get_embedding_service() {
            use crate::services::brain::embedding_service::EmbeddingService;
            query_embedding = svc.embed_text(query).ok();
        }

        let tokens: Vec<String> = query.to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();

        let mut scored_memories: Vec<(f32, SupermemoryRecord)> = data.memories
            .into_iter()
            .map(|m| {
                let mut score = 0.0;
                
                // 1. BM25 / Keyword Score
                if !tokens.is_empty() {
                    let haystack = format!(
                        "{} {} {} {}",
                        m.name.to_lowercase(),
                        m.summary.to_lowercase(),
                        m.tags.join(" ").to_lowercase(),
                        m.file_path.to_lowercase()
                    );
                    let matched_tokens = tokens.iter().filter(|t| haystack.contains(*t)).count();
                    score += (matched_tokens as f32 / tokens.len() as f32) * 0.5;
                }

                // 2. Vector Semantic Score
                if let (Some(q_emb), Some(m_emb)) = (&query_embedding, &m.embedding) {
                    let sim = crate::services::brain::embedding_service::cosine_similarity(q_emb, m_emb);
                    if sim > 0.2 { // Threshold
                        score += sim * 0.5;
                    }
                }

                (score, m)
            })
            .filter(|(score, _)| *score > 0.05 || tokens.is_empty())
            .collect();

        scored_memories.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored_memories.into_iter().map(|(_, m)| m).take(limit).collect()
    }

    /// Write a new memory record
    pub fn write_memory(&self, mut record: SupermemoryRecord) {
        if record.embedding.is_none() {
            if let Ok(svc) = crate::services::brain::embedding_service::get_embedding_service() {
                use crate::services::brain::embedding_service::EmbeddingService;
                let text_to_embed = format!("{} {} {}", record.name, record.summary, record.tags.join(" "));
                if let Ok(emb) = svc.embed_text(&text_to_embed) {
                    record.embedding = Some(emb);
                }
            }
        }

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
