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
use crate::services::claude_cli;

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

fn redact_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    if key.len() > 8 {
        format!("{}***{}", &key[0..4], &key[key.len()-4..])
    } else {
        "***".to_string()
    }
}

async fn get_settings() -> Json<serde_json::Value> {
    claude_settings::ensure_persisted();
    let mut settings = claude_settings::read_settings();
    
    if let Some(env) = settings.get_mut("env").and_then(|v| v.as_object_mut()) {
        if let Some(key) = env.get("ANTHROPIC_API_KEY").and_then(|v| v.as_str()) {
            let redacted = redact_api_key(key);
            env.insert("ANTHROPIC_API_KEY".to_string(), serde_json::json!(redacted));
        }
    }
    
    Json(settings)
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
    
    let kind = crate::services::agent::AgentKind::Claude;
    
    let project = body.project.clone().unwrap_or_default();
    let guard = match crate::services::workspace_guard::WorkspaceGuard::new(&state.runtime_config.projects_root) {
        Ok(g) => g,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    };
    let project = if project.is_empty() {
        "".to_string()
    } else {
        match guard.resolve_safe(&project, "") {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e.to_string() }))).into_response(),
        }
    };

    let (task, cancel_rx) = state
        .task_manager
        .create_task("claude", &project, Some(json!({ "agent": "claude-code" })));
    let task_id = task.id.clone();
    state.task_manager.update_status(&task_id, crate::models::task::TaskStatus::Running, None);

    let opts = crate::services::agent::ExecOptions {
        cwd: body.project.filter(|p| !p.is_empty()),
        ..Default::default()
    };

    let state_bg = state.clone();
    let task_bg = task_id.clone();
    let prompt = body.prompt.clone();
    tokio::spawn(async move {
        let backend = crate::services::agent::new(kind);
        let mut session = match backend.execute(prompt, opts, cancel_rx).await {
            Ok(s) => s,
            Err(e) => {
                let msg = e.to_string();
                state_bg.task_manager.append_event(&task_bg, &crate::services::agent::AgentMessage::Error { content: msg.clone() });
                state_bg.task_manager.update_status(&task_bg, crate::models::task::TaskStatus::Failed, Some(msg));
                return;
            }
        };

        while let Some(event) = session.messages.recv().await {
            state_bg.task_manager.append_event(&task_bg, &event);
        }

        match session.result.await {
            Ok(result) => {
                let status = match result.status.as_str() {
                    "completed" => crate::models::task::TaskStatus::Completed,
                    "cancelled" | "aborted" => crate::models::task::TaskStatus::Cancelled,
                    _ => crate::models::task::TaskStatus::Failed,
                };
                let error = if result.error.is_empty() { None } else { Some(result.error) };
                state_bg.task_manager.update_status(&task_bg, status, error);
            }
            Err(e) => {
                state_bg.task_manager.update_status(&task_bg, crate::models::task::TaskStatus::Failed, Some(e.to_string()));
            }
        }
    });

    Json(json!({ "taskId": task_id, "status": "running" })).into_response()
}
