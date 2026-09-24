use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::domains::code_review::CodeReviewEngine;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 生产级代码审查工具 (Code Review Tool)
/// 借鉴 Alibaba OpenCodeReview 架构，融合五重门降噪过滤、拓扑语义分组与高危规则库
pub struct CodeReviewTool {
    pub workspace_root: PathBuf,
}

impl CodeReviewTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for CodeReviewTool {
    fn name(&self) -> &'static str {
        "code_review"
    }

    fn description(&self) -> &'static str {
        "Perform industrial-grade automated code review over current workspace changes or specific files. Powered by Alibaba OpenCodeReview architecture, it applies 5-gate noise filtering, topology-based change grouping, and multi-language rulesets (detecting NPEs, concurrency hazards, resource leaks, and SQL injections) with line-level precision."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["workspace", "files"],
                    "description": "Review mode: 'workspace' reviews all uncommitted git changes (default); 'files' reviews specific target files."
                },
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of relative file paths to review."
                }
            }
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: code_review]
- Role: Pre-commit Quality & Security Gate (改后代码审查门禁)
- Usage: Invoke `code_review` after editing files or before committing code to intercept critical bugs, null pointer exceptions, concurrency hazards, and resource leaks.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let files: Option<Vec<String>> = args
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            });

        let engine = CodeReviewEngine::new(self.workspace_root.clone());
        let report = engine
            .review_workspace(files.as_deref())
            .map_err(|e| format!("Code review failed: {}", e))?;

        serde_json::to_value(report).map_err(|e| e.to_string())
    }
}
