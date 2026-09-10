//! 文件系统工具集 (Filesystem Tools)
//! 为大脑 B (Superagent) 赋予文件读写与修改的物理操作能力

use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

use std::path::PathBuf;

fn is_path_safe(path: &Path, workspace_root: &Path) -> bool {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };

    let normalized = match std::fs::canonicalize(&absolute_path) {
        Ok(p) => p,
        Err(_) => {
            if let Some(parent) = absolute_path.parent() {
                if let Ok(p) = std::fs::canonicalize(parent) {
                    p.join(absolute_path.file_name().unwrap_or_default())
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }
    };

    let cwd_canonical =
        std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    normalized.starts_with(cwd_canonical)
}

fn require_safe_path(path: &Path, operation: &str, workspace_root: &Path) -> Result<(), String> {
    if is_path_safe(path, workspace_root) {
        Ok(())
    } else {
        Err(format!(
            "Security Error: {} is outside the workspace",
            operation
        ))
    }
}

/// 文件读取工具 (View File)
pub struct ReadFileTool {
    pub workspace_root: PathBuf,
}

impl ReadFileTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for ReadFileTool {
    fn name(&self) -> &'static str {
        "view_file"
    }

    fn description(&self) -> &'static str {
        "View the contents of a file from the local filesystem. Always use absolute paths."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "AbsolutePath": {
                    "type": "string",
                    "description": "Path to file to view. Must be an absolute path."
                },
                "start_line": {
                    "type": "integer",
                    "description": "Optional. The 1-indexed line number to start reading from. Used for pagination."
                },
                "end_line": {
                    "type": "integer",
                    "description": "Optional. The 1-indexed line number to stop reading at. Used for pagination."
                }
            },
            "required": ["AbsolutePath"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: view_file]
- Role: Sensory Organ (文件视神经)
- Note: Do not worry about huge files consuming too many context tokens. The Engine's Context Interceptor will automatically compress massive files into crisp Skeleton Views using RTK when needed.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("AbsolutePath")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'AbsolutePath' argument")?;

        let path = Path::new(path_str);
        require_safe_path(path, "file read", &self.workspace_root)?;
        if !path.exists() {
            return Err(format!("File not found: {}", path_str));
        }

        let start_line = args
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let end_line = args
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        match fs::read_to_string(path) {
            Ok(full_content) => {
                let mut content = full_content.clone();
                let mut is_paginated = false;
                if start_line.is_some() || end_line.is_some() {
                    let lines: Vec<&str> = full_content.lines().collect();
                    let start_idx = start_line.unwrap_or(1).saturating_sub(1);
                    let end_idx = end_line.unwrap_or(lines.len()).min(lines.len());
                    if start_idx < lines.len() && start_idx < end_idx {
                        content = lines[start_idx..end_idx].join("\n");
                        is_paginated = true;
                    }
                }

                Ok(json!({
                    "content": content,
                    "paginated": is_paginated,
                    "bytes": content.len(),
                    "file_path": path_str
                }))
            }
            Err(e) => Err(format!("Failed to read file: {}", e)),
        }
    }
}

/// 文件写入工具 (Write To File)
pub struct WriteFileTool {
    pub workspace_root: PathBuf,
}

impl WriteFileTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_to_file"
    }

    fn description(&self) -> &'static str {
        "Create a new file or overwrite an existing file with the specified content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "TargetFile": {
                    "type": "string",
                    "description": "The target file to create and write code to. Must be an absolute path."
                },
                "CodeContent": {
                    "type": "string",
                    "description": "The code contents to write to the file."
                }
            },
            "required": ["TargetFile", "CodeContent"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let target_file = args
            .get("TargetFile")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'TargetFile' argument")?;

        let code_content = args
            .get("CodeContent")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'CodeContent' argument")?;

        let path = Path::new(target_file);
        if !is_path_safe(path, &self.workspace_root) {
            return Err("Security Error: Path traversal detected. Operations are restricted to the workspace.".into());
        }

        // 自动创建父目录
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Err(format!("Failed to create parent directories: {}", e));
                }
            }
        }
        // 自动快照：脊髓反射 (Spinal Reflex - Shadow Backup)
        eprintln!(
            "\x1b[36m[Spinal Reflex] 运动中枢触发防卫：自动备份 {}\x1b[0m",
            target_file
        );
        let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
        let cwd_str = cwd.to_string_lossy().to_string();
        let _ = crate::services::git_service::record_ai_snapshot(
            &cwd_str,
            &format!(
                "[Shadow Backup] Auto-backup before modifying {}",
                target_file
            ),
        );

        match fs::write(path, code_content) {
            Ok(_) => Ok(
                json!({ "status": "success", "message": format!("File written to {}", target_file) }),
            ),
            Err(e) => Err(format!("Failed to write to file: {}", e)),
        }
    }
}

pub struct ReplaceInFileTool {
    pub workspace_root: PathBuf,
}

impl ReplaceInFileTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for ReplaceInFileTool {
    fn name(&self) -> &'static str {
        "replace_in_file"
    }

    fn description(&self) -> &'static str {
        "Replace a specific block of text in a file with new text."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The absolute path to the file" },
                "search_text": { "type": "string", "description": "The exact text block to search for and replace. Must match EXACTLY including whitespaces and indentation." },
                "replace_text": { "type": "string", "description": "The new text block to insert" }
            },
            "required": ["path", "search_text", "replace_text"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("Missing path")?;
        let path = Path::new(path_str);
        if !is_path_safe(path, &self.workspace_root) {
            return Err("Security Error: Path traversal detected. Operations are restricted to the workspace.".into());
        }
        let search_text = args
            .get("search_text")
            .and_then(|v| v.as_str())
            .ok_or("Missing search_text")?;
        let replace_text = args
            .get("replace_text")
            .and_then(|v| v.as_str())
            .ok_or("Missing replace_text")?;

        let mut content =
            std::fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;
        if !content.contains(search_text) {
            return Err("Search text not found in the file. Please ensure exact match including whitespace.".into());
        }

        let cwd =
            std::env::current_dir().unwrap_or_else(|_| std::path::Path::new(".").to_path_buf());
        let _ = crate::services::git_service::record_ai_snapshot(
            &cwd.to_string_lossy().to_string(),
            &format!(
                "[Shadow Backup] Auto-backup before replace_in_file {}",
                path_str
            ),
        );
        content = content.replace(search_text, replace_text);

        let cwd =
            std::env::current_dir().unwrap_or_else(|_| std::path::Path::new(".").to_path_buf());
        let _ = crate::services::git_service::record_ai_snapshot(
            &cwd.to_string_lossy().to_string(),
            &format!(
                "[Shadow Backup] Auto-backup before multi_replace {}",
                path_str
            ),
        );
        std::fs::write(path, content).map_err(|e| format!("Failed to write file: {}", e))?;

        // (&format!("Auto snapshot before replace_in_file {}", path));

        Ok(
            json!({"success": true, "message": format!("Successfully replaced text in {}", path.display())}),
        )
    }
}

/// 多区块代码重构工具 (AST/Block-level Refactoring)
pub struct MultiReplaceTool {
    pub workspace_root: PathBuf,
}

impl MultiReplaceTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for MultiReplaceTool {
    fn name(&self) -> &'static str {
        "multi_replace_file_content"
    }

    fn description(&self) -> &'static str {
        "AST/Block-level automatic code refactoring. Edits multiple non-adjacent blocks of code in the same file simultaneously. Use this instead of replace_in_file when making edits across a file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The absolute path to the file" },
                "replacements": {
                    "type": "array",
                    "description": "Array of replacement objects",
                    "items": {
                        "type": "object",
                        "properties": {
                            "search_text": { "type": "string", "description": "The exact text block to search for." },
                            "replace_text": { "type": "string", "description": "The new text block." }
                        },
                        "required": ["search_text", "replace_text"]
                    }
                }
            },
            "required": ["path", "replacements"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("Missing path")?;
        let path = Path::new(path_str);
        if !is_path_safe(path, &self.workspace_root) {
            return Err("Security Error: Path traversal detected.".into());
        }

        let mut content =
            std::fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;

        let reps = args
            .get("replacements")
            .and_then(|v| v.as_array())
            .ok_or("Missing or invalid replacements array")?;

        let mut replaced_count = 0;
        let original_content = content.clone();

        for rep in reps {
            let search_text = rep
                .get("search_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let replace_text = rep
                .get("replace_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if search_text.is_empty() {
                continue;
            }

            if !content.contains(search_text) {
                // Rollback
                let _ = std::fs::write(path, original_content);
                return Err(format!("Search text block not found in file exactly as provided. Block starting with: {:.50}...", search_text));
            }

            let cwd =
                std::env::current_dir().unwrap_or_else(|_| std::path::Path::new(".").to_path_buf());
            let _ = crate::services::git_service::record_ai_snapshot(
                &cwd.to_string_lossy().to_string(),
                &format!(
                    "[Shadow Backup] Auto-backup before replace_in_file {}",
                    path_str
                ),
            );
            content = content.replace(search_text, replace_text);
            replaced_count += 1;
        }

        std::fs::write(path, &content).map_err(|e| format!("Failed to write file: {}", e))?;

        Ok(json!({
            "status": "success",
            "blocks_replaced": replaced_count,
            "message": format!("Successfully applied {} refactoring blocks.", replaced_count)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_path_traversal_prevention() {
        let tool = WriteFileTool {
            workspace_root: std::env::current_dir().unwrap(),
        };
        let malicious_path = if cfg!(windows) {
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        } else {
            "../../../../etc/passwd"
        };

        let args = json!({
            "TargetFile": malicious_path,
            "CodeContent": "malicious"
        });

        let result = tool.execute(args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Security Error"));
    }

    #[test]
    fn read_and_list_reject_paths_outside_workspace() {
        let outside = if cfg!(windows) {
            "C:\\Windows\\win.ini"
        } else {
            "/etc/hosts"
        };
        assert!(ReadFileTool {
            workspace_root: std::env::current_dir().unwrap()
        }
        .execute(json!({ "AbsolutePath": outside }))
        .is_err());
        assert!(ListDirectoryTool {
            workspace_root: std::env::current_dir().unwrap()
        }
        .execute(json!({ "path": outside }))
        .is_err());
    }
}

pub struct ListDirectoryTool {
    pub workspace_root: PathBuf,
}

impl ListDirectoryTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for ListDirectoryTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List all files and subdirectories within a directory. Use this to explore unknown codebases."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The absolute path to the directory" }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("Missing path")?;
        let path = std::path::Path::new(path_str);
        require_safe_path(path, "directory listing", &self.workspace_root)?;
        if !path.is_dir() {
            return Err(format!("{} is not a directory or does not exist", path_str));
        }
        let mut entries = vec![];
        match std::fs::read_dir(path) {
            Ok(read_dir) => {
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.path().is_dir();
                    let t = if is_dir { "DIR" } else { "FILE" };
                    entries.push(format!("[{}] {}", t, name));
                }
            }
            Err(e) => return Err(format!("Failed to read dir: {}", e)),
        }
        entries.sort();
        Ok(json!({"files": entries}))
    }
}
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
            .and_then(|v| v.as_str())
            .ok_or("Missing SearchDirectory")?;
        let pattern = args
            .get("Pattern")
            .and_then(|v| v.as_str())
            .ok_or("Missing Pattern")?;

        let path = Path::new(dir);
        if !path.is_dir() {
            return Err(format!("Directory not found: {}", dir));
        }

        let mut results = vec![];
        let walker = ignore::WalkBuilder::new(path).build();
        for result in walker {
            if let Ok(entry) = result {
                let file_name = entry.file_name().to_string_lossy();
                if file_name.contains(pattern) {
                    results.push(entry.path().to_string_lossy().into_owned());
                    if results.len() >= 100 {
                        break;
                    }
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
            .and_then(|v| v.as_str())
            .ok_or("Missing SearchPath")?;
        let query = args
            .get("Query")
            .and_then(|v| v.as_str())
            .ok_or("Missing Query")?;

        let path = Path::new(dir);
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
                            return true; // limit reached
                        }
                    }
                }
            }
            false
        };

        if path.is_file() {
            process_file(path);
        } else {
            let walker = ignore::WalkBuilder::new(path).build();
            for result in walker {
                if let Ok(entry) = result {
                    if entry.file_type().map_or(false, |ft| ft.is_file()) {
                        if process_file(entry.path()) {
                            break;
                        }
                    }
                }
            }
        }

        Ok(json!({ "matches": results }))
    }
}
