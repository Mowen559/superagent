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

/// AST 级别结构化代码搜索与模式重写工具 (Powered by ast-grep)
pub struct AstGrepTool {
    pub workspace_root: PathBuf,
}

impl AstGrepTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for AstGrepTool {
    fn name(&self) -> &'static str {
        "ast_grep"
    }

    fn description(&self) -> &'static str {
        "Perform AST-level structural code search or rewriting using ast-grep patterns. Tolerates whitespace/newline variations. Use $VAR for single nodes and $$$VARS for multiple statements/parameters."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "The AST pattern to search for (e.g. 'fn $NAME($$$ARGS) -> Result<$T> { $$$BODY }')" },
                "rewrite": { "type": "string", "description": "Optional rewrite replacement pattern (e.g. 'fn $NAME($$$ARGS) -> Result<$T, AppError> { $$$BODY }')" },
                "path": { "type": "string", "description": "Target file or directory path relative to workspace or absolute." },
                "language": { "type": "string", "description": "Optional language selector: rust, typescript, python, javascript, etc." }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or("Missing pattern")?;
        let rewrite = args.get("rewrite").and_then(|v| v.as_str());
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let language = args.get("language").and_then(|v| v.as_str());

        let target_path = resolve_safe_path(path_str, &self.workspace_root)?;
        if !target_path.exists() {
            return Err(format!("Target path '{}' does not exist", path_str));
        }

        let mut cmd = std::process::Command::new("ast-grep");
        cmd.arg("run");
        cmd.arg("--pattern").arg(pattern);

        if let Some(rw) = rewrite {
            super::diagnostics::record_shadow_backup(&self.workspace_root, &target_path, "ast_grep_rewrite");
            cmd.arg("--rewrite").arg(rw);
            cmd.arg("--update-all");
        } else {
            cmd.arg("--json=stream");
        }

        if let Some(lang) = language {
            cmd.arg("--lang").arg(lang);
        }

        cmd.arg(target_path.to_string_lossy().to_string());
        cmd.current_dir(&self.workspace_root);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let output = cmd.output().map_err(|e| format!("Failed to execute ast-grep: {}. Make sure ast-grep is on PATH.", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        if rewrite.is_some() {
            Ok(json!({
                "status": "success",
                "mode": "rewrite",
                "message": format!("AST rewrite executed successfully on '{}'", target_path.display()),
                "output": stdout.trim()
            }))
        } else {
            let mut matches = Vec::new();
            for line in stdout.lines() {
                if let Ok(item) = serde_json::from_str::<Value>(line) {
                    matches.push(item);
                }
            }
            if matches.is_empty() && !stdout.trim().is_empty() {
                Ok(json!({ "matches_raw": stdout.trim(), "status": "success" }))
            } else {
                Ok(json!({ "matches": matches, "count": matches.len(), "status": "success" }))
            }
        }
    }
}
