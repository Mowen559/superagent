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
        let output = tokio::process::Command::new("cmd")
            .args(&["/c", cmd])
            .output()
            .await
            .map_err(|e| format!("Failed to execute verification command: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!("TDD Verification Failed. \nSTDOUT:\n{}\nSTDERR:\n{}", stdout, stderr));
        }

        Ok(())
    }
}

