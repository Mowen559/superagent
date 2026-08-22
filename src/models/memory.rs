use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupermemoryRecord {
    pub id: String,
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub complexity: String,
    #[serde(default)]
    pub node_type: String,
    #[serde(default)]
    pub source_graph_path: String,
    #[serde(default)]
    pub source_label: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupermemoryLogEntry {
    pub id: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub log_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_memory_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupermemoryStoreData {
    pub version: u32,
    pub memories: Vec<SupermemoryRecord>,
    pub logs: Vec<SupermemoryLogEntry>,
    pub stats: SupermemoryStats,
    #[serde(default)]
    pub corpus: KnowledgeCorpusState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCorpusState {
    #[serde(default = "default_corpus_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub documents: Vec<KnowledgeDocumentRecord>,
}

impl Default for KnowledgeCorpusState {
    fn default() -> Self {
        Self {
            schema_version: default_corpus_schema_version(),
            documents: Vec::new(),
        }
    }
}

fn default_corpus_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocumentRecord {
    pub document_id: String,
    pub source_path: String,
    pub display_name: String,
    pub content_hash: String,
    pub revision: u64,
    pub format: String,
    pub byte_size: u64,
    pub global_context: String,
    pub updated_at: String,
    #[serde(default)]
    pub chunks: Vec<KnowledgeChunkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeChunkRecord {
    pub record_id: String,
    pub chunk_index: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub start_paragraph: usize,
    pub end_paragraph: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_page: Option<usize>,
    #[serde(default)]
    pub heading_path: Vec<String>,
    pub citation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupermemoryStats {
    pub total_queries: u32,
    pub active_agents: u32,
    pub last_sync: String,
    pub total_memories: u32,
}

impl Default for SupermemoryStoreData {
    fn default() -> Self {
        Self {
            version: 1,
            memories: Vec::new(),
            logs: Vec::new(),
            corpus: KnowledgeCorpusState::default(),
            stats: SupermemoryStats {
                total_queries: 0,
                active_agents: 0,
                last_sync: chrono::Utc::now().to_rfc3339(),
                total_memories: 0,
            },
        }
    }
}
