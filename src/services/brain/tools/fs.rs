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
        Some(r#"
[Tool Projection: view_file]
- Role: Sensory Organ (文件视神经)
- Usage: For large unfamiliar code files, set `filter_level: "aggressive"` to inspect type/function API skeletons with up to 90% token reduction.
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

        let filter_level_str = args
            .get("filter_level")
            .and_then(|v| v.as_str())
            .unwrap_or("none");
        let filter_level = std::str::FromStr::from_str(filter_level_str)
            .unwrap_or(crate::services::token_filter::FilterLevel::None);

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

                let mut rtk_metrics = json!({});
                if filter_level != crate::services::token_filter::FilterLevel::None {
                    let comp = crate::services::token_filter::compress(&content, path_str, filter_level);
                    crate::services::token_filter::TokenSavingsTracker::global().record_savings(&comp);
                    content = comp.content;
                    rtk_metrics = json!({
                        "level": comp.level,
                        "reduction_percent": comp.reduction_percent,
                        "tokens_saved_est": comp.original_tokens_est.saturating_sub(comp.compressed_tokens_est),
                    });
                }

                Ok(json!({
                    "content": content,
                    "paginated": is_paginated,
                    "bytes": content.len(),
                    "file_path": path_str,
                    "rtk_metrics": rtk_metrics
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
            Ok(_) => {
                let diagnostics = collect_file_diagnostics(path, code_content);
                let arch_advisories = crate::services::brain::architectural_linter::ArchitecturalLinter::lint(path, code_content);
                let mut message = format!("File written to {}", target_file);
                if let Some(adv_text) = crate::services::brain::architectural_linter::ArchitecturalLinter::format_advisories(&arch_advisories) {
                    message.push_str(&adv_text);
                }
                Ok(
                    json!({ "status": "success", "message": message, "diagnostics": diagnostics }),
                )
            }
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
        let (actual_search, repair_msg) = if let Some(alignment) =
            crate::services::diff_relocation::AnchorAligner::align(&content, search_text)
        {
            if alignment.repaired {
                (
                    alignment.matched_text,
                    format!(
                        " (Anchor auto-repaired via {}, confidence: {:.2})",
                        alignment.repair_strategy, alignment.confidence
                    ),
                )
            } else {
                (search_text.to_string(), String::new())
            }
        } else {
            return Err("Search text not found in the file even after fuzzy anchor repair. Please check surrounding code.".into());
        };

        let cwd =
            std::env::current_dir().unwrap_or_else(|_| std::path::Path::new(".").to_path_buf());
        let _ = crate::services::git_service::record_ai_snapshot(
            &cwd.to_string_lossy().to_string(),
            &format!(
                "[Shadow Backup] Auto-backup before replace_in_file {}",
                path_str
            ),
        );
        content = content.replace(&actual_search, replace_text);
        std::fs::write(path, &content).map_err(|e| format!("Failed to write file: {}", e))?;
        let diagnostics = collect_file_diagnostics(path, &content);

        // 改前/改中符号波及面推演 (Call-Graph Impact Radar)
        let impact_warning = if let Some(sym) = crate::services::diff_impact::extract_declared_symbol(&actual_search) {
            let app_paths = crate::config::app_paths::AppPaths::new();
            let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
            let cwd_str = self.workspace_root.to_string_lossy().to_string();
            let db_path = config.codebase_memory_db_path(Some(&cwd_str));
            let db_path_str = db_path.to_string_lossy().to_string();
            if let Ok(radar) = crate::services::diff_impact::DiffImpactAnalyzer::analyze_symbol_impact(
                &self.workspace_root,
                &db_path_str,
                &sym,
                Some(path),
            ) {
                if radar.total_callers > 0 {
                    eprintln!(
                        "\x1b[33m[Impact Radar] Warning: Modified symbol '{}' is referenced by {} external location(s): {:?}\x1b[0m",
                        sym, radar.total_callers, radar.affected_files
                    );
                    Some(serde_json::to_value(radar).unwrap_or(Value::Null))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let arch_advisories = crate::services::brain::architectural_linter::ArchitecturalLinter::lint(path, &content);
        let mut msg = format!("Successfully replaced text in {}{}", path.display(), repair_msg);
        if let Some(adv_text) = crate::services::brain::architectural_linter::ArchitecturalLinter::format_advisories(&arch_advisories) {
            msg.push_str(&adv_text);
        }
        let mut res = json!({"success": true, "message": msg, "diagnostics": diagnostics});
        if let Some(impact) = impact_warning {
            res["impact_radar"] = impact;
        }
        Ok(res)
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

            let actual_search = if content.contains(search_text) {
                search_text.to_string()
            } else if let Some(alignment) = crate::services::diff_relocation::AnchorAligner::align(&content, search_text) {
                alignment.matched_text
            } else {
                // Rollback
                let _ = std::fs::write(path, original_content);
                return Err(format!("Search text block not found in file even after anchor self-repair. Block starting with: {:.50}...", search_text));
            };

            let cwd =
                std::env::current_dir().unwrap_or_else(|_| std::path::Path::new(".").to_path_buf());
            let _ = crate::services::git_service::record_ai_snapshot(
                &cwd.to_string_lossy().to_string(),
                &format!(
                    "[Shadow Backup] Auto-backup before replace_in_file {}",
                    path_str
                ),
            );
            content = content.replace(&actual_search, replace_text);
            replaced_count += 1;
        }

        std::fs::write(path, &content).map_err(|e| format!("Failed to write file: {}", e))?;
        let diagnostics = collect_file_diagnostics(path, &content);
        let arch_advisories = crate::services::brain::architectural_linter::ArchitecturalLinter::lint(path, &content);
        let mut msg = format!("Successfully applied {} refactoring blocks.", replaced_count);
        if let Some(adv_text) = crate::services::brain::architectural_linter::ArchitecturalLinter::format_advisories(&arch_advisories) {
            msg.push_str(&adv_text);
        }

        let mut res = json!({
            "status": "success",
            "blocks_replaced": replaced_count,
            "message": msg,
            "diagnostics": diagnostics
        });

        if let Some(sym) = crate::services::diff_impact::extract_declared_symbol(&content) {
            let app_paths = crate::config::app_paths::AppPaths::new();
            let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
            let cwd_str = self.workspace_root.to_string_lossy().to_string();
            let db_path = config.codebase_memory_db_path(Some(&cwd_str));
            let db_path_str = db_path.to_string_lossy().to_string();
            if let Ok(radar) = crate::services::diff_impact::DiffImpactAnalyzer::analyze_symbol_impact(
                &self.workspace_root,
                &db_path_str,
                &sym,
                Some(path),
            ) {
                if radar.total_callers > 0 {
                    res["impact_radar"] = serde_json::to_value(radar).unwrap_or(Value::Null);
                }
            }
        }

        Ok(res)
    }
}

/// 收集已写入文件的即时语法与行级编译器诊断 (Compiler/Syntax in the Loop)
pub fn collect_file_diagnostics(path: &Path, content: &str) -> Vec<Value> {
    let mut diagnostics = Vec::new();

    // 1. LSP 全局活跃客户端诊断 (若运行着 rust-analyzer/tsserver/jdtls)
    let path_str = path.to_string_lossy().to_string();
    let lsp_diags = crate::services::brain::tools::lsp_client::get_global_diagnostics_for_file(&path_str);
    diagnostics.extend(lsp_diags);

    // 2. 内置快速轻量语法结构校验 (Fast Structural Syntax Checker)
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "json" => {
            if let Err(e) = serde_json::from_str::<Value>(content) {
                diagnostics.push(json!({
                    "severity": "error",
                    "line": e.line(),
                    "column": e.column(),
                    "source": "SyntaxValidator",
                    "message": format!("JSON 语法错误: {}", e)
                }));
            }
        }
        "toml" => {
            if let Err(e) = toml::from_str::<toml::Value>(content) {
                diagnostics.push(json!({
                    "severity": "error",
                    "line": e.span().map(|s| s.start).unwrap_or(1),
                    "source": "SyntaxValidator",
                    "message": format!("TOML 语法错误: {}", e.message())
                }));
            }
        }
        "rs" | "ts" | "tsx" | "js" | "jsx" | "java" => {
            // 基础括号与分隔符匹配平衡度检测 (Bracket Balance Check)
            let mut stack = Vec::new();
            let mut line = 1usize;
            let mut col = 0usize;
            let mut in_line_comment = false;
            let mut in_block_comment = false;
            let mut in_string = false;
            let mut string_char = ' ';
            let mut chars = content.chars().peekable();

            while let Some(ch) = chars.next() {
                col += 1;
                if ch == '\n' {
                    line += 1;
                    col = 0;
                    in_line_comment = false;
                    continue;
                }

                if in_line_comment {
                    continue;
                }

                if in_block_comment {
                    if ch == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        col += 1;
                        in_block_comment = false;
                    }
                    continue;
                }

                if in_string {
                    if ch == '\\' {
                        let _ = chars.next();
                        col += 1;
                    } else if ch == string_char {
                        in_string = false;
                    }
                    continue;
                }

                if ch == '/' && chars.peek() == Some(&'/') {
                    chars.next();
                    col += 1;
                    in_line_comment = true;
                    continue;
                }

                if ch == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    col += 1;
                    in_block_comment = true;
                    continue;
                }

                if ch == '"' || ch == '\'' || ch == '`' {
                    in_string = true;
                    string_char = ch;
                    continue;
                }

                match ch {
                    '{' | '(' | '[' => stack.push((ch, line, col)),
                    '}' => {
                        if let Some((open, _, _)) = stack.pop() {
                            if open != '{' {
                                diagnostics.push(json!({
                                    "severity": "error",
                                    "line": line,
                                    "column": col,
                                    "source": "SyntaxValidator",
                                    "message": format!("括号不匹配: 遇到 '}}'，但此前未闭合的左括号为 '{}'", open)
                                }));
                                break;
                            }
                        } else {
                            diagnostics.push(json!({
                                "severity": "error",
                                "line": line,
                                "column": col,
                                "source": "SyntaxValidator",
                                "message": "括号不匹配: 出现多余未配对的右花括号 '}'"
                            }));
                            break;
                        }
                    }
                    ')' => {
                        if let Some((open, _, _)) = stack.pop() {
                            if open != '(' {
                                diagnostics.push(json!({
                                    "severity": "error",
                                    "line": line,
                                    "column": col,
                                    "source": "SyntaxValidator",
                                    "message": format!("括号不匹配: 遇到 ')'，但此前未闭合的左括号为 '{}'", open)
                                }));
                                break;
                            }
                        } else {
                            diagnostics.push(json!({
                                "severity": "error",
                                "line": line,
                                "column": col,
                                "source": "SyntaxValidator",
                                "message": "括号不匹配: 出现多余未配对的右圆括号 ')'"
                            }));
                            break;
                        }
                    }
                    ']' => {
                        if let Some((open, _, _)) = stack.pop() {
                            if open != '[' {
                                diagnostics.push(json!({
                                    "severity": "error",
                                    "line": line,
                                    "column": col,
                                    "source": "SyntaxValidator",
                                    "message": format!("括号不匹配: 遇到 ']'，但此前未闭合的左括号为 '{}'", open)
                                }));
                                break;
                            }
                        } else {
                            diagnostics.push(json!({
                                "severity": "error",
                                "line": line,
                                "column": col,
                                "source": "SyntaxValidator",
                                "message": "括号不匹配: 出现多余未配对的右方括号 ']'"
                            }));
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if diagnostics.is_empty() && !stack.is_empty() {
                if let Some((open, o_line, o_col)) = stack.last() {
                    diagnostics.push(json!({
                        "severity": "error",
                        "line": o_line,
                        "column": o_col,
                        "source": "SyntaxValidator",
                        "message": format!("结构未闭合: 位于第 {} 行的 '{}' 未找到对应的闭合右括号", o_line, open)
                    }));
                }
            }
        }
        _ => {}
    }

    // 3. DeepCloud 核心架构契约实时 Linter (Architectural Linter)
    let arch_advisories = crate::services::brain::architectural_linter::ArchitecturalLinter::lint(path, content);
    for adv in arch_advisories {
        diagnostics.push(json!({
            "severity": "warning",
            "line": 1,
            "column": 1,
            "source": "ArchitectureLinter",
            "rule": adv.rule_name,
            "message": format!("[{}] {}. Fix: {}", adv.rule_name, adv.violation, adv.prescribed_fix)
        }));
    }

    diagnostics
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

    #[test]
    fn test_collect_file_diagnostics() {
        // 1. JSON 校验
        let valid_json = Path::new("test.json");
        assert!(collect_file_diagnostics(valid_json, r#"{"valid": true}"#).is_empty());
        let invalid_json = collect_file_diagnostics(valid_json, r#"{"invalid": }"#);
        assert!(!invalid_json.is_empty());
        assert_eq!(invalid_json[0]["severity"], "error");

        // 2. 括号匹配校验
        let valid_rs = Path::new("main.rs");
        assert!(collect_file_diagnostics(valid_rs, "fn main() { println!(\"hello\"); }").is_empty());

        let invalid_rs = collect_file_diagnostics(valid_rs, "fn main() { let x = (1 + 2; }");
        assert!(!invalid_rs.is_empty());
        assert_eq!(invalid_rs[0]["severity"], "error");

        // 3. 未闭合花括号
        let unclosed_rs = collect_file_diagnostics(valid_rs, "fn main() { if true { println!(\"yes\"); }");
        assert!(!unclosed_rs.is_empty());
        assert_eq!(unclosed_rs[0]["severity"], "error");
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

/// 语义符号骨架大纲工具 (View Outline)
/// 为智能体提供轻量级源码符号与路由轮廓提取，免去阅读数百行函数体造成的上下文冲刷
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

                if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    continue;
                }

                if trimmed.starts_with("package ") || trimmed.starts_with("import ") {
                    if trimmed.starts_with("package ") {
                        outline.push(format!("L{:4}: {}", line_num, trimmed));
                    }
                    continue;
                }

                // Annotations like @RestController, @PostMapping, @Override, etc.
                if trimmed.starts_with('@') {
                    pending_annotations.push(format!("L{:4}:   {}", line_num, trimmed));
                    continue;
                }

                // Class, Interface, Record, Enum declarations
                let is_type_decl = trimmed.contains("class ")
                    || trimmed.contains("interface ")
                    || trimmed.contains("enum ")
                    || trimmed.contains("record ");

                // Method declarations (public/protected/private ... (...))
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

                if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
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
                    let indent = if is_fn_decl && !trimmed.starts_with("pub ") { "  " } else { "" };
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

                if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
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
                    || (trimmed.starts_with("const ") && (trimmed.contains("= (") || trimmed.contains("= async (") || trimmed.contains(": React.")));

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
        Some(r#"
[Tool Projection: view_outline]
- Role: Semantic Structural Radar (代码语义骨架雷达)
- Usage: For large unfamiliar code files (> 100 lines), always invoke `view_outline` first to inspect class structures, method signatures, Spring routes, or exported APIs. This saves 90%+ tokens compared to reading the entire file, and provides exact line numbers for surgical edits.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'path' argument")?;
        let path = Path::new(path_str);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };

        if !is_path_safe(&resolved, &self.workspace_root) {
            return Err("Security Error: Path traversal detected. Operations are restricted to the workspace.".into());
        }

        if !resolved.exists() {
            return Err(format!("File not found: {}", resolved.display()));
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

#[cfg(test)]
mod outline_tests {
    use super::*;

    #[test]
    fn test_extract_symbol_outline_java() {
        let java_code = r#"
package com.deepcloud.engine.api;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/api/v1/jobs")
public class SpiderJobController extends BaseController {

    @PostMapping
    @RequirePermission("engine:job:create")
    public Result<Long> createJob(@Valid @RequestBody CreateJobCmd cmd) {
        // Implementation with 50 lines
        return Result.ok(1L);
    }

    @GetMapping("/{id}")
    public Result<SpiderJobVO> getJob(@PathVariable("id") Long id) {
        // Implementation
        return Result.ok(null);
    }
}
"#;
        let outline = extract_symbol_outline(java_code, "java");
        let joined = outline.join("\n");
        assert!(joined.contains("package com.deepcloud.engine.api;"));
        assert!(joined.contains("@RestController"));
        assert!(joined.contains("@RequestMapping(\"/api/v1/jobs\")"));
        assert!(joined.contains("public class SpiderJobController extends BaseController"));
        assert!(joined.contains("@PostMapping"));
        assert!(joined.contains("public Result<Long> createJob(@Valid @RequestBody CreateJobCmd cmd)"));
        assert!(joined.contains("@GetMapping(\"/{id}\")"));
        assert!(!joined.contains("Implementation with 50 lines"));
    }

    #[test]
    fn test_extract_symbol_outline_rust() {
        let rs_code = r#"
pub mod engine;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SpiderJob {
    pub id: String,
}

impl SpiderJob {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    fn internal_helper(&self) {
        // secret logic
    }
}
"#;
        let outline = extract_symbol_outline(rs_code, "rs");
        let joined = outline.join("\n");
        assert!(joined.contains("pub mod engine;"));
        assert!(joined.contains("#[derive(Debug, Clone)]"));
        assert!(joined.contains("pub struct SpiderJob"));
        assert!(joined.contains("impl SpiderJob"));
        assert!(joined.contains("pub fn new(id: String) -> Self"));
        assert!(joined.contains("fn internal_helper(&self)"));
        assert!(!joined.contains("secret logic"));
    }
}
