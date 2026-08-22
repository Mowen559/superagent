use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};

/// 原生图谱搜索中枢 (Native Graph Search Organ)
pub struct NativeSearchGraphTool { pub workspace_root: std::path::PathBuf }
impl NativeSearchGraphTool { pub fn new(w: std::path::PathBuf) -> Self { Self { workspace_root: w } } }

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
                "Action": {
                    "type": "string",
                    "enum": ["search", "get_callers", "get_callees", "get_impact", "query_openclaw_state"],
                    "description": "The graph traversal action. Use search to find a node ID, then use get_callers/get_callees with that ID."
                },
                "query": {
                    "type": "string",
                    "description": "The term to search for (for search), or the Node ID (for get_callers and get_callees)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of nodes to return. Default is 20."
                },
                "project_path": {
                    "type": "string",
                    "description": "Optional project root directory path."
                }
            },
            "required": ["Action", "query"]
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
        let action = args
            .get("Action")
            .and_then(|v| v.as_str())
            .unwrap_or("search");
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("Missing query")?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let path_input = args
            .get("project_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let cwd = self.workspace_root.clone();
        let target_path = if path_input.is_empty() {
            cwd.to_string_lossy().to_string()
        } else {
            path_input.to_string()
        };

        let db_path_str = if target_path.ends_with(".db") {
            target_path.clone()
        } else {
            let app_paths = crate::config::app_paths::AppPaths::new();
            let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
            let db_path = config.codebase_memory_db_path(Some(&target_path));
            db_path.to_string_lossy().to_string()
        };

        let nodes_result = match action {
            "get_callers" => {
                crate::services::graph_service::get_callers(&db_path_str, query, limit)
            }
            "get_callees" => {
                crate::services::graph_service::get_callees(&db_path_str, query, limit)
            }
            _ => crate::services::graph_service::search_nodes(&db_path_str, query, limit),
        };

        match nodes_result {
            Ok(nodes) => {
                let nodes_json: Vec<Value> = nodes
                    .into_iter()
                    .map(|n| {
                        json!({
                            "id": n.id,
                            "label": n.label,
                            "node_type": n.node_type,
                            "file_path": n.file_path,
                            "language": n.language
                        })
                    })
                    .collect();
                Ok(json!({ "status": "success", "nodes": nodes_json }))
            }
            Err(e) => Err(format!("Native graph search failed: {}", e)),
        }
    }
}
