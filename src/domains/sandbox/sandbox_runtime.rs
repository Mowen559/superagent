use async_trait::async_trait;
use std::collections::HashMap;

pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait]
pub trait SandboxRuntime: Send + Sync {
    async fn execute(&self, cmd: &str, args: &[&str], env: HashMap<String, String>) -> Result<ExecutionResult, String>;
    async fn write_file(&self, path: &str, content: &str) -> Result<(), String>;
    async fn read_file(&self, path: &str) -> Result<String, String>;
}

