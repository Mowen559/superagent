//! 文件系统工具集 (Filesystem Tools)
//! 为大脑 B (Superagent) 赋予文件读写与修改的物理操作能力

use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

use std::path::PathBuf;

fn is_path_safe(path: &Path) -> bool {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
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
    
    let cwd_canonical = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    normalized.starts_with(cwd_canonical)
}


/// 文件读取工具 (View File)
pub struct ReadFileTool;

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
                "filter_level": {
                    "type": "string",
                    "description": "RTK Compression level. Options: 'none' (full file), 'minimal' (remove comments), 'aggressive' (skeleton view: imports, signatures, types only. Highly recommended for large or unknown files)."
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
- Note: RTK (Token Killer) is integrated. When exploring unfamiliar codebases or huge files, ALWAYS pass `filter_level: "aggressive"` first. This will instantly strip function bodies and return a crisp Skeleton View (signatures, types, constants), saving massive context tokens!
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("AbsolutePath")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'AbsolutePath' argument")?;

        let filter_level_str = args
            .get("filter_level")
            .and_then(|v| v.as_str())
            .unwrap_or("none"); // Default to none, unless forced by size

        let path = Path::new(path_str);
        if !path.exists() {
            return Err(format!("File not found: {}", path_str));
        }

        let start_line = args.get("start_line").and_then(|v| v.as_u64()).map(|v| v as usize);
        let end_line = args.get("end_line").and_then(|v| v.as_u64()).map(|v| v as usize);

        match fs::read_to_string(path) {
            Ok(full_content) => {
                use crate::services::token_filter::{compress, FilterLevel};
                use std::str::FromStr;
                
                let requested_level = FilterLevel::from_str(filter_level_str).unwrap_or(FilterLevel::None);
                
                // Handle Pagination if start_line or end_line is specified
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

                if requested_level != FilterLevel::None {
                    let result = compress(&content, path_str, requested_level);
                    Ok(json!({ 
                        "content": result.content, 
                        "rtk_compression": format!("{:?}", requested_level), 
                        "reduction_percent": result.reduction_percent,
                        "paginated": is_paginated
                    }))
                } else {
                    Ok(json!({ 
                        "content": content,
                        "paginated": is_paginated,
                        "bytes": content.len()
                    }))
                }
            },
            Err(e) => Err(format!("Failed to read file: {}", e)),
        }
    }
}

/// 文件写入工具 (Write To File)
pub struct WriteFileTool;

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
        if !is_path_safe(path) {
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
        eprintln!("\x1b[36m[Spinal Reflex] 运动中枢触发防卫：自动备份 {}\x1b[0m", target_file);
        let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
        let cwd_str = cwd.to_string_lossy().to_string();
        let _ = crate::services::git_service::record_ai_snapshot(
            &cwd_str,
            &format!("[Shadow Backup] Auto-backup before modifying {}", target_file)
        );

        match fs::write(path, code_content) {
            Ok(_) => Ok(json!({ "status": "success", "message": format!("File written to {}", target_file) })),
            Err(e) => Err(format!("Failed to write to file: {}", e)),
        }
    }
}


pub struct ReplaceInFileTool;

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
        let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing path")?;
        let path = Path::new(path_str);
        if !is_path_safe(path) {
            return Err("Security Error: Path traversal detected. Operations are restricted to the workspace.".into());
        }
        let search_text = args.get("search_text").and_then(|v| v.as_str()).ok_or("Missing search_text")?;
        let replace_text = args.get("replace_text").and_then(|v| v.as_str()).ok_or("Missing replace_text")?;

        let mut content = std::fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;
        if !content.contains(search_text) {
            return Err("Search text not found in the file. Please ensure exact match including whitespace.".into());
        }
        
        content = content.replace(search_text, replace_text);
        std::fs::write(path, content).map_err(|e| format!("Failed to write file: {}", e))?;
        
        // (&format!("Auto snapshot before replace_in_file {}", path));

        Ok(json!({"success": true, "message": format!("Successfully replaced text in {}", path.display())}))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_path_traversal_prevention() {
        let tool = WriteFileTool;
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
}


pub struct ListDirectoryTool;

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
        let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing path")?;
        let path = std::path::Path::new(path_str);
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
