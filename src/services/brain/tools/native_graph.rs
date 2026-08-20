use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};

/// 原生图谱搜索中枢 (Native Graph Search Organ)
pub struct NativeSearchGraphTool;

impl ToolExecutor for NativeSearchGraphTool {
    fn name(&self) -> &'static str {
        "search_graph"
    }

    fn description(&self) -> &'static str {
        "Directly query the native Knowledge Graph SQLite database via hub's graph_service."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The term or symbol to search for in the codebase graph."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of nodes to return. Default is 20."
                },
                "project_path": {
                    "type": "string",
                    "description": "Optional project root directory path to search graph for."
                }
            },
            "required": ["query"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: search_graph]
- Role: Sensory Organ (感知中枢)
- Priority: CRITICAL (Highest Priority for codebase exploration)
- Usage: When exploring a codebase, you MUST prioritize this native graph search over blind grep/find/cat.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let query = args.get("query").and_then(|v| v.as_str()).ok_or("Missing query")?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let path_input = args.get("project_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let target_path = if path_input.is_empty() {
            cwd.to_string_lossy().to_string()
        } else {
            path_input.to_string()
        };

        eprintln!("\x1b[32m[Native Sensory] 👁️ Querying Knowledge Graph for: {} in {}\x1b[0m", query, target_path);

        let db_path_str = if target_path.ends_with(".db") {
            target_path.clone()
        } else {
            let app_paths = crate::config::app_paths::AppPaths::new();
            let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
            let db_path = config.codebase_memory_db_path(Some(&target_path));
            db_path.to_string_lossy().to_string()
        };

        match crate::services::graph_service::search_nodes(&db_path_str, query, limit) {
            Ok(nodes) => {
                let nodes_json: Vec<Value> = nodes.into_iter().map(|n| {
                    json!({
                        "id": n.id,
                        "label": n.label,
                        "node_type": n.node_type,
                        "file_path": n.file_path,
                        "language": n.language
                    })
                }).collect();
                Ok(json!({ "status": "success", "nodes": nodes_json }))
            },
            Err(e) => Err(format!("Native graph search failed: {}", e)),
        }
    }
}
