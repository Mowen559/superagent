use serde_json::{json, Value};
use crate::services::brain::executor::ToolExecutor;
use crate::services::supermemory_store::SupermemoryStore;
use crate::config::runtime::RuntimeConfig;
use crate::config::app_paths::AppPaths;
use crate::models::memory::SupermemoryRecord;

pub struct KnowledgeSearchTool;

impl ToolExecutor for KnowledgeSearchTool {
    fn name(&self) -> &'static str {
        "knowledge_search_tool"
    }

    fn description(&self) -> &'static str {
        "Search the knowledge base (SuperMemory) for facts, concepts, or past documents. Use this FIRST when answering factual questions or recalling long-term knowledge."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query, concepts, or keywords to look up in the knowledge base."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Default is 5."
                }
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        let app_paths = AppPaths::new();
        let config = RuntimeConfig::from_env(&app_paths);
        let store = SupermemoryStore::new(&config, None);

        let results = store.search(query, limit);

        if results.is_empty() {
            return Ok(json!({ "message": "No matching knowledge found in the database." }));
        }

        let formatted: Vec<Value> = results.into_iter().map(|r| {
            json!({
                "id": r.id,
                "name": r.name,
                "content": r.summary,
                "tags": r.tags,
                "source": r.file_path,
                "updated_at": r.updated_at
            })
        }).collect();

        Ok(json!({
            "total_found": formatted.len(),
            "results": formatted
        }))
    }
}

pub struct KnowledgeIngestTool;

impl ToolExecutor for KnowledgeIngestTool {
    fn name(&self) -> &'static str {
        "knowledge_ingest_tool"
    }

    fn description(&self) -> &'static str {
        "Ingest a long text or document into the knowledge base by chunking it with a contextual prefix. Useful for reading entire books or massive files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The local path of the file to ingest."
                },
                "global_context": {
                    "type": "string",
                    "description": "A 1-3 sentence summary of the entire document to prepend to every chunk to preserve global semantic meaning."
                }
            },
            "required": ["file_path", "global_context"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let file_path = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
        let global_context = args.get("global_context").and_then(|v| v.as_str()).unwrap_or("");

        if file_path.is_empty() || global_context.is_empty() {
            return Err("Both file_path and global_context are required.".into());
        }

        let content = std::fs::read_to_string(file_path).map_err(|e| format!("Failed to read file: {}", e))?;

        let app_paths = AppPaths::new();
        let config = RuntimeConfig::from_env(&app_paths);
        let store = SupermemoryStore::new(&config, None);

        use crate::services::brain::contextual_chunker::ContextualChunker;
        let chunker = ContextualChunker::new(1000); // chunk size approx 1000 chars

        let chunks = chunker.chunk_text(&content, global_context);
        let chunk_count = chunks.len();

        let doc_name = std::path::Path::new(file_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        for chunk in chunks {
            let chunk_content = chunk.to_llm_string();
            let record = SupermemoryRecord {
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("{} (Part {})", doc_name, chunk.chunk_index + 1),
                summary: chunk_content,
                tags: vec!["knowledge".to_string(), "ingested".to_string()],
                complexity: "knowledge_node".to_string(),
                node_type: "document_chunk".to_string(),
                source_graph_path: "".to_string(),
                source_label: "Knowledge Ingest".to_string(),
                file_path: file_path.to_string(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                embedding: None,
            };
            store.write_memory(record);
        }

        Ok(json!({
            "status": "success",
            "message": format!("Successfully chunked and ingested document into {} pieces.", chunk_count)
        }))
    }
}
