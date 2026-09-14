use serde_json::Value;
use std::sync::Arc;

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

/// 能力缝隙: 物理提供者 (Capability Seam: Provider)
pub struct LocalShellProvider;

impl ShellService for LocalShellProvider {
    fn execute(
        &self,
        cmd: &str,
        cwd: &str,
        run_in_bg: bool,
        _timeout_secs: u64,
    ) -> Result<Value, String> {
        eprintln!("[LocalShellProvider] Physically executing command: {}", cmd);

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

        if run_in_bg {
            command.stdin(std::process::Stdio::piped());
            command.stdout(std::process::Stdio::piped());
            command.stderr(std::process::Stdio::piped());
            let child = command.spawn().map_err(|e| e.to_string())?;
            return Ok(serde_json::json!({
                "status": "spawned",
                "pid": child.id()
            }));
        }

        let output = command.output().map_err(|e| e.to_string())?;
        let stdout_raw = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr_raw = String::from_utf8_lossy(&output.stderr).to_string();

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

        Ok(serde_json::json!({
            "exit_code": output.status.code(),
            "stdout": stdout_res.content,
            "stderr": stderr_res.content,
            "rtk_metrics": {
                "stdout_reduction_percent": stdout_res.reduction_percent,
                "tokens_saved_est": stdout_res.original_tokens_est.saturating_sub(stdout_res.compressed_tokens_est),
                "tracker_stats": crate::services::token_filter::TokenSavingsTracker::global().get_stats()
            }
        }))
    }
}
