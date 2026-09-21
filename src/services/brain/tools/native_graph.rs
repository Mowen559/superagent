use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::{json, Value};

/// 原生图谱搜索中枢 (Native Graph Search Organ)
pub struct NativeSearchGraphTool {
    pub workspace_root: std::path::PathBuf,
}
impl NativeSearchGraphTool {
    pub fn new(w: std::path::PathBuf) -> Self {
        Self { workspace_root: w }
    }
}

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
                    "enum": [
                        "search",
                        "get_callers",
                        "get_callees",
                        "get_impact",
                        "trace_path",
                        "get_schema",
                        "get_code_snippet",
                        "get_dead_code",
                        "query_openclaw_state"
                    ],
                    "description": "The graph traversal action. E.g. search, get_callers, get_callees, get_impact, trace_path, get_schema, get_code_snippet, get_dead_code."
                },
                "query": {
                    "type": "string",
                    "description": "The symbol/term to search for, or Node ID/Function Name for callers/callees/trace_path/code_snippet."
                },
                "direction": {
                    "type": "string",
                    "enum": ["inbound", "outbound", "both"],
                    "description": "Direction for trace_path. Default is outbound."
                },
                "depth": {
                    "type": "integer",
                    "description": "Traversal depth for trace_path or get_impact. Default is 3."
                },
                "include_neighbors": {
                    "type": "boolean",
                    "description": "Whether to include caller/callee neighbors in get_code_snippet. Default is true."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of items to return. Default is 20."
                },
                "project_path": {
                    "type": "string",
                    "description": "Optional project root directory path."
                }
            },
            "required": ["Action"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: search_graph]
- Role: Sensory Organ (感知中枢)
- Priority: CRITICAL (Highest Priority for codebase exploration)
- Usage: When exploring a codebase, you MUST prioritize this native graph search over blind grep/find/cat.
- Capabilities: search, callers/callees, impact analysis, multi-hop trace_path, schema metadata, code snippets with neighbors, and dead code detection.
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
            .unwrap_or("");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
        let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("outbound");
        let include_neighbors = args.get("include_neighbors").and_then(|v| v.as_bool()).unwrap_or(true);

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

        match action {
            "get_schema" => {
                let schema = crate::domains::knowledge_graph::graph_service::get_graph_schema(&db_path_str)
                    .map_err(|e| format!("Get graph schema failed: {}", e))?;
                Ok(json!({ "status": "success", "schema": schema }))
            }
            "get_dead_code" => {
                let dead = crate::domains::knowledge_graph::graph_service::get_dead_code(&db_path_str, limit)
                    .map_err(|e| format!("Get dead code failed: {}", e))?;
                Ok(json!({ "status": "success", "dead_code": dead }))
            }
            "trace_path" => {
                if query.is_empty() {
                    return Err("trace_path requires 'query' as root symbol name".to_string());
                }
                let trace = crate::domains::knowledge_graph::graph_service::trace_path(
                    &db_path_str,
                    query,
                    direction,
                    depth,
                    "calls",
                )
                .map_err(|e| format!("Trace path failed: {}", e))?;
                Ok(json!({ "status": "success", "trace": trace }))
            }
            "get_code_snippet" => {
                if query.is_empty() {
                    return Err("get_code_snippet requires 'query' as symbol qualified_name".to_string());
                }
                let snippet = crate::domains::knowledge_graph::graph_service::get_code_snippet(
                    &db_path_str,
                    query,
                    include_neighbors,
                    Some(std::path::Path::new(&target_path)),
                )
                .map_err(|e| format!("Get code snippet failed: {}", e))?;
                Ok(json!({ "status": "success", "snippet": snippet }))
            }
            "get_impact" => {
                if query.is_empty() {
                    return Err("get_impact requires 'query' as target node id".to_string());
                }
                let nodes = crate::domains::knowledge_graph::graph_service::get_impact(&db_path_str, query, depth)
                    .map_err(|e| format!("Get impact failed: {}", e))?;
                Ok(json!({ "status": "success", "impact_nodes": nodes }))
            }
            "get_callers" => {
                if query.is_empty() {
                    return Err("get_callers requires 'query' as target node id".to_string());
                }
                let nodes = crate::domains::knowledge_graph::graph_service::get_callers(&db_path_str, query, limit)
                    .map_err(|e| format!("Get callers failed: {}", e))?;
                Ok(json!({ "status": "success", "nodes": nodes }))
            }
            "get_callees" => {
                if query.is_empty() {
                    return Err("get_callees requires 'query' as source node id".to_string());
                }
                let nodes = crate::domains::knowledge_graph::graph_service::get_callees(&db_path_str, query, limit)
                    .map_err(|e| format!("Get callees failed: {}", e))?;
                Ok(json!({ "status": "success", "nodes": nodes }))
            }
            _ => {
                let nodes = crate::domains::knowledge_graph::graph_service::search_nodes(&db_path_str, query, limit)
                    .map_err(|e| format!("Native graph search failed: {}", e))?;
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
        }
    }
}
