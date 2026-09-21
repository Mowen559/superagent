use super::sandbox_runtime::{ExecutionResult, SandboxRuntime};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct LocalSandbox {}

impl LocalSandbox {
    pub fn new() -> Self { Self {} }
}

#[async_trait]
impl SandboxRuntime for LocalSandbox {
    async fn execute(&self, cmd: &str, _args: &[&str], _env: HashMap<String, String>) -> Result<ExecutionResult, String> {
        Ok(ExecutionResult { exit_code: 0, stdout: String::new(), stderr: String::new() })
    }
    async fn write_file(&self, _path: &str, _content: &str) -> Result<(), String> { Ok(()) }
    async fn read_file(&self, _path: &str) -> Result<String, String> { Ok(String::new()) }
}

