use serde_json::{json, Value};
use std::path::PathBuf;

use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::domains::version_control::repo_map::RepoMapEngine;

pub struct RepoMapTool {
    pub workspace_root: PathBuf,
}

impl RepoMapTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for RepoMapTool {
    fn name(&self) -> &'static str {
        "repo_map_tool"
    }

    fn description(&self) -> &'static str {
        "Inspect an Aider-style compressed Repository Map of the codebase. It computes PageRank on symbol call-graphs to return the most critical classes, interfaces, methods, and types with their file paths and line numbers in under 1,000 tokens."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "focus_path": {
                    "type": "string",
                    "description": "Optional subdirectory or file prefix to focus the map on (e.g. 'iam-service', 'engine-service/src/api')."
                },
                "query": {
                    "type": "string",
                    "description": "Optional keyword or concept to boost relevant symbols in the PageRank ranking (e.g. 'auth', 'spider', 'user')."
                },
                "max_symbols": {
                    "type": "integer",
                    "description": "Maximum number of top-ranked symbols to include (default: 35).",
                    "minimum": 5,
                    "maximum": 100
                }
            }
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: repo_map_tool]
- Role: Repository Semantic Skeleton (Aider-style PageRank Map)
- Usage: Call `repo_map_tool` to get a 30,000-foot view of relevant classes and methods before guessing or doing expensive broad searches.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let focus_path = args.get("focus_path").and_then(|v| v.as_str());
        let query = args.get("query").and_then(|v| v.as_str());
        let max_symbols = args.get("max_symbols").and_then(|v| v.as_u64()).unwrap_or(35) as usize;

        let app_paths = crate::config::app_paths::AppPaths::new();
        let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
        let cwd_str = self.workspace_root.to_string_lossy().to_string();
        let db_path = config.codebase_memory_db_path(Some(&cwd_str));

        let engine = RepoMapEngine::new(db_path);
        let repo_map = engine.generate_map(focus_path, query, max_symbols)?;

        Ok(json!({
            "status": "success",
            "focus_path": focus_path,
            "query": query,
            "repo_map": repo_map
        }))
    }
}
