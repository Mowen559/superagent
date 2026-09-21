use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::domains::knowledge_graph::understand_service;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 项目代码画像与架构感知中枢 (Project Architecture & Code Intelligence Organ)
pub struct ProjectIndexTool {
    pub workspace_root: PathBuf,
}

impl ProjectIndexTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn resolve_db_path(&self, project_path_arg: Option<&str>) -> (String, bool) {
        let target_path = project_path_arg
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.workspace_root.clone());

        let app_paths = crate::config::app_paths::AppPaths::new();
        let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
        let db_path = config.codebase_memory_db_path(Some(&target_path.to_string_lossy()));
        let exists = db_path.exists();
        (db_path.to_string_lossy().to_string(), exists)
    }
}

impl ToolExecutor for ProjectIndexTool {
    fn name(&self) -> &'static str {
        "project_index"
    }

    fn description(&self) -> &'static str {
        "Inspect project-level architecture overview, cross-module dependency topology, high-centrality files, or single-file symbol/reference graph. Enables proactive capability & codebase awareness."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": {
                    "type": "string",
                    "enum": [
                        "inspect_overview",
                        "inspect_file",
                        "check_index_status"
                    ],
                    "description": "The architectural inspection action to perform: 'inspect_overview' for global module dependency topology, 'inspect_file' for incoming/outgoing relations of a specific file, or 'check_index_status' to check SQLite index health."
                },
                "FilePath": {
                    "type": "string",
                    "description": "Relative file path within the project (required for 'inspect_file', e.g. 'iam-service/src/main/java/com/deepcloud/iam/controller/AuthController.java')."
                },
                "ProjectPath": {
                    "type": "string",
                    "description": "Optional project root directory path. Defaults to current workspace."
                }
            },
            "required": ["Action"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: project_index]
- Role: Primary Architecture Sensory Organ (架构感知器官)
- Priority: HIGHEST for understanding architecture before making edits.
- Philosophy: Before writing code, use `project_index(Action='inspect_overview')` to map module dependencies, or `project_index(Action='inspect_file')` to check callers/callees.
- Integration: Perfectly pairs with `ask_user` when trade-offs appear.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args
            .get("Action")
            .and_then(Value::as_str)
            .unwrap_or("inspect_overview");

        let project_path_opt = args.get("ProjectPath").and_then(Value::as_str);
        let (db_path, db_exists) = self.resolve_db_path(project_path_opt);

        match action {
            "check_index_status" => {
                let primary_lang = if db_exists {
                    understand_service::detect_primary_language(&db_path).unwrap_or_else(|_| "unknown".to_string())
                } else {
                    "unindexed".to_string()
                };

                Ok(json!({
                    "status": "success",
                    "database_path": db_path,
                    "database_exists": db_exists,
                    "primary_language": primary_lang,
                    "message": if db_exists {
                        "Index is ready for deep architectural comprehension."
                    } else {
                        "Database not found. Run analysis or use search_graph to build/verify."
                    }
                }))
            }
            "inspect_overview" => {
                if !db_exists {
                    return Ok(json!({
                        "status": "index_missing",
                        "database_path": db_path,
                        "message": "Project index database has not been built yet. You can inspect directory structure with list_dir and find_by_name directly."
                    }));
                }

                let overview = understand_service::understand_project(&db_path)
                    .map_err(|e| format!("Failed to understand project: {}", e))?;

                let primary_lang = understand_service::detect_primary_language(&db_path)
                    .unwrap_or_else(|_| "unknown".to_string());

                Ok(json!({
                    "status": "success",
                    "primary_language": primary_lang,
                    "project_overview": overview
                }))
            }
            "inspect_file" => {
                let file_path = args
                    .get("FilePath")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Missing required parameter 'FilePath' for inspect_file".to_string())?;

                if !db_exists {
                    return Ok(json!({
                        "status": "index_missing",
                        "file": file_path,
                        "message": "Index missing. Fallback to view_outline or read_file to inspect symbols."
                    }));
                }

                let file_intel = understand_service::understand_file(&db_path, file_path)
                    .map_err(|e| format!("Failed to inspect file {}: {}", file_path, e))?;

                Ok(json!({
                    "status": "success",
                    "file_intelligence": file_intel
                }))
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }
}
