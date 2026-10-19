//! 只读文件浏览、目录列举与源码符号大纲提取工具

use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::infrastructure::fs_path::resolve_safe_path;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

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
                },
                "filter_level": {
                    "type": "string",
                    "enum": ["none", "minimal", "aggressive"],
                    "description": "Optional RTK compression filter level. 'minimal' removes comments & collapses blank lines. 'aggressive' preserves only type/function signatures and constants (API Skeleton Outline)."
                }
            },
            "required": ["AbsolutePath"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(
            r#"
[Tool Projection: view_file]
- Role: Sensory Organ (文件视神经)
- Usage: For large unfamiliar code files, set `filter_level: "aggressive"` to inspect type/function API skeletons with up to 90% token reduction.
"#
            .trim()
            .to_string(),
        )
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("AbsolutePath")
            .or_else(|| args.get("path"))
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'AbsolutePath' argument")?;

        let resolved_path = resolve_safe_path(path_str, &self.workspace_root)?;
        if !resolved_path.exists() {
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

        let filter_level_str = args
            .get("filter_level")
            .and_then(|v| v.as_str())
            .unwrap_or("none");
        let filter_level = std::str::FromStr::from_str(filter_level_str)
            .unwrap_or(crate::domains::token_compression::FilterLevel::None);

        match fs::read_to_string(&resolved_path) {
            Ok(full_content) => {
                let all_lines: Vec<&str> = full_content.lines().collect();
                let total_lines = all_lines.len();

                let (start_idx, end_idx, is_truncated) = match (start_line, end_line) {
                    (Some(s), Some(e)) => {
                        let start = s.saturating_sub(1);
                        let end = e.min(total_lines);
                        (start, end, false)
                    }
                    (Some(s), None) => {
                        let start = s.saturating_sub(1);
                        let end = (start + 400).min(total_lines);
                        (start, end, end < total_lines)
                    }
                    (None, Some(e)) => {
                        let end = e.min(total_lines);
                        (0, end, false)
                    }
                    (None, None) => {
                        if total_lines > 400 {
                            (0, 400, true)
                        } else {
                            (0, total_lines, false)
                        }
                    }
                };

                let slice_lines = if start_idx < total_lines && start_idx < end_idx {
                    &all_lines[start_idx..end_idx]
                } else {
                    &all_lines[..0]
                };

                let mut numbered_content = String::new();
                for (offset, line) in slice_lines.iter().enumerate() {
                    let line_no = start_idx + offset + 1;
                    numbered_content.push_str(&format!("{}: {}\n", line_no, line));
                }

                if is_truncated {
                    numbered_content.push_str(&format!(
                        "\n[NOTE: File has {} total lines. Showing lines {}-{} (truncated for token efficiency). Specify start_line: {}, end_line: {} to view the next chunk.]",
                        total_lines,
                        start_idx + 1,
                        end_idx,
                        end_idx + 1,
                        (end_idx + 400).min(total_lines)
                    ));
                }

                let mut content = numbered_content;
                let mut rtk_metrics = json!({});
                if filter_level != crate::domains::token_compression::FilterLevel::None {
                    let comp =
                        crate::domains::token_compression::compress(&full_content, path_str, filter_level);
                    crate::domains::token_compression::TokenSavingsTracker::global()
                        .record_savings(&comp);
                    content = comp.content;
                    rtk_metrics = json!({
                        "level": comp.level,
                        "reduction_percent": comp.reduction_percent,
                        "tokens_saved_est": comp.original_tokens_est.saturating_sub(comp.compressed_tokens_est),
                    });
                }

                Ok(json!({
                    "content": content,
                    "total_lines": total_lines,
                    "start_line": start_idx + 1,
                    "end_line": end_idx,
                    "truncated": is_truncated,
                    "bytes": content.len(),
                    "file_path": resolved_path.to_string_lossy().to_string(),
                    "rtk_metrics": rtk_metrics
                }))
            }
            Err(e) => Err(format!("Failed to read file: {}", e)),
        }
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
                "path": { "type": "string", "description": "The path to the directory (optional, default: workspace root)" },
                "DirectoryPath": { "type": "string", "description": "Alias for path" }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("path")
            .or_else(|| args.get("DirectoryPath"))
            .or_else(|| args.get("dir"))
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let path = resolve_safe_path(path_str, &self.workspace_root)?;
        if !path.is_dir() {
            return Err(format!("{} is not a directory or does not exist", path_str));
        }
        let mut entries = vec![];
        match std::fs::read_dir(&path) {
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

/// 语义符号骨架大纲工具 (View Outline)
pub struct ViewOutlineTool {
    pub workspace_root: PathBuf,
}

impl ViewOutlineTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

pub fn extract_symbol_outline(content: &str, ext: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut outline = Vec::new();
    let ext_lower = ext.to_lowercase();

    match ext_lower.as_str() {
        "java" => {
            let mut pending_annotations = Vec::new();
            for (idx, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                let line_num = idx + 1;

                if trimmed.is_empty()
                    || trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                {
                    continue;
                }

                if trimmed.starts_with("package ") || trimmed.starts_with("import ") {
                    if trimmed.starts_with("package ") {
                        outline.push(format!("L{:4}: {}", line_num, trimmed));
                    }
                    continue;
                }

                if trimmed.starts_with('@') {
                    pending_annotations.push(format!("L{:4}:   {}", line_num, trimmed));
                    continue;
                }

                let is_type_decl = trimmed.contains("class ")
                    || trimmed.contains("interface ")
                    || trimmed.contains("enum ")
                    || trimmed.contains("record ");

                let is_method_decl = (trimmed.starts_with("public ")
                    || trimmed.starts_with("protected ")
                    || trimmed.starts_with("private ")
                    || trimmed.starts_with("static "))
                    && trimmed.contains('(')
                    && (trimmed.ends_with('{') || trimmed.ends_with(';') || trimmed.contains(')'));

                if is_type_decl || is_method_decl {
                    for ann in pending_annotations.drain(..) {
                        outline.push(ann);
                    }
                    let clean_sig = trimmed.trim_end_matches('{').trim();
                    let indent = if is_type_decl { "" } else { "  " };
                    outline.push(format!("L{:4}: {}{}", line_num, indent, clean_sig));
                } else {
                    pending_annotations.clear();
                }
            }
        }
        "rs" => {
            let mut pending_attrs = Vec::new();
            for (idx, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                let line_num = idx + 1;

                if trimmed.is_empty()
                    || trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                {
                    continue;
                }

                if trimmed.starts_with("#[") {
                    pending_attrs.push(format!("L{:4}:   {}", line_num, trimmed));
                    continue;
                }

                let is_mod = trimmed.starts_with("pub mod ") || trimmed.starts_with("mod ");
                let is_type_decl = trimmed.starts_with("pub struct ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("pub enum ")
                    || trimmed.starts_with("enum ")
                    || trimmed.starts_with("pub trait ")
                    || trimmed.starts_with("trait ")
                    || trimmed.starts_with("impl ")
                    || trimmed.starts_with("pub type ")
                    || trimmed.starts_with("type ");

                let is_fn_decl = (trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("pub async fn ")
                    || trimmed.starts_with("fn ")
                    || trimmed.starts_with("async fn "))
                    && trimmed.contains('(');

                if is_mod || is_type_decl || is_fn_decl {
                    for attr in pending_attrs.drain(..) {
                        outline.push(attr);
                    }
                    let clean_sig = trimmed.trim_end_matches('{').trim();
                    let indent = if is_fn_decl && !trimmed.starts_with("pub ") {
                        "  "
                    } else {
                        ""
                    };
                    outline.push(format!("L{:4}: {}{}", line_num, indent, clean_sig));
                } else {
                    pending_attrs.clear();
                }
            }
        }
        "ts" | "tsx" | "js" | "jsx" => {
            for (idx, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                let line_num = idx + 1;

                if trimmed.is_empty()
                    || trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                {
                    continue;
                }

                let is_export = trimmed.starts_with("export default ")
                    || trimmed.starts_with("export function ")
                    || trimmed.starts_with("export const ")
                    || trimmed.starts_with("export interface ")
                    || trimmed.starts_with("export type ")
                    || trimmed.starts_with("export class ")
                    || trimmed.starts_with("export enum ");

                let is_decl = trimmed.starts_with("function ")
                    || trimmed.starts_with("interface ")
                    || trimmed.starts_with("type ")
                    || trimmed.starts_with("class ")
                    || (trimmed.starts_with("const ")
                        && (trimmed.contains("= (")
                            || trimmed.contains("= async (")
                            || trimmed.contains(": React.")));

                if is_export || is_decl {
                    let clean_sig = trimmed.trim_end_matches('{').trim();
                    outline.push(format!("L{:4}: {}", line_num, clean_sig));
                }
            }
        }
        "py" => {
            let mut pending_decorators = Vec::new();
            for (idx, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                let line_num = idx + 1;

                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }

                if trimmed.starts_with('@') {
                    pending_decorators.push(format!("L{:4}: {}", line_num, trimmed));
                    continue;
                }

                let is_def = trimmed.starts_with("def ") || trimmed.starts_with("async def ");
                let is_class = trimmed.starts_with("class ");

                if is_def || is_class {
                    for dec in pending_decorators.drain(..) {
                        outline.push(dec);
                    }
                    outline.push(format!("L{:4}: {}", line_num, trimmed));
                } else {
                    pending_decorators.clear();
                }
            }
        }
        _ => {
            for (idx, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                let line_num = idx + 1;
                if trimmed.starts_with("func ")
                    || trimmed.starts_with("def ")
                    || trimmed.starts_with("fn ")
                    || trimmed.starts_with("class ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("interface ")
                {
                    outline.push(format!("L{:4}: {}", line_num, trimmed));
                }
            }
        }
    }

    outline
}

impl ToolExecutor for ViewOutlineTool {
    fn name(&self) -> &'static str {
        "view_outline"
    }

    fn description(&self) -> &'static str {
        "Extract high-level symbol outline (classes, interfaces, methods, routes, annotations, functions) with 1-based line numbers from a source code file. Supports Java, Rust, TypeScript, JavaScript, Python, Go. Extremely token-efficient for understanding file structure before viewing or modifying sections."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to file to outline. Must be an absolute path or relative to workspace root."
                }
            },
            "required": ["path"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(
            r#"
[Tool Projection: view_outline]
- Role: Semantic Structural Radar (代码语义骨架雷达)
- Usage: For large unfamiliar code files (> 100 lines), always invoke `view_outline` first to inspect class structures, method signatures, Spring routes, or exported APIs. This saves 90%+ tokens compared to reading the entire file, and provides exact line numbers for surgical edits.
"#
            .trim()
            .to_string(),
        )
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("path")
            .or_else(|| args.get("AbsolutePath"))
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .ok_or("Missing 'path' argument")?;
        let resolved = resolve_safe_path(path_str, &self.workspace_root)?;

        if !resolved.exists() {
            return Err(format!("File not found: {}", path_str));
        }

        let content = std::fs::read_to_string(&resolved)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        let ext = resolved
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let outline_lines = extract_symbol_outline(&content, ext);
        let outline_text = outline_lines.join("\n");
        let total_lines = content.lines().count();
        let orig_bytes = content.len();
        let outline_bytes = outline_text.len();
        let reduction_percent = if orig_bytes > 0 && outline_bytes < orig_bytes {
            ((orig_bytes - outline_bytes) as f64 / orig_bytes as f64) * 100.0
        } else {
            0.0
        };

        Ok(json!({
            "path": resolved.display().to_string(),
            "language": ext,
            "total_lines": total_lines,
            "outline_symbol_count": outline_lines.len(),
            "outline": outline_text,
            "reduction_percent": (reduction_percent * 10.0).round() / 10.0
        }))
    }
}
