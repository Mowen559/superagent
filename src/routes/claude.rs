//! Local Claude Code CLI endpoints: availability probe, settings CRUD, and
//! headless prompt runs (background tasks polled via /api/tasks).

use std::sync::Arc;
use axum::{Router, routing::{get, post}, extract::State, Json};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use crate::AppState;
use crate::config::claude_settings;
use crate::services::{claude_cli, task_manager::TaskManager};

#[derive(Deserialize)]
pub struct ClaudeRunBody {
    pub prompt: String,
    pub project: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/claude/status", get(claude_status))
        .route("/api/claude/settings", get(get_settings).put(put_settings))
        .route("/api/claude/run", post(run))
}

async fn claude_status() -> Json<serde_json::Value> {
    let version = claude_cli::detect_version().await;
    let settings = claude_settings::read_settings();
    Json(json!({
        "installed": version.is_some(),
        "version": version,
        "settingsPath": claude_settings::settings_path().to_string_lossy(),
        "model": settings.get("model").cloned().unwrap_or_default(),
    }))
}

async fn get_settings() -> Json<serde_json::Value> {
    claude_settings::ensure_persisted();
    Json(claude_settings::read_settings())
}

async fn put_settings(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    Json(claude_settings::write_settings(body))
}

async fn run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ClaudeRunBody>,
) -> Response {
    if body.prompt.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing prompt" }))).into_response();
    }
    let task_manager = Arc::new(TaskManager::new(&state.app_paths));
    let task_id = claude_cli::run_prompt(
        task_manager,
        body.project.clone(),
        body.prompt.clone(),
        Some(state.runtime_config.rtk_bin.clone()),
    );
    Json(json!({ "taskId": task_id, "status": "running" })).into_response()
}
