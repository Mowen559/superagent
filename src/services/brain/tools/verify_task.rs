use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use crate::services::brain::capabilities::shell::{LocalShellProvider, ShellService};
use crate::domains::agent_cognitive::executor::ToolExecutor;

pub struct VerifyTaskTool {
    pub shell: Arc<LocalShellProvider>,
    pub workspace_root: PathBuf,
}

impl VerifyTaskTool {
    pub fn new(shell: Arc<LocalShellProvider>, workspace_root: PathBuf) -> Self {
        Self { shell, workspace_root }
    }

    fn extract_failure_highlights(output: &str) -> Vec<String> {
        let mut highlights = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("error[")
                || trimmed.starts_with("error:")
                || trimmed.contains("FAILED")
                || trimmed.contains("AssertionError")
                || trimmed.contains("Exception in thread")
                || trimmed.contains("BUILD FAILURE")
                || trimmed.contains("panic in")
            {
                highlights.push(trimmed.chars().take(180).collect());
                if highlights.len() >= 10 {
                    break;
                }
            }
        }
        highlights
    }
}

impl ToolExecutor for VerifyTaskTool {
    fn name(&self) -> &'static str {
        "verify_task_tool"
    }

    fn description(&self) -> &'static str {
        "Execute automated tests or build verification commands (e.g. 'cargo check --tests', 'mvn test -Dtest=...', 'npm test') to validate code changes before concluding. Automatically captures compiler/assertion failures and provides structured self-healing guidance."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The exact verification or test command to execute (e.g. 'cargo check --tests', 'npm test')."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional subdirectory relative to workspace root to run the test in."
                }
            },
            "required": ["command"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: verify_task_tool]
- Role: Test-Driven Verification Gate (SWE-agent style)
- Rule: NEVER finish an execution task without running tests/checks. Use `verify_task_tool` to confirm all assertions pass before claiming completion.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "command is required for verify_task_tool".to_string())?;

        let run_dir = if let Some(sub) = args.get("cwd").and_then(|v| v.as_str()) {
            self.workspace_root.join(sub)
        } else {
            self.workspace_root.clone()
        };
        let cwd_str = run_dir.to_string_lossy().to_string();

        let output_val = self.shell.execute(command, &cwd_str, false, 90)?;
        let exit_code = output_val.get("exit_code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let stdout = output_val.get("stdout").and_then(|s| s.as_str()).unwrap_or("");
        let stderr = output_val.get("stderr").and_then(|s| s.as_str()).unwrap_or("");
        let is_success = exit_code == 0;

        let full_output = format!("{}\n{}", stdout, stderr);

        if is_success {
            Ok(json!({
                "status": "passed",
                "exit_code": 0,
                "command": command,
                "summary": "Verification succeeded! All tests/checks passed without errors.",
                "output_tail": full_output.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
            }))
        } else {
            let failures = Self::extract_failure_highlights(&full_output);
            Ok(json!({
                "status": "failed",
                "exit_code": exit_code,
                "command": command,
                "failure_count": failures.len(),
                "failures": failures,
                "suggested_action": "Verification failed! Do NOT claim task completion. Diagnose the reported failures, inspect the failing files, formulate a fix, and re-run verify_task_tool.",
                "output_tail": full_output.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_failure_highlights() {
        let output = r#"
            Compiling foo v0.1.0
            error[E0308]: mismatched types in src/main.rs:10:5
            error: could not compile `foo` due to 1 previous error
            Finished test
        "#;
        let highlights = VerifyTaskTool::extract_failure_highlights(output);
        assert_eq!(highlights.len(), 2);
        assert!(highlights[0].contains("error[E0308]"));
    }
}
