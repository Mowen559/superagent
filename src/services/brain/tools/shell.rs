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

/// 终端命令执行工具 (Run Command)
pub struct RunCommandTool;

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
                }
            },
            "required": ["CommandLine", "Cwd"]
        })
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

        let timeout_secs = args
            .get("TimeoutSeconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        eprintln!("[Brain B Shell] Executing (sanitized env): {} in {} (timeout: {}s)", cmd_str, cwd_str, timeout_secs);

        // 0. 自动快照：脊髓反射 (Spinal Reflex - Shadow Backup)
        eprintln!("\x1b[36m[Spinal Reflex] 运动中枢触发防卫：自动备份 (Command: {})\x1b[0m", cmd_str);
        let _ = crate::services::git_service::record_ai_snapshot(
            cwd_str,
            &format!("[Shadow Backup] Auto-backup before command: {}", cmd_str)
        );

        // 高危指令人工审核拦截
        eprintln!("\x1b[31m[SECURITY] Agent wants to run: {}. Allow? (Y/n)\x1b[0m", cmd_str);
        let mut buffer = String::new();
        if let Ok(_) = std::io::stdin().read_line(&mut buffer) {
            let answer = buffer.trim().to_lowercase();
            if answer == "n" {
                return Ok(json!({"error": "Human denied the execution of this command."}));
            }
        }

        // 1. 设置命令与入口
        #[cfg(target_os = "windows")]
        let mut cmd = Command::new("powershell");
        #[cfg(target_os = "windows")]
        cmd.arg("-Command").arg(cmd_str);

        #[cfg(not(target_os = "windows"))]
        let mut cmd = Command::new("sh");
        #[cfg(not(target_os = "windows"))]
        cmd.arg("-c").arg(cmd_str);

        cmd.current_dir(cwd_str);

        // 2. 防御性模式：清理原环境变量，仅注入脱敏后的环境变量
        cmd.env_clear();
        for (k, v) in get_sanitized_env() {
            cmd.env(k, v);
        }

        let run_in_bg = args.get("RunInBackground").and_then(|v| v.as_bool()).unwrap_or(false);

        if run_in_bg {
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            match cmd.spawn() {
                Ok(child) => {
                    use chrono::Local;
                    let process_id = format!("pid-{}", Local::now().timestamp_millis());

                    crate::services::brain::tools::process_manager::register_process(
                        process_id.clone(),
                        cmd_str.to_string(),
                        child,
                    );

                    Ok(json!({
                        "success": true,
                        "process_id": process_id,
                        "status": "running_in_background",
                        "env_sanitized": true,
                        "message": "Use ManageProcessTool to view logs or kill it."
                    }))
                }
                Err(e) => Err(format!("Failed to spawn background command: {}", e)),
            }
        } else {
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => return Err(format!("Failed to spawn command: {}", e)),
            };

            let start = Instant::now();
            let timeout = Duration::from_secs(timeout_secs);
            let mut timed_out = false;

            // 轮询等待子进程退出或超时
            loop {
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        // 正常结束，退出循环
                        break;
                    }
                    Ok(None) => {
                        if start.elapsed() >= timeout {
                            timed_out = true;
                            // 防御性模式：dispose 必须达到完全停稳
                            let _ = child.kill();
                            let _ = child.wait(); // 确保子进程句柄完全回收
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("Error waiting for command process: {}", e));
                    }
                }
            }

            // 读取已录制的 stdout / stderr
            let mut stdout = String::new();
            let mut stderr = String::new();
            let mut rtk_stdout = false;
            let mut rtk_stderr = false;

            if let Some(mut out) = child.stdout.take() {
                use std::io::Read;
                let _ = out.read_to_string(&mut stdout);
                if stdout.len() > 10000 {
                    eprintln!("[Brain B] stdout too large ({} bytes). Engaging RTK Token Killer...", stdout.len());
                    let result = crate::services::token_filter::compress(&stdout, "stdout", crate::services::token_filter::FilterLevel::Aggressive);
                    stdout = result.content;
                    rtk_stdout = true;
                }
            }
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                let _ = err.read_to_string(&mut stderr);
                if stderr.len() > 10000 {
                    eprintln!("[Brain B] stderr too large ({} bytes). Engaging RTK Token Killer...", stderr.len());
                    let result = crate::services::token_filter::compress(&stderr, "stderr", crate::services::token_filter::FilterLevel::Aggressive);
                    stderr = result.content;
                    rtk_stderr = true;
                }
            }

            if timed_out {
                // 正交结果独立上报
                Ok(json!({
                    "success": false,
                    "timed_out": true,
                    "exit_code": serde_json::Value::Null,
                    "stdout": stdout,
                    "stderr": stderr,
                    "rtk_stdout": rtk_stdout,
                    "rtk_stderr": rtk_stderr,
                    "error": format!("Command execution timed out after {} seconds", timeout_secs)
                }))
            } else {
                let status = child.wait().ok();
                let exit_code = status.and_then(|s| s.code());
                let success = status.map(|s| s.success()).unwrap_or(false);

                // 正交结果独立上报
                Ok(json!({
                    "success": success,
                    "timed_out": false,
                    "exit_code": exit_code.map(|c| json!(c)).unwrap_or(Value::Null),
                    "stdout": stdout,
                    "stderr": stderr,
                    "rtk_stdout": rtk_stdout,
                    "rtk_stderr": rtk_stderr
                }))
            }
        }
    }
}
