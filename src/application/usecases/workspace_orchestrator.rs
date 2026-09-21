use std::sync::Arc;
use axum::http::StatusCode;
use serde_json::Value;

pub struct WorkspaceOrchestrator {}

impl WorkspaceOrchestrator {
    pub fn new() -> Self { Self {} }
    pub async fn init_workspace(&self, path: &str) -> Result<Value, StatusCode> {
        Ok(serde_json::json!({"status": "success"}))
    }
}

