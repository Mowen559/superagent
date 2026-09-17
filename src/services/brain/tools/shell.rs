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

fn normalize_path(p: &std::path::Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    let stripped = if s.starts_with(r"\\?\") {
        &s[4..]
    } else {
        &s
    };
    std::path::PathBuf::from(stripped.to_lowercase())
}

use crate::services::brain::capabilities::shell::ShellService;
use std::sync::Arc;

/// 终端命令执行工具 (Run Command) - 消费者 (Capability Seam: Consumer)
pub struct RunCommandTool {
    shell_service: Arc<dyn ShellService>,
    workspace_root: std::path::PathBuf,
}

impl RunCommandTool {
    pub fn new(shell_service: Arc<dyn ShellService>, workspace_root: std::path::PathBuf) -> Self {
        Self {
            shell_service,
            workspace_root,
        }
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
                    "description": "Optional current working directory. Default is '.' (the workspace root). Relative paths are resolved against the workspace root."
                },
                "RunInBackground": {
                    "type": "boolean",
                    "description": "If true, the command will be spawned in the background and return a process_id immediately. You can manage it later using ManageProcessTool."
                },
                "TimeoutSeconds": {
                    "type": "integer",
                    "description": "Optional timeout in seconds for foreground execution (default: 60)."
                },
                "ApprovalTicket": {
                    "type": "string",
                    "description": "If the command is highly destructive (like rm, del, drop), you MUST provide an approval ticket obtained via AskUserTool."
                }
            },
            "required": ["CommandLine"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: run_command]
- Role: Motor Organ (运动中枢)
- Note: The underlying Rust neuron has a "Spinal Reflex" (脊髓反射) hardwired. When you execute this action, the system will automatically call `ShadowGit` at the bottom layer for an imperceptible snapshot backup.
- Usage: For starting servers or long-blocking scripts, you MUST set `RunInBackground: true`.
- Danger: Highly destructive commands (like rm, del, cargo clean) require an 'ApprovalTicket' from AskUserTool.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let cmd_str = args
            .get("CommandLine")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'CommandLine' argument")?;

        let has_ticket = args
            .get("ApprovalTicket")
            .and_then(|v| v.as_str())
            .is_some();
        let cmd_lower = cmd_str.to_lowercase();
        let is_dangerous = cmd_lower.contains("rm -rf")
            || cmd_lower.contains("del ")
            || cmd_lower.contains("cargo clean");

        if is_dangerous && !has_ticket {
            return Err("ERROR: This command is highly destructive (Interaction Approval Required). You MUST first use AskUserTool to explain what you are deleting and request permission. Once the user approves, include the user's reply in the 'ApprovalTicket' parameter to proceed.".to_string());
        }

        // 1. 命令行文本自愈：若大模型在命令行参数中硬编码了历史 run 的沙箱绝对路径，自愈替换为当前 workspace_root
        let mut repaired_cmd = cmd_str.to_string();
        if repaired_cmd.contains(".aindex-hub") && repaired_cmd.contains("runs") && repaired_cmd.contains("workspace") {
            if let Ok(re) = regex::Regex::new(r#"[A-Za-z]:[\\/][^"'\s\n]*\.aindex-hub[\\/]runs[\\/][^\\/]+[\\/]workspace"#) {
                let current_ws = self.workspace_root.to_string_lossy();
                repaired_cmd = re.replace_all(&repaired_cmd, current_ws.as_ref()).to_string();
            }
        }

        // 2. 智能路径解析与自愈纠偏 (Auto-healing & Defensive path normalization for Cwd)
        let resolved_cwd_buf = match args.get("Cwd").and_then(|v| v.as_str()) {
            None => self.workspace_root.clone(),
            Some(raw) if raw.trim().is_empty() || raw.trim() == "." => self.workspace_root.clone(),
            Some(raw) => {
                let trimmed = raw.trim();
                let norm_slash = trimmed.replace('\\', "/");
                // 若大模型误引用了历史 run 目录或其它沙箱 workspace，自动重定向至当前任务的对应子目录或根目录
                if norm_slash.contains(".aindex-hub") && norm_slash.contains("/runs/") && norm_slash.contains("/workspace") {
                    if let Some(pos) = norm_slash.find("/workspace") {
                        let sub = &norm_slash[pos + "/workspace".len()..];
                        let sub_clean = sub.trim_start_matches('/');
                        if !sub_clean.is_empty() && sub_clean != "." {
                            let candidate = self.workspace_root.join(sub_clean);
                            if candidate.is_dir() {
                                candidate
                            } else {
                                self.workspace_root.clone()
                            }
                        } else {
                            self.workspace_root.clone()
                        }
                    } else {
                        self.workspace_root.clone()
                    }
                } else {
                    let path = std::path::Path::new(trimmed);
                    if path.is_absolute() {
                        let norm_target = normalize_path(path);
                        let norm_root = normalize_path(&self.workspace_root);
                        if norm_target.starts_with(&norm_root) {
                            path.to_path_buf()
                        } else {
                            // 自愈：若超出工作区范围，自动纠偏回当前工作区根目录
                            self.workspace_root.clone()
                        }
                    } else {
                        let candidate = self.workspace_root.join(path);
                        if candidate.is_dir() {
                            candidate
                        } else {
                            self.workspace_root.clone()
                        }
                    }
                }
            }
        };

        let target_cwd_str = resolved_cwd_buf.to_string_lossy().to_string();

        let timeout_secs = args
            .get("TimeoutSeconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        let run_in_bg = args
            .get("RunInBackground")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Delegate to the actual capability seam provider
        self.shell_service
            .execute(&repaired_cmd, &target_cwd_str, run_in_bg, timeout_secs)
    }
}
