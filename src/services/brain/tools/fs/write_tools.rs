//! 文件创建、覆写与单/多区块精准替换工具

use super::diagnostics::{
    analyze_symbol_impact_radar, collect_file_diagnostics, format_arch_advisories,
    record_shadow_backup,
};
use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::infrastructure::fs_path::resolve_safe_path;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

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
            .or_else(|| args.get("path"))
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'TargetFile' argument")?;

        let code_content = args
            .get("CodeContent")
            .or_else(|| args.get("content"))
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'CodeContent' argument")?;

        let path = resolve_safe_path(target_file, &self.workspace_root)?;

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directories: {}", e))?;
            }
        }

        eprintln!(
            "\x1b[36m[Spinal Reflex] 运动中枢触发防卫：自动备份 {}\x1b[0m",
            path.display()
        );
        record_shadow_backup(&self.workspace_root, &path, "modifying");

        match fs::write(&path, code_content) {
            Ok(_) => {
                let diagnostics = collect_file_diagnostics(&path, code_content);
                let mut message = format!("File written to {}", path.display());
                if let Some(adv_text) = format_arch_advisories(&path, code_content) {
                    message.push_str(&adv_text);
                }

                let has_compiler_errors = diagnostics
                    .iter()
                    .any(|d| d.get("severity").and_then(|s| s.as_str()) == Some("error"));
                if has_compiler_errors {
                    let error_summaries: Vec<String> = diagnostics
                        .iter()
                        .filter(|d| d.get("severity").and_then(|s| s.as_str()) == Some("error"))
                        .map(|d| {
                            let line = d.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
                            let m = d.get("message").and_then(|m| m.as_str()).unwrap_or("syntax error");
                            format!("- Line {}: {}", line, m)
                        })
                        .collect();
                    message.push_str(&format!(
                        "\n\n⚠️ [CRITICAL COMPILER/SYNTAX ERROR DETECTED]: The file contains syntax/compilation errors after writing!\n{}\nAction required: You must IMMEDIATELY fix these errors before declaring the task finished!",
                        error_summaries.join("\n")
                    ));
                }

                Ok(json!({
                    "status": "success",
                    "message": message,
                    "diagnostics": diagnostics
                }))
            }
            Err(e) => Err(format!("Failed to write to file: {}", e)),
        }
    }
}

/// 单区块内容替换工具 (Replace In File)
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
            .or_else(|| args.get("TargetFile"))
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .ok_or("Missing path")?;
        let path = resolve_safe_path(path_str, &self.workspace_root)?;
        if !path.exists() {
            return Err(format!("File not found: {}", path_str));
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
            std::fs::read_to_string(&path).map_err(|e| format!("Failed to read file: {}", e))?;
        let (actual_search, repair_msg) = if let Some(alignment) =
            crate::domains::version_control::diff_relocation::AnchorAligner::align(
                &content,
                search_text,
            ) {
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
            let hint_msg = if let Some(candidate) =
                crate::domains::version_control::diff_relocation::AnchorAligner::find_closest_candidate(
                    &content,
                    search_text,
                ) {
                format!(
                    "Search text not found in '{}'.\n\
                    [SWE-agent Actionable Hint]: A near-matching code block was located at lines {}-{}:\n\
                    --------------------------------------------------\n\
                    {}\n\
                    --------------------------------------------------\n\
                    Guidance: Please inspect the actual lines above. Use `view_file` to verify line numbers or adjust your `search_text` to match the exact code shown above.",
                    path.display(),
                    candidate.start_line,
                    candidate.end_line,
                    candidate.snippet
                )
            } else {
                format!(
                    "Search text not found in '{}'. Please call `view_file` to read the current file content before attempting replacement.",
                    path.display()
                )
            };
            return Err(hint_msg);
        };

        record_shadow_backup(&self.workspace_root, &path, "replace_in_file");
        content = content.replace(&actual_search, replace_text);
        std::fs::write(&path, &content).map_err(|e| format!("Failed to write file: {}", e))?;

        let diagnostics = collect_file_diagnostics(&path, &content);
        let impact_warning =
            analyze_symbol_impact_radar(&self.workspace_root, &path, &actual_search);

        let mut msg = format!("Successfully replaced text in {}{}", path.display(), repair_msg);
        if let Some(adv_text) = format_arch_advisories(&path, &content) {
            msg.push_str(&adv_text);
        }

        let has_compiler_errors = diagnostics
            .iter()
            .any(|d| d.get("severity").and_then(|s| s.as_str()) == Some("error"));
        if has_compiler_errors {
            let error_summaries: Vec<String> = diagnostics
                .iter()
                .filter(|d| d.get("severity").and_then(|s| s.as_str()) == Some("error"))
                .map(|d| {
                    let line = d.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
                    let m = d.get("message").and_then(|m| m.as_str()).unwrap_or("syntax error");
                    format!("- Line {}: {}", line, m)
                })
                .collect();
            msg.push_str(&format!(
                "\n\n⚠️ [CRITICAL COMPILER/SYNTAX ERROR DETECTED]: The file contains syntax/compilation errors after this edit!\n{}\nAction required: You must IMMEDIATELY fix these errors before declaring the task finished!",
                error_summaries.join("\n")
            ));
        }

        let mut res = json!({
            "success": true,
            "message": msg,
            "diagnostics": diagnostics
        });
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
            .or_else(|| args.get("TargetFile"))
            .and_then(|v| v.as_str())
            .ok_or("Missing path")?;
        let path = resolve_safe_path(path_str, &self.workspace_root)?;
        if !path.exists() {
            return Err(format!("File not found: {}", path_str));
        }

        let mut content =
            std::fs::read_to_string(&path).map_err(|e| format!("Failed to read file: {}", e))?;

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
            } else if let Some(alignment) =
                crate::domains::version_control::diff_relocation::AnchorAligner::align(
                    &content,
                    search_text,
                )
            {
                alignment.matched_text
            } else {
                let _ = std::fs::write(&path, original_content);
                return Err(format!(
                    "Search text block not found in file even after anchor self-repair. Block starting with: {:.50}...",
                    search_text
                ));
            };

            record_shadow_backup(&self.workspace_root, &path, "replace_in_file");
            content = content.replace(&actual_search, replace_text);
            replaced_count += 1;
        }

        std::fs::write(&path, &content).map_err(|e| format!("Failed to write file: {}", e))?;
        let diagnostics = collect_file_diagnostics(&path, &content);
        let mut msg = format!("Successfully applied {} refactoring blocks.", replaced_count);
        if let Some(adv_text) = format_arch_advisories(&path, &content) {
            msg.push_str(&adv_text);
        }

        let mut res = json!({
            "status": "success",
            "blocks_replaced": replaced_count,
            "message": msg,
            "diagnostics": diagnostics
        });

        if let Some(impact) = analyze_symbol_impact_radar(&self.workspace_root, &path, &content) {
            res["impact_radar"] = impact;
        }

        Ok(res)
    }
}
