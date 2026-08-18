//! 文件系统工具集 (Filesystem Tools)
//! 为大脑 B (Superagent) 赋予文件读写与修改的物理操作能力

use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

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
                }
            },
            "required": ["AbsolutePath"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("AbsolutePath")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'AbsolutePath' argument")?;

        let path = Path::new(path_str);
        if !path.exists() {
            return Err(format!("File not found: {}", path_str));
        }

        match fs::read_to_string(path) {
            Ok(content) => {
                // 如果文件太大，挂载 RTK Token Filter 进行降维压缩
                if content.len() > 10000 {
                    eprintln!("[Brain B] File too large ({} bytes). Engaging RTK Token Killer (Aggressive)...", content.len());
                    let result = crate::services::token_filter::compress(&content, path_str, crate::services::token_filter::FilterLevel::Aggressive);
                    
                    Ok(json!({ 
                        "content": result.content, 
                        "rtk_compression": true, 
                        "reduction_percent": result.reduction_percent 
                    }))
                } else {
                    Ok(json!({ "content": content }))
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
        
        // 自动创建父目录
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Err(format!("Failed to create parent directories: {}", e));
                }
            }
        }

        match fs::write(path, code_content) {
            Ok(_) => Ok(json!({ "status": "success", "message": format!("File written to {}", target_file) })),
            Err(e) => Err(format!("Failed to write to file: {}", e)),
        }
    }
}
