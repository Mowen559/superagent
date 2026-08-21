//! 终端与进程控制工具 (Shell & Subprocess Tools)
//! 吸收 DeepSeek Harness 防御性模式 (Defensive Patterns):
//! 1. 环境变量脱敏 (Sanitize Sensitive Credentials: KEY, SECRET, TOKEN, PASSWORD)
//! 2. 正交结果独立上报 (timed_out, exit_code, success, signal)
//! 3. 完整的超时回收与子进程完全停稳 Disposal

use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

/// 提取脱敏后的系统环境变量，防止将宿主 API 密钥泄露给命令输出或日志
fn get_sanitized_env() -> HashMap<String, String> {
    let sensitive_keywords = ["KEY", "SECRET", "TOKEN", "PASSWORD"];
    std::env::vars()
        .filter(|(k, _)| {
            let k_upper = k.to_uppercase();
            !sensitive_keywords.iter().any(|kw| k_upper.contains(kw))
        })
        .collect()
}

use std::sync::Arc;
use crate::services::brain::capabilities::shell::ShellService;

/// 终端命令执行工具 (Run Command) - 消费者 (Capability Seam: Consumer)
pub struct RunCommandTool {
    shell_service: Arc<dyn ShellService>,
}

impl RunCommandTool {
    pub fn new(shell_service: Arc<dyn ShellService>) -> Self {
        Self { shell_service }
    }
}

impl ToolExecutor for RunCommandTool {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn description(&self) -> &'static str {
        "Execute a command in the system shell (PowerShell on Windows, Bash on Linux/macOS) and return its output safely."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "CommandLine": {
                    "type": "string",
                    "description": "The exact command line string to execute."
                },
                "Cwd": {
                    "type": "string",
                    "description": "The current working directory for the command."
                },
                "RunInBackground": {
                    "type": "boolean",
                    "description": "If true, the command will be spawned in the background and return a process_id immediately. You can manage it later using ManageProcessTool."
                },
                "TimeoutSeconds": {
                    "type": "integer",
                    "description": "Optional timeout in seconds for foreground execution (default: 60)."
                },
                "rtk_hook": {
                    "type": "boolean",
                    "description": "If true, invokes the RTK token killer to compress massive shell outputs (like large git diffs or massive logs) into structured AST skeletons before returning them."
                },
                "ApprovalTicket": {
                    "type": "string",
                    "description": "If the command is highly destructive (like rm, del, drop), you MUST provide an approval ticket obtained via AskUserTool."
                }
            },
            "required": ["CommandLine", "Cwd"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: run_command]
- Role: Motor Organ (运动中枢)
- Note: The underlying Rust neuron has a "Spinal Reflex" (脊髓反射) hardwired. When you execute this action, the system will automatically call `ShadowGit` at the bottom layer for an imperceptible snapshot backup.
- Usage: For starting servers or long-blocking scripts, you MUST set `RunInBackground: true`.
- Danger: Highly destructive commands (like rm, del, cargo clean) require an 'ApprovalTicket' from AskUserTool.
- RTK Interceptor: If you expect a massive output (like a global codebase diff), set `rtk_hook: true` to transparently compress the output into a skeleton, saving tokens.
- Circuit Breaker: Outputs exceeding 100,000 bytes will be aggressively truncated to protect your context window.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let cmd_str = args
            .get("CommandLine")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'CommandLine' argument")?;

        let cwd_str = args
            .get("Cwd")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'Cwd' argument")?;

        let has_ticket = args.get("ApprovalTicket").and_then(|v| v.as_str()).is_some();
        let cmd_lower = cmd_str.to_lowercase();
        let is_dangerous = cmd_lower.contains("rm -rf") || cmd_lower.contains("del ") || cmd_lower.contains("cargo clean");
        
        if is_dangerous && !has_ticket {
            return Err("ERROR: This command is highly destructive (Interaction Approval Required). You MUST first use AskUserTool to explain what you are deleting and request permission. Once the user approves, include the user's reply in the 'ApprovalTicket' parameter to proceed.".to_string());
        }

        let timeout_secs = args
            .get("TimeoutSeconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);


        let run_in_bg = args.get("RunInBackground").and_then(|v| v.as_bool()).unwrap_or(false);
        let enable_rtk_hook = args.get("rtk_hook").and_then(|v| v.as_bool()).unwrap_or(false);

        // Delegate to the actual capability seam provider
        let mut result = self.shell_service.execute(cmd_str, cwd_str, run_in_bg, timeout_secs)?;

        // If it's running in background, we don't intercept logs here
        if run_in_bg {
            return Ok(result);
        }

        // Post-execution Interceptor: Overload Circuit Breaker and RTK Hook
        if let Some(stdout_val) = result.get_mut("stdout") {
            if let Some(stdout_str) = stdout_val.as_str() {
                let mut output_str = stdout_str.to_string();
                let original_len = output_str.len();

                // 1. RTK Hook compression requested by LLM
                if enable_rtk_hook {
                    use crate::services::token_filter::{compress, FilterLevel};
                    eprintln!("\x1b[35m[Terminal Hook] RTK Intercepting and compressing shell output...\x1b[0m");
                    // We assume shell output usually contains code or diffs. We trick the filter to compress it aggressively.
                    let filter_result = compress(&output_str, "shell_output.txt", FilterLevel::Aggressive);
                    output_str = filter_result.content;
                    result["rtk_hook_engaged"] = json!(true);
                    result["original_size_bytes"] = json!(original_len);
                }

                // 2. 100,000 Bytes Hard Truncation Circuit Breaker (if still too large)
                if output_str.len() > 100_000 {
                    eprintln!("\x1b[31m[System Guard] Shell output exceeded 100,000 bytes circuit breaker. Truncating...\x1b[0m");
                    let mut truncated = output_str[..100_000].to_string();
                    truncated.push_str("\n\n... [TRUNCATED BY CIRCUIT BREAKER: Output exceeded 100,000 bytes. If you need the full logs, please use `> file.txt` to redirect the output to a file and read it with view_file pagination] ...");
                    output_str = truncated;
                    result["circuit_breaker"] = json!("triggered: >100,000 bytes");
                }

                // Update the JSON with the finalized output string
                result["stdout"] = json!(output_str);
            }
        }

        Ok(result)
    }
}
