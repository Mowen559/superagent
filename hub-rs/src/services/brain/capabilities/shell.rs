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
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(serde_json::json!({
            "exit_code": output.status.code(),
            "stdout": stdout,
            "stderr": stderr
        }))
    }
}
