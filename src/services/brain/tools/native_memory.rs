use crate::models::memory::SupermemoryRecord;
use crate::services::brain::executor::ToolExecutor;
use crate::services::supermemory_store::SupermemoryStore;
use serde_json::{json, Value};

/// 原生超级记忆海马体 (Native SuperMemory Organ)
pub struct NativeMemoryTool { pub workspace_root: std::path::PathBuf }
impl NativeMemoryTool { pub fn new(w: std::path::PathBuf) -> Self { Self { workspace_root: w } } }

impl ToolExecutor for NativeMemoryTool {
    fn name(&self) -> &'static str {
        "super_memory"
    }

    fn description(&self) -> &'static str {
        "Store or retrieve long-term core memory using the native SupermemoryStore."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Either 'store' or 'retrieve'."
                },
                "key": {
                    "type": "string",
                    "description": "The key or title for the memory."
                },
                "content": {
                    "type": "string",
                    "description": "The payload to store (only for 'store' action)."
                }
            },
            "required": ["action", "key"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: super_memory]
- Role: Memory System (海马体记忆)
- Usage: Use `store`/`retrieve` to persist long-term core memory (e.g., design decisions, global state). Your subconscious will automatically load it.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("retrieve");
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let app_paths = crate::config::app_paths::AppPaths::new();
        let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
        let store = SupermemoryStore::new(&config, None);

        match action {
            "store" => {
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let record = SupermemoryRecord {
                    id: key.clone(),
                    name: key.clone(),
                    summary: content.clone(),
                    tags: vec!["superagent".to_string()],
                    file_path: "".to_string(),
                    complexity: "".to_string(),
                    node_type: "".to_string(),
                    source_graph_path: "".to_string(),
                    source_label: "".to_string(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    embedding: crate::services::brain::embedding_service::get_embedding_service()
                        .ok()
                        .and_then(|svc| {
                            use crate::services::brain::embedding_service::EmbeddingService;
                            let combined = format!("{} {}", key, content);
                            svc.embed_text(&combined).ok()
                        }),
                };
                store.write_memory(record);
                eprintln!(
                    "\x1b[35m[Native Memory] 🧠 Stored long-term memory: {}\x1b[0m",
                    key
                );
                Ok(json!({ "status": "stored", "key": key }))
            }
            "retrieve" => {
                eprintln!(
                    "\x1b[35m[Native Memory] 🧠 Retrieving long-term memory: {}\x1b[0m",
                    key
                );
                let results = store.search(&key, 5);
                let json_results: Vec<Value> = results
                    .into_iter()
                    .map(|r| {
                        json!({
                            "key": r.id,
                            "content": r.summary,
                            "timestamp": r.updated_at
                        })
                    })
                    .collect();
                Ok(json!({ "status": "retrieved", "results": json_results }))
            }
            _ => Err("Invalid action".to_string()),
        }
    }
}
