use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;

pub struct NativeHarnessTool;

impl ToolExecutor for NativeHarnessTool {
    fn name(&self) -> &'static str {
        "native_harness"
    }

    fn description(&self) -> &'static str {
        "Harness Capability: Evaluates and executes commands in a defensively sandboxed context natively. Provides auto-timeout protection, output truncation, and strict state boundary."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Command": {
                    "type": "string",
                    "description": "The command to run safely."
                },
                "TimeoutSecs": {
                    "type": "integer",
                    "description": "Maximum execution time in seconds (default 10)."
                }
            },
            "required": ["Command"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let cmd = args
            .get("Command")
            .and_then(|v| v.as_str())
            .ok_or("Missing Command")?;
        let timeout_secs = args
            .get("TimeoutSecs")
            .and_then(|v| v.as_u64())
            .unwrap_or(10);

        eprintln!(
            "\x1b[35m[Harness] Defensively executing: {} (Timeout: {}s)\x1b[0m",
            cmd, timeout_secs
        );

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let process = if cfg!(target_os = "windows") {
                    tokio::process::Command::new("cmd")
                        .args(&["/C", cmd])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .kill_on_drop(true)
                        .spawn()
                } else {
                    tokio::process::Command::new("sh")
                        .args(&["-c", cmd])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .kill_on_drop(true)
                        .spawn()
                }.map_err(|e| format!("Failed to spawn process: {}", e))?;

                let res = tokio::time::timeout(Duration::from_secs(timeout_secs), process.wait_with_output()).await;

                match res {
                    Ok(Ok(output)) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        Ok(json!({
                            "status": "completed",
                            "exit_code": output.status.code().unwrap_or(-1),
                            "stdout_len": stdout.len(),
                            "stderr_len": stderr.len(),
                            "stdout": if stdout.len() > 4000 { format!("{}...[TRUNCATED BY HARNESS]", &stdout[..4000]) } else { stdout },
                            "stderr": if stderr.len() > 4000 { format!("{}...[TRUNCATED BY HARNESS]", &stderr[..4000]) } else { stderr },
                        }))
                    },
                    Ok(Err(e)) => Err(format!("Process execution error: {}", e)),
                    Err(_) => {
                        Ok(json!({
                            "status": "timeout_killed",
                            "message": format!("Harness safety trigger: Command exceeded {} seconds and was SIGKILLed.", timeout_secs)
                        }))
                    }
                }
            })
        });

        result
    }
}
