//! 终端与进程控制工具 (Shell & Subprocess Tools)
//! 赋予大脑 B 执行系统命令的能力，这是实现大模型“敲键盘”、自我编译和环境交互的核心

use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::process::Command;

/// 终端命令执行工具 (Run Command)
pub struct RunCommandTool;

impl ToolExecutor for RunCommandTool {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn description(&self) -> &'static str {
        "Execute a command in the system shell (PowerShell on Windows, Bash on Linux/macOS) and return its output."
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

        eprintln!("Executing command: {} in {}", cmd_str, cwd_str);

        // Windows 下使用 powershell，Linux 下使用 sh
        #[cfg(target_os = "windows")]
        let mut cmd = Command::new("powershell");
        #[cfg(target_os = "windows")]
        cmd.arg("-Command").arg(cmd_str);

        #[cfg(not(target_os = "windows"))]
        let mut cmd = Command::new("sh");
        #[cfg(not(target_os = "windows"))]
        cmd.arg("-c").arg(cmd_str);

        cmd.current_dir(cwd_str);

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
                        child
                    );

                    Ok(json!({
                        "success": true,
                        "process_id": process_id,
                        "status": "running_in_background",
                        "message": "Use ManageProcessTool to view logs or kill it."
                    }))
                }
                Err(e) => Err(format!("Failed to spawn background command: {}", e)),
            }
        } else {
            match cmd.output() {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    
                    Ok(json!({
                        "success": output.status.success(),
                        "exit_code": output.status.code().unwrap_or(-1),
                        "stdout": stdout,
                        "stderr": stderr
                    }))
                }
                Err(e) => Err(format!("Failed to execute command: {}", e)),
            }
        }
    }
}
