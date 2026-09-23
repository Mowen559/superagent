//! 文件查找与内容全文检索工具

use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::infrastructure::fs_path::resolve_safe_path;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct FindByNameTool {
    pub workspace_root: PathBuf,
}

impl FindByNameTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for FindByNameTool {
    fn name(&self) -> &'static str {
        "find_by_name"
    }

    fn description(&self) -> &'static str {
        "Search for files and subdirectories within a specified directory by matching filename pattern. Respects .gitignore automatically."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "SearchDirectory": { "type": "string", "description": "The directory to search within." },
                "Pattern": { "type": "string", "description": "The substring to search for in filenames." }
            },
            "required": ["SearchDirectory", "Pattern"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let dir = args
            .get("SearchDirectory")
            .or_else(|| args.get("dir"))
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let pattern = args
            .get("Pattern")
            .and_then(|v| v.as_str())
            .ok_or("Missing Pattern")?;

        let path = resolve_safe_path(dir, &self.workspace_root)?;
        if !path.is_dir() {
            return Err(format!("Directory not found: {}", dir));
        }

        let mut results = vec![];
        let walker = ignore::WalkBuilder::new(&path).build();
        for result in walker.flatten() {
            let file_name = result.file_name().to_string_lossy();
            if file_name.contains(pattern) {
                results.push(result.path().to_string_lossy().into_owned());
                if results.len() >= 100 {
                    break;
                }
            }
        }

        Ok(json!({ "matches": results }))
    }
}

pub struct GrepSearchTool {
    pub workspace_root: PathBuf,
}

impl GrepSearchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for GrepSearchTool {
    fn name(&self) -> &'static str {
        "grep_search"
    }

    fn description(&self) -> &'static str {
        "Find exact pattern matches within files in a directory. Returns file names and line numbers. Respects .gitignore."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "SearchPath": { "type": "string", "description": "The directory path to search." },
                "Query": { "type": "string", "description": "The exact string pattern to look for." }
            },
            "required": ["SearchPath", "Query"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let dir = args
            .get("SearchPath")
            .or_else(|| args.get("path"))
            .or_else(|| args.get("dir"))
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let query = args
            .get("Query")
            .and_then(|v| v.as_str())
            .ok_or("Missing Query")?;

        let path = resolve_safe_path(dir, &self.workspace_root)?;
        if !path.exists() {
            return Err(format!("Path not found: {}", dir));
        }

        let mut results = vec![];

        let mut process_file = |file_path: &Path| {
            if let Ok(content) = std::fs::read_to_string(file_path) {
                for (i, line) in content.lines().enumerate() {
                    if line.contains(query) {
                        results.push(format!(
                            "{}:{}: {}",
                            file_path.display(),
                            i + 1,
                            line.trim()
                        ));
                        if results.len() >= 100 {
                            return true;
                        }
                    }
                }
            }
            false
        };

        if path.is_file() {
            process_file(&path);
        } else {
            let walker = ignore::WalkBuilder::new(&path).build();
            for entry in walker.flatten() {
                if entry.file_type().map_or(false, |ft| ft.is_file())
                    && process_file(entry.path())
                {
                    break;
                }
            }
        }

        Ok(json!({ "matches": results }))
    }
}
