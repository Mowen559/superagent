use std::sync::Arc;
use axum::http::StatusCode;
use serde_json::Value;

pub struct CodebaseUnderstandingUsecase {}

impl CodebaseUnderstandingUsecase {
    pub fn new() -> Self { Self {} }
    pub async fn analyze(&self, path: &str) -> Result<Value, StatusCode> {
        Ok(serde_json::json!({"status": "analyzed"}))
    }
}

