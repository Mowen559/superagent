use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TddConfig {
    pub max_retries: usize,
    pub verification_command: Option<String>,
}

impl Default for TddConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            verification_command: None,
        }
    }
}

pub struct TddEvaluator {
    config: TddConfig,
}

impl TddEvaluator {
    pub fn new(config: TddConfig) -> Self {
        Self { config }
    }

    /// Pre-flight check before returning to user.
    /// If it fails, returns Err with the compiler/test error string.
    pub async fn evaluate_codebase(&self, cmd: &str) -> Result<(), String> {
        // --- FIX 2: TDD Deadlock Prevention ---
        // 1. Timeout prevents eternal hangs on `npm start` or infinite loops
        // 2. kill_on_drop(true) guarantees the zombie cmd process is murdered when timeout drops the Future
        let timeout_duration = std::time::Duration::from_secs(120);
        let output_result = tokio::time::timeout(
            timeout_duration,
            tokio::process::Command::new("cmd")
                .args(&["/c", cmd])
                .kill_on_drop(true)
                .output()
        ).await;

        let output = match output_result {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(format!("Failed to execute verification command: {}", e)),
            Err(_) => return Err(format!("TDD Verification strictly timed out after {} seconds. The command likely launched a persistent dev server or hung infinitely.", timeout_duration.as_secs())),
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!("TDD Verification Failed. \nSTDOUT:\n{}\nSTDERR:\n{}", stdout, stderr));
        }

        Ok(())
    }
}

