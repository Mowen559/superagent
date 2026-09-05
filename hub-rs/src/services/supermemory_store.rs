use crate::config::runtime::RuntimeConfig;
use crate::models::memory::{
    KnowledgeDocumentRecord, SupermemoryLogEntry, SupermemoryRecord, SupermemoryStoreData,
};
use serde::Serialize;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

lazy_static::lazy_static! {
    // All in-process writers share one checkpoint lock. The JSON format is
    // retained for compatibility; atomic rename prevents torn files.
    static ref STORE_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredMemoryRecord {
    pub record: SupermemoryRecord,
    pub score: f32,
    pub keyword_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_score: Option<f32>,
}

/// Local JSON memory store with atomic checkpoints and Corpus Manifest
/// mutations. Generic Memory callers keep their original interface, while the
/// Knowledge Corpus Runtime gets a transactional replacement operation.
#[derive(Debug, Clone)]
pub struct SupermemoryStore {
    store_path: PathBuf,
}

impl SupermemoryStore {
    pub fn new(runtime_config: &RuntimeConfig, project_path: Option<&str>) -> Self {
        Self {
            store_path: runtime_config.supermemory_store_path(project_path),
        }
    }

    pub fn from_path(store_path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: store_path.into(),
        }
    }

    pub fn load(&self) -> SupermemoryStoreData {
        self.load_result().unwrap_or_default()
    }

    pub fn load_result(&self) -> Result<SupermemoryStoreData, String> {
        if !self.store_path.exists() {
            return Ok(SupermemoryStoreData::default());
        }
        let content = std::fs::read_to_string(&self.store_path)
            .map_err(|error| format!("failed to read {}: {error}", self.store_path.display()))?;
        serde_json::from_str(&content)
            .map_err(|error| format!("failed to parse {}: {error}", self.store_path.display()))
    }

    pub fn save(&self, data: &SupermemoryStoreData) {
        if let Err(error) = self.save_result(data) {
            eprintln!("[SupermemoryStore] {error}");
        }
    }

    pub fn save_result(&self, data: &SupermemoryStoreData) -> Result<(), String> {
        let _guard = STORE_WRITE_LOCK
            .lock()
            .map_err(|_| "supermemory checkpoint lock is poisoned".to_string())?;
        self.with_checkpoint_lock(|| self.write_checkpoint(data))
    }

    /// Compatibility search using hybrid keyword/vector ranking.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SupermemoryRecord> {
        let query_embedding = best_effort_embedding(query);
        self.search_scored_result(query, limit, query_embedding.as_deref(), None)
            .unwrap_or_default()
            .into_iter()
            .map(|hit| hit.record)
            .collect()
    }

    pub fn search_scored_result(
        &self,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
        node_type: Option<&str>,
    ) -> Result<Vec<ScoredMemoryRecord>, String> {
        self.search_scored_internal(query, limit, query_embedding, node_type, None)
    }

    pub fn search_scored_ids_result(
        &self,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
        allowed_ids: &HashSet<String>,
    ) -> Result<Vec<ScoredMemoryRecord>, String> {
        self.search_scored_internal(query, limit, query_embedding, None, Some(allowed_ids))
    }

    fn search_scored_internal(
        &self,
        query: &str,
        limit: usize,
        query_embedding: Option<&[f32]>,
        node_type: Option<&str>,
        allowed_ids: Option<&HashSet<String>>,
    ) -> Result<Vec<ScoredMemoryRecord>, String> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let tokens = search_tokens(query);
        let mut scored = self
            .load_result()?
            .memories
            .into_iter()
            .filter(|memory| node_type.is_none_or(|kind| memory.node_type == kind))
            .filter(|memory| allowed_ids.is_none_or(|allowed| allowed.contains(memory.id.as_str())))
            .filter_map(|memory| {
                let keyword_score = keyword_score(&memory, &tokens);
                let vector_score =
                    query_embedding
                        .zip(memory.embedding.as_deref())
                        .map(|(query, candidate)| {
                            crate::services::brain::embedding_service::cosine_similarity(
                                query, candidate,
                            )
                            .max(0.0)
                        });
                let score = match vector_score {
                    Some(vector) => keyword_score * 0.55 + vector * 0.45,
                    None => keyword_score,
                };
                (score > 0.05).then_some(ScoredMemoryRecord {
                    record: memory,
                    score,
                    keyword_score,
                    vector_score,
                })
            })
            .collect::<Vec<_>>();

        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.record.id.cmp(&right.record.id))
        });
        scored.truncate(limit.min(100));
        Ok(scored)
    }

    /// Deterministic retrieval for Run preparation. It never initializes or
    /// downloads an embedding model, keeping admission fast and offline.
    pub fn search_keyword(&self, query: &str, limit: usize) -> Vec<SupermemoryRecord> {
        self.search_scored_result(query, limit, None, None)
            .unwrap_or_default()
            .into_iter()
            .map(|hit| hit.record)
            .collect()
    }

    pub fn write_memory(&self, record: SupermemoryRecord) {
        if let Err(error) = self.write_memory_result(record) {
            eprintln!("[SupermemoryStore] {error}");
        }
    }

    pub fn write_memory_result(&self, mut record: SupermemoryRecord) -> Result<(), String> {
        if record.embedding.is_none() {
            record.embedding = best_effort_embedding(&format!(
                "{} {} {}",
                record.name,
                record.summary,
                record.tags.join(" ")
            ));
        }
        self.mutate(|data| {
            if let Some(existing) = data.memories.iter_mut().find(|item| item.id == record.id) {
                *existing = record;
            } else {
                data.memories.push(record);
            }
            refresh_stats(data);
            Ok(())
        })
    }

    /// Atomically replace one Knowledge Document version, its Manifest entry,
    /// and every underlying Knowledge Chunk in a single Store checkpoint.
    pub fn replace_knowledge_document(
        &self,
        document: KnowledgeDocumentRecord,
        records: Vec<SupermemoryRecord>,
    ) -> Result<(), String> {
        self.mutate(|data| {
            let old_ids = data
                .corpus
                .documents
                .iter()
                .filter(|old| {
                    old.document_id == document.document_id
                        || old.source_path == document.source_path
                })
                .flat_map(|old| old.chunks.iter().map(|chunk| chunk.record_id.clone()))
                .collect::<HashSet<_>>();
            data.memories.retain(|memory| {
                !old_ids.contains(&memory.id)
                    && !(memory.node_type == "document_chunk"
                        && memory.file_path == document.source_path)
            });
            data.corpus.documents.retain(|old| {
                old.document_id != document.document_id && old.source_path != document.source_path
            });
            data.memories.extend(records);
            data.corpus.documents.push(document);
            data.corpus
                .documents
                .sort_by(|left, right| left.source_path.cmp(&right.source_path));
            refresh_stats(data);
            Ok(())
        })
    }

    pub fn knowledge_documents(&self) -> Result<Vec<KnowledgeDocumentRecord>, String> {
        Ok(self.load_result()?.corpus.documents)
    }

    pub fn knowledge_document_by_source(
        &self,
        source_path: &str,
    ) -> Result<Option<KnowledgeDocumentRecord>, String> {
        Ok(self
            .load_result()?
            .corpus
            .documents
            .into_iter()
            .find(|document| document.source_path == source_path))
    }

    pub fn delete_knowledge_document(
        &self,
        reference: &str,
    ) -> Result<Option<KnowledgeDocumentRecord>, String> {
        self.mutate(|data| {
            let Some(index) = data.corpus.documents.iter().position(|document| {
                document.document_id == reference || document.source_path == reference
            }) else {
                return Ok(None);
            };
            let document = data.corpus.documents.remove(index);
            let record_ids = document
                .chunks
                .iter()
                .map(|chunk| chunk.record_id.as_str())
                .collect::<HashSet<_>>();
            data.memories.retain(|memory| {
                !record_ids.contains(memory.id.as_str())
                    && !(memory.node_type == "document_chunk"
                        && memory.file_path == document.source_path)
            });
            refresh_stats(data);
            Ok(Some(document))
        })
    }

    pub fn add_log(&self, log_type: &str, message: Option<String>, query: Option<String>) {
        let result = self.mutate(|data| {
            data.logs.push(SupermemoryLogEntry {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                log_type: log_type.to_string(),
                message,
                query,
                response: None,
                tokens_used: None,
                matched_memory_ids: None,
            });
            if data.logs.len() > 500 {
                data.logs = data.logs.split_off(data.logs.len() - 500);
            }
            if log_type == "query" {
                data.stats.total_queries += 1;
            }
            refresh_stats(data);
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("[SupermemoryStore] {error}");
        }
    }

    pub fn get_logs(&self, limit: usize) -> Vec<SupermemoryLogEntry> {
        self.load().logs.into_iter().rev().take(limit).collect()
    }

    fn mutate<T>(
        &self,
        operation: impl FnOnce(&mut SupermemoryStoreData) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = STORE_WRITE_LOCK
            .lock()
            .map_err(|_| "supermemory checkpoint lock is poisoned".to_string())?;
        self.with_checkpoint_lock(|| {
            let mut data = self.load_result()?;
            let output = operation(&mut data)?;
            self.write_checkpoint(&data)?;
            Ok(output)
        })
    }

    fn with_checkpoint_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let parent = self.store_path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        let mut lock_name = self
            .store_path
            .file_name()
            .unwrap_or_default()
            .to_os_string();
        lock_name.push(".lock");
        let lock_path = parent.join(lock_name);
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .map_err(|error| format!("failed to open {}: {error}", lock_path.display()))?;
        let mut lock = fd_lock::RwLock::new(lock_file);
        let _file_guard = lock
            .write()
            .map_err(|error| format!("failed to lock {}: {error}", lock_path.display()))?;
        operation()
    }

    fn write_checkpoint(&self, data: &SupermemoryStoreData) -> Result<(), String> {
        let parent = self.store_path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("failed to create store checkpoint: {error}"))?;
        serde_json::to_writer_pretty(temporary.as_file_mut(), data)
            .map_err(|error| format!("failed to serialize store checkpoint: {error}"))?;
        temporary
            .as_file_mut()
            .flush()
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|error| format!("failed to flush store checkpoint: {error}"))?;
        temporary.persist(&self.store_path).map_err(|error| {
            format!(
                "failed to atomically replace {}: {}",
                self.store_path.display(),
                error.error
            )
        })?;
        Ok(())
    }
}

fn refresh_stats(data: &mut SupermemoryStoreData) {
    data.stats.total_memories = data.memories.len() as u32;
    data.stats.last_sync = chrono::Utc::now().to_rfc3339();
}

fn best_effort_embedding(text: &str) -> Option<Vec<f32>> {
    if text.trim().is_empty() {
        return None;
    }
    crate::services::brain::embedding_service::get_embedding_service()
        .ok()
        .and_then(|service| {
            use crate::services::brain::embedding_service::EmbeddingService;
            service.embed_text(text).ok()
        })
}

fn keyword_score(memory: &SupermemoryRecord, tokens: &[String]) -> f32 {
    if tokens.is_empty() {
        return 0.0;
    }
    let haystack = format!(
        "{} {} {} {} {}",
        memory.name.to_lowercase(),
        memory.summary.to_lowercase(),
        memory.tags.join(" ").to_lowercase(),
        memory.file_path.to_lowercase(),
        memory.source_label.to_lowercase()
    );
    let matched = tokens
        .iter()
        .filter(|token| haystack.contains(token.as_str()))
        .count();
    matched as f32 / tokens.len() as f32
}

fn search_tokens(query: &str) -> Vec<String> {
    let normalized = query.to_lowercase();
    let mut tokens = normalized
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let cjk = normalized
        .chars()
        .filter(|ch| !ch.is_ascii() && !ch.is_whitespace())
        .collect::<Vec<_>>();
    tokens.extend(
        cjk.windows(2)
            .map(|window| window.iter().collect::<String>()),
    );
    tokens.sort();
    tokens.dedup();
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::memory::{KnowledgeChunkRecord, KnowledgeDocumentRecord};

    fn memory(id: &str, source: &str, text: &str) -> SupermemoryRecord {
        SupermemoryRecord {
            id: id.to_string(),
            file_path: source.to_string(),
            name: "chunk".to_string(),
            summary: text.to_string(),
            tags: vec!["knowledge".to_string()],
            complexity: "knowledge_node".to_string(),
            node_type: "document_chunk".to_string(),
            source_graph_path: String::new(),
            source_label: "source#L1-L1".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            embedding: None,
        }
    }

    fn document(hash: &str, record_id: &str) -> KnowledgeDocumentRecord {
        KnowledgeDocumentRecord {
            document_id: "doc-1".to_string(),
            source_path: "workspace/doc.md".to_string(),
            display_name: "doc.md".to_string(),
            content_hash: hash.to_string(),
            revision: 1,
            format: "markdown".to_string(),
            byte_size: 10,
            global_context: "doc".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            chunks: vec![KnowledgeChunkRecord {
                record_id: record_id.to_string(),
                chunk_index: 0,
                start_line: 1,
                end_line: 1,
                start_paragraph: 1,
                end_paragraph: 1,
                start_page: None,
                end_page: None,
                heading_path: Vec::new(),
                citation: "workspace/doc.md#L1-L1".to_string(),
            }],
        }
    }

    #[test]
    fn search_tokenizer_supports_cjk_without_whitespace() {
        let tokens = search_tokens("完善系统架构功能");
        assert!(tokens.contains(&"系统".to_string()));
        assert!(tokens.contains(&"架构".to_string()));
    }

    #[test]
    fn document_replacement_removes_old_chunks_in_same_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let store = SupermemoryStore::from_path(temp.path().join("memory.json"));
        store
            .replace_knowledge_document(
                document("old", "old-record"),
                vec![memory("old-record", "workspace/doc.md", "old content")],
            )
            .unwrap();
        store
            .replace_knowledge_document(
                document("new", "new-record"),
                vec![memory("new-record", "workspace/doc.md", "new content")],
            )
            .unwrap();

        let data = store.load_result().unwrap();
        assert_eq!(data.corpus.documents.len(), 1);
        assert_eq!(data.memories.len(), 1);
        assert_eq!(data.memories[0].id, "new-record");
    }

    #[test]
    fn scored_search_exposes_keyword_score_without_embeddings() {
        let temp = tempfile::tempdir().unwrap();
        let store = SupermemoryStore::from_path(temp.path().join("memory.json"));
        store
            .replace_knowledge_document(
                document("hash", "record"),
                vec![memory(
                    "record",
                    "workspace/doc.md",
                    "atomic corpus manifest",
                )],
            )
            .unwrap();
        let hits = store
            .search_scored_result("corpus manifest", 5, None, Some("document_chunk"))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score > 0.9);
        assert!(hits[0].vector_score.is_none());
    }
}
