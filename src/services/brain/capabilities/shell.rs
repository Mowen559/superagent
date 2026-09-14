use serde_json::Value;
use std::io::Read;
use std::time::{Duration, Instant};

/// 能力缝隙: 服务定义 (Capability Seam: Service Definition)
pub trait ShellService: Send + Sync {
    fn execute(
        &self,
        cmd: &str,
        cwd: &str,
        run_in_bg: bool,
        timeout_secs: u64,
    ) -> Result<Value, String>;
}

/// 智能探测终端输出中的交互式提示 (Interactive Prompt Detector)
pub fn detect_interactive_prompt(output: &str) -> Option<&'static str> {
    let trimmed = output.trim_end();
    if trimmed.ends_with("[y/N]")
        || trimmed.ends_with("[Y/n]")
        || trimmed.ends_with("(y/n)")
        || trimmed.ends_with("[y/n]?")
    {
        Some("Confirmation prompt [y/N]")
    } else if trimmed.ends_with("Password:")
        || trimmed.ends_with("Enter password:")
        || trimmed.ends_with("Passphrase:")
    {
        Some("Password prompt")
    } else if trimmed.ends_with("Press [Enter] to continue:")
        || trimmed.ends_with("Press any key to continue . . .")
    {
        Some("Press enter prompt")
    } else if trimmed.ends_with("? ")
        || (trimmed.ends_with(':')
            && !trimmed.contains("error:")
            && !trimmed.contains("warning:")
            && !trimmed.contains("http://")
            && !trimmed.contains("https://"))
    {
        Some("Interactive query prompt")
    } else {
        None
    }
}

/// 去除终端输出中的 ANSI / VT100 控制字符 (ANSI Escape Code Stripper)
pub fn strip_ansi_escapes(input: &str) -> String {
    let Ok(re) = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b[()][AB012]") else {
        return input.to_string();
    };
    re.replace_all(input, "").to_string()
}

/// 高危灾难性命令防护门禁 (Catastrophic Command Safety Guard)
pub fn validate_command_safety(cmd: &str) -> Result<(), String> {
    let lower = cmd.trim().to_lowercase();
    let dangerous_patterns = [
        "rm -rf /",
        "rm -rf /*",
        "rm -rf ~",
        "rm -fr /",
        "rmdir /s /q c:\\",
        "rmdir /s /q c:/",
        "rmdir /s /q /",
        "format c:",
        "mkfs.",
        ":(){ :|:& };:",
        "dd if=/dev/zero of=/dev/sd",
        "dd if=/dev/urandom of=/dev/sd",
        "> /dev/sda",
    ];

    for pattern in dangerous_patterns {
        if lower.contains(pattern) {
            return Err(format!(
                "Command rejected by CommandSafetyGuard: Destructive command pattern detected ('{}'). Execution aborted for safety.",
                pattern
            ));
        }
    }

    Ok(())
}

/// 能力缝隙: 物理提供者 (Capability Seam: Provider)
pub struct LocalShellProvider;

impl ShellService for LocalShellProvider {
    fn execute(
        &self,
        cmd: &str,
        cwd: &str,
        run_in_bg: bool,
        timeout_secs: u64,
    ) -> Result<Value, String> {
        // 安全护栏前置过滤
        validate_command_safety(cmd)?;

        eprintln!("[LocalShellProvider] Executing command: {}", cmd);

        let mut command = std::process::Command::new(if cfg!(target_os = "windows") {
            "powershell"
        } else {
            "sh"
        });
        command
            .arg(if cfg!(target_os = "windows") {
                "-Command"
            } else {
                "-c"
            })
            .arg(cmd);
        command.current_dir(cwd);
        command.stdin(std::process::Stdio::piped());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        // 1. 后台长驻进程直接托管到 ProcessManager
        if run_in_bg {
            let child = command.spawn().map_err(|e| e.to_string())?;
            let pid = child.id();
            let proc_id = format!("proc-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            crate::services::brain::tools::process_manager::register_process(
                proc_id.clone(),
                cmd.to_string(),
                child,
            );
            return Ok(serde_json::json!({
                "status": "spawned",
                "process_id": proc_id,
                "pid": pid,
                "message": format!("Command spawned in background as process '{}'. Use manage_process to interact.", proc_id)
            }));
        }

        // 2. 前台执行：具备双向检测、交互式提示探测与秒级超时抢占
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let timeout = Duration::from_secs(if timeout_secs == 0 { 60 } else { timeout_secs });
        let start = Instant::now();

        let mut stdout_pipe = child.stdout.take().ok_or("Failed to open stdout pipe")?;
        let mut stderr_pipe = child.stderr.take().ok_or("Failed to open stderr pipe")?;

        let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
        let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();

        // 异步Stdout流读取线程
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stdout_pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let _ = stdout_tx.send(buf[..n].to_vec());
            }
        });

        // 异步Stderr流读取线程
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr_pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let _ = stderr_tx.send(buf[..n].to_vec());
            }
        });

        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let mut last_output_time = Instant::now();

        loop {
            // 收集新接收到的管道流
            while let Ok(chunk) = stdout_rx.try_recv() {
                stdout_bytes.extend_from_slice(&chunk);
                last_output_time = Instant::now();
            }
            while let Ok(chunk) = stderr_rx.try_recv() {
                stderr_bytes.extend_from_slice(&chunk);
                last_output_time = Instant::now();
            }

            // 检查子进程是否已自然退出
            match child.try_wait() {
                Ok(Some(status)) => {
                    // 读取可能遗留的缓冲区尾巴
                    std::thread::sleep(Duration::from_millis(50));
                    while let Ok(chunk) = stdout_rx.try_recv() {
                        stdout_bytes.extend_from_slice(&chunk);
                    }
                    while let Ok(chunk) = stderr_rx.try_recv() {
                        stderr_bytes.extend_from_slice(&chunk);
                    }

                    let stdout_raw = strip_ansi_escapes(&String::from_utf8_lossy(&stdout_bytes));
                    let stderr_raw = strip_ansi_escapes(&String::from_utf8_lossy(&stderr_bytes));

                    let stdout_res = crate::services::token_filter::compress_by_command(cmd, &stdout_raw);
                    let stderr_res = if stderr_raw.lines().count() > 20 || stderr_raw.len() > 1000 {
                        crate::services::token_filter::compress_terminal_output(&stderr_raw, 60)
                    } else {
                        crate::services::token_filter::compress(
                            &stderr_raw,
                            "log.txt",
                            crate::services::token_filter::FilterLevel::None,
                        )
                    };

                    return Ok(serde_json::json!({
                        "status": "completed",
                        "exit_code": status.code(),
                        "stdout": stdout_res.content,
                        "stderr": stderr_res.content,
                        "duration_ms": start.elapsed().as_millis(),
                        "rtk_metrics": {
                            "stdout_reduction_percent": stdout_res.reduction_percent,
                            "tokens_saved_est": stdout_res.original_tokens_est.saturating_sub(stdout_res.compressed_tokens_est),
                            "tracker_stats": crate::services::token_filter::TokenSavingsTracker::global().get_stats()
                        }
                    }));
                }
                Ok(None) => {
                    let current_stdout = strip_ansi_escapes(&String::from_utf8_lossy(&stdout_bytes));
                    // 交互式提示感知：若进程停顿超过 1.2s 且输出尾部匹配人机交互提示
                    if last_output_time.elapsed() >= Duration::from_millis(1200) {
                        if let Some(prompt_type) = detect_interactive_prompt(&current_stdout) {
                            eprintln!(
                                "\x1b[33m[LocalShellProvider] Interactive prompt detected: {} ({:?})\x1b[0m",
                                prompt_type, current_stdout.lines().last().unwrap_or_default()
                            );
                            let proc_id = format!("proc-{}", &uuid::Uuid::new_v4().to_string()[..8]);
                            crate::services::brain::tools::process_manager::register_process(
                                proc_id.clone(),
                                cmd.to_string(),
                                child,
                            );
                            return Ok(serde_json::json!({
                                "status": "interactive_prompt_detected",
                                "prompt_type": prompt_type,
                                "process_id": proc_id,
                                "prompt_line": current_stdout.lines().last().unwrap_or_default().trim(),
                                "stdout": current_stdout,
                                "message": format!(
                                    "Command paused at interactive prompt: '{}'. Process transferred to background as '{}'. Use manage_process(Action: 'send_input', ProcessId: '{}', Input: '...') to reply.",
                                    prompt_type, proc_id, proc_id
                                )
                            }));
                        }
                    }

                    // 超时抢占检查 (Mid-flight Timeout Preemption)
                    if start.elapsed() >= timeout {
                        eprintln!(
                            "\x1b[31m[LocalShellProvider] Command timed out after {}s; preempting and recovering state...\x1b[0m",
                            timeout.as_secs()
                        );
                        let _ = child.kill();
                        let _ = child.wait(); // 完全停稳，杜绝孤儿僵尸进程

                        let stdout_raw = strip_ansi_escapes(&String::from_utf8_lossy(&stdout_bytes));
                        let stderr_raw = strip_ansi_escapes(&String::from_utf8_lossy(&stderr_bytes));

                        return Ok(serde_json::json!({
                            "status": "timed_out",
                            "exit_code": -1,
                            "timed_out": true,
                            "message": format!("Command timed out after {} seconds and was cleanly preempted.", timeout.as_secs()),
                            "stdout": stdout_raw,
                            "stderr": stderr_raw,
                            "duration_ms": start.elapsed().as_millis(),
                        }));
                    }

                    std::thread::sleep(Duration::from_millis(60));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Process polling error: {}", e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_escapes() {
        let raw = "\x1b[31mError:\x1b[0m Failed to \x1b[1;32mcompile\x1b[0m";
        let cleaned = strip_ansi_escapes(raw);
        assert_eq!(cleaned, "Error: Failed to compile");
    }

    #[test]
    fn test_validate_command_safety() {
        assert!(validate_command_safety("cargo check --tests").is_ok());
        assert!(validate_command_safety("git status").is_ok());

        assert!(validate_command_safety("rm -rf /").is_err());
        assert!(validate_command_safety("rmdir /s /q C:\\").is_err());
        assert!(validate_command_safety("format c:").is_err());
    }

    #[test]
    fn test_detect_interactive_prompt() {
        assert!(detect_interactive_prompt("Do you want to continue? [y/N]").is_some());
        assert!(detect_interactive_prompt("Password:").is_some());
        assert!(detect_interactive_prompt("Compilation finished successfully.").is_none());
    }
}

