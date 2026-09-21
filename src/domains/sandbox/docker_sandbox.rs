use super::sandbox_runtime::{ExecutionResult, SandboxRuntime};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct DockerSandbox {
    container_id: String,
}

impl DockerSandbox {
    pub fn new(container_id: String) -> Self { Self { container_id } }
}

#[async_trait]
impl SandboxRuntime for DockerSandbox {
    async fn execute(&self, _cmd: &str, _args: &[&str], _env: HashMap<String, String>) -> Result<ExecutionResult, String> {
        Ok(ExecutionResult { exit_code: 0, stdout: "Docker Exec Mock".to_string(), stderr: String::new() })
    }
    async fn write_file(&self, _path: &str, _content: &str) -> Result<(), String> { Ok(()) }
    async fn read_file(&self, _path: &str) -> Result<String, String> { Ok(String::new()) }
}

