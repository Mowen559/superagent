use crate::models::memory::SupermemoryRecord;
use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::supermemory_store::SupermemoryStore;
use serde_json::{json, Value};

/// 原生超级记忆海马体 (Native SuperMemory Organ)
pub struct NativeMemoryTool {
    pub workspace_root: std::path::PathBuf,
}
impl NativeMemoryTool {
    pub fn new(w: std::path::PathBuf) -> Self {
        Self { workspace_root: w }
    }
}

impl ToolExecutor for NativeMemoryTool {
    fn name(&self) -> &'static str {
        "super_memory"
    }

    fn description(&self) -> &'static str {
        "Store, retrieve, or search project-level long-term core memory and architecture guidelines using SupermemoryStore. Actions: 'store', 'retrieve', 'search', 'list'."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["store", "retrieve", "search", "list"],
                    "description": "Action to perform: 'store' to record, 'retrieve' to fetch by key, 'search' for semantic/keyword search, 'list' for recent entries."
                },
                "key": {
                    "type": "string",
                    "description": "The key or title for the memory (required for 'store' and 'retrieve')."
                },
                "query": {
                    "type": "string",
                    "description": "The search query (required for 'search')."
                },
                "content": {
                    "type": "string",
                    "description": "The payload/content to store (required for 'store')."
                }
            },
            "required": ["action"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: super_memory]
- Role: Native Long-Term Memory (超级记忆海马体)
- Usage: Query project architecture guidelines, design decisions, and distilled lessons using action='search' or 'retrieve'. Persist newly verified architectural decisions or critical patterns using action='store'.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("search");

        // 智能定位主工程 supermemory 路径 (沙箱逃逸自愈定位)
        let resolved_project_path = {
            if self.workspace_root.join(".supermemory").exists() {
                self.workspace_root.clone()
            } else if let Some(parent) = self.workspace_root.parent() {
                let baseline_file = parent.join("baseline.json");
                if baseline_file.exists() {
                    if let Ok(content) = std::fs::read_to_string(&baseline_file) {
                        if let Ok(v) = serde_json::from_str::<Value>(&content) {
                            if let Some(p) = v.get("projectPath").and_then(|p| p.as_str()) {
                                let norm = p.trim_start_matches(r"\\?\");
                                std::path::PathBuf::from(norm)
                            } else {
                                self.workspace_root.clone()
                            }
                        } else {
                            self.workspace_root.clone()
                        }
                    } else {
                        self.workspace_root.clone()
                    }
                } else {
                    self.workspace_root.clone()
                }
            } else {
                self.workspace_root.clone()
            }
        };

        let app_paths = crate::config::app_paths::AppPaths::new();
        let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
        let project_path_str = resolved_project_path.to_string_lossy().to_string();
        let store = SupermemoryStore::new(&config, Some(&project_path_str));

        match action {
            "store" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing required 'key' for action='store'".to_string())?;
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing required 'content' for action='store'".to_string())?;

                let record = SupermemoryRecord {
                    id: key.to_string(),
                    name: key.to_string(),
                    summary: content.to_string(),
                    tags: vec!["superagent".to_string(), "architectural_decision".to_string()],
                    file_path: "".to_string(),
                    complexity: "core".to_string(),
                    node_type: "decision".to_string(),
                    source_graph_path: "".to_string(),
                    source_label: "SuperAgentReflexion".to_string(),
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
                    "\x1b[35m[Native Memory] 🧠 Stored long-term memory to {}: {}\x1b[0m",
                    project_path_str, key
                );
                Ok(json!({ "status": "stored", "key": key, "store_target": project_path_str }))
            }
            "retrieve" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing required 'key' for action='retrieve'".to_string())?;
                let results = store.search(key, 5);
                let json_results: Vec<Value> = results
                    .into_iter()
                    .map(|r| {
                        json!({
                            "key": r.id,
                            "name": r.name,
                            "content": r.summary,
                            "tags": r.tags,
                            "updatedAt": r.updated_at
                        })
                    })
                    .collect();
                Ok(json!({ "status": "retrieved", "query": key, "results": json_results }))
            }
            "search" => {
                let query = args
                    .get("query")
                    .or_else(|| args.get("key"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let results = store.search(query, 5);
                let json_results: Vec<Value> = results
                    .into_iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "name": r.name,
                            "summary": r.summary,
                            "nodeType": r.node_type,
                            "tags": r.tags,
                            "updatedAt": r.updated_at
                        })
                    })
                    .collect();
                Ok(json!({ "status": "success", "results": json_results }))
            }
            "list" => {
                let data = store.load();
                let recent: Vec<Value> = data
                    .memories
                    .iter()
                    .rev()
                    .take(10)
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "name": r.name,
                            "summary": r.summary,
                            "nodeType": r.node_type,
                            "updatedAt": r.updated_at
                        })
                    })
                    .collect();
                Ok(json!({ "total": data.memories.len(), "recent": recent }))
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }
}
