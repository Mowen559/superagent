//! Local coding-agent endpoints (`/api/agents/*`).
//!
//! Surfaces the agent adapter layer (`services::agent`) over HTTP:
//! - `GET  /api/agents`                     — detect every known CLI on PATH.
//! - `POST /api/agents/run`                 — launch a backend as a background
//!   task; structured events land in `{task_id}.events.jsonl` and fan out over
//!   the TaskManager event bus.
//! - `GET  /api/agents/tasks/:id/events`    — SSE stream: replay persisted
//!   events, then follow the live broadcast until the run finishes.

use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;

use crate::models::task::TaskStatus;
use crate::services::agent::{self, AgentKind, AgentMessage, ExecOptions, types::SyncConfigPayload};
use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/agents", get(list_agents))
        .route("/api/agents/run", post(run_agent))
        .route("/api/agents/tasks/{task_id}/events", get(stream_events))
        .route("/api/agents/{kind}/config/sync", post(sync_agent_config))
        .route("/api/agents/{kind}/skills", get(list_agent_skills))
        .route("/api/agents/{kind}/skills/install", post(install_agent_skill))
        .route("/api/agents/{kind}/skills/{skill_name}/update", post(update_agent_skill))
        .route("/api/agents/{kind}/skills/{skill_name}/uninstall", post(uninstall_agent_skill))
        .route("/api/agents/{kind}/mcps", get(list_agent_mcps))
        .route("/api/agents/{kind}/models", get(list_agent_models))
}

/// Probe every known agent CLI and report install/version/support status.
async fn list_agents() -> Json<serde_json::Value> {
    let agents = agent::detect_all().await;
    Json(json!({ "agents": agents }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBody {
    pub agent: String,
    pub prompt: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub resume_session_id: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
}

/// Launch an agent run as a background task. Returns `{ taskId }` immediately;
/// the caller streams progress via `/api/agents/tasks/:id/events`.
async fn run_agent(State(state): State<Arc<AppState>>, Json(body): Json<RunBody>) -> Response {
    if body.prompt.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing prompt" }))).into_response();
    }
    let kind = match AgentKind::from_str(&body.agent) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    };

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
        .create_task("agent", &project, Some(json!({ "agent": body.agent })));
    let task_id = task.id.clone();

    state
        .task_manager
        .update_status(&task_id, TaskStatus::Running, None);

    let opts = ExecOptions {
        cwd: body.project.clone().filter(|p| !p.is_empty()),
        model: body.model.clone().filter(|m| !m.is_empty()),
        resume_session_id: body.resume_session_id.clone().filter(|s| !s.is_empty()),
        thinking_level: body.thinking_level.clone().filter(|t| !t.is_empty()),
        ..Default::default()
    };

    let state_bg = state.clone();
    let task_bg = task_id.clone();
    let prompt = body.prompt.clone();
    tokio::spawn(async move {
        drive_run(state_bg, task_bg, kind, prompt, opts, cancel_rx).await;
    });

    Json(json!({ "taskId": task_id, "status": "running" })).into_response()
}

/// Execute a backend and pump its event stream into the task's event store,
/// then record the final outcome as the task status. Runs on a background task.
async fn drive_run(
    state: Arc<AppState>,
    task_id: String,
    kind: AgentKind,
    prompt: String,
    opts: ExecOptions,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
) {
    let backend = agent::new(kind);
    let mut session = match backend.execute(prompt, opts, cancel_rx).await {
        Ok(s) => s,
        Err(e) => {
            let msg = e.to_string();
            state
                .task_manager
                .append_event(&task_id, &AgentMessage::Error { content: msg.clone() });
            state
                .task_manager
                .update_status(&task_id, TaskStatus::Failed, Some(msg));
            return;
        }
    };

    // Drain streaming events until the backend closes the channel.
    while let Some(event) = session.messages.recv().await {
        state.task_manager.append_event(&task_id, &event);
    }

    // Await the terminal outcome and map it onto the task status.
    match session.result.await {
        Ok(result) => {
            let status = match result.status.as_str() {
                "completed" => TaskStatus::Completed,
                "cancelled" | "aborted" => TaskStatus::Cancelled,
                _ => TaskStatus::Failed,
            };
            let error = if result.error.is_empty() {
                None
            } else {
                Some(result.error.clone())
            };
            state.task_manager.update_status(&task_id, status, error);
        }
        Err(_) => {
            // Backend dropped the result sender without sending — treat as failure.
            state.task_manager.update_status(
                &task_id,
                TaskStatus::Failed,
                Some("agent exited without reporting a result".to_string()),
            );
        }
    }
}

/// SSE endpoint: replay the persisted backlog, then follow the live broadcast.
/// A terminal event (or the channel closing) ends the stream.
async fn stream_events(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe first so no event emitted between backlog-read and subscribe is
    // lost; the backlog is the durable history, the receiver the live tail.
    let backlog = state.task_manager.get_events(&task_id);
    let backlog_stream = stream::iter(backlog.into_iter().map(to_sse_event));

    let boxed_stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = if let Some(receiver) = state.task_manager.subscribe_events(&task_id) {
        let live_stream = BroadcastStream::new(receiver).filter_map(|item| async move {
            match item {
                Ok(event) => Some(to_sse_event(event)),
                // Lagged: subscriber fell behind; skip the missed window rather than
                // abort — the durable jsonl file remains the complete record.
                Err(_) => None,
            }
        });
        Box::pin(backlog_stream.chain(live_stream))
    } else {
        // Task already finished and channel dropped; just send backlog and end stream.
        Box::pin(backlog_stream)
    };

    Sse::new(boxed_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Serialize an [`AgentMessage`] as a named SSE event (`event: <kind>`), with
/// the tagged JSON as the data payload. Infallible: serialization of our own
/// enum cannot fail, and a fallback keeps the stream alive if it somehow does.
fn to_sse_event(message: AgentMessage) -> Result<Event, Infallible> {
    let name = match &message {
        AgentMessage::Text { .. } => "text",
        AgentMessage::Thinking { .. } => "thinking",
        AgentMessage::ToolUse { .. } => "tool-use",
        AgentMessage::ToolResult { .. } => "tool-result",
        AgentMessage::Status { .. } => "status",
        AgentMessage::Error { .. } => "error",
        AgentMessage::Log { .. } => "log",
    };
    let data = serde_json::to_string(&message).unwrap_or_else(|_| "{}".to_string());
    Ok(Event::default().event(name).data(data))
}

/// Synchronize the configuration of a specific agent.
async fn sync_agent_config(
    Path(kind_str): Path<String>,
    Json(payload): Json<SyncConfigPayload>,
) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid agent kind: {}", e) }))).into_response(),
    };

    let backend = agent::new(kind);
    match backend.sync_config(&payload, None).await {
        Ok(_) => Json(json!({ "success": true })).into_response(),
        Err(e) => {
            let err_msg = e.to_string();
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": err_msg }))).into_response()
        }
    }
}

async fn list_agent_skills(Path(kind_str): Path<String>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid agent kind: {}", e) }))).into_response(),
    };

    let spec = agent::discovery::get_agent_discovery_spec(kind);
    let mut cache = agent::skills::ScannerCache::new();
    let skills = agent::skills::scan_skills_for_agent(kind.info().slug, &spec.skill_paths, &mut cache);

    Json(json!({ "skills": skills })).into_response()
}

async fn list_agent_mcps(Path(kind_str): Path<String>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid agent kind: {}", e) }))).into_response(),
    };

    let spec = agent::discovery::get_agent_discovery_spec(kind);
    let mcps = agent::skills::parse_registered_services(&spec.config_path, &spec.mcp_keys);

    Json(json!({ "mcps": mcps })).into_response()
}

async fn install_agent_skill(
    Path(kind_str): Path<String>,
    Json(req): Json<agent::skills_sync::InstallLocalSkillRequest>,
) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid agent kind: {}", e) }))).into_response(),
    };

    match agent::skills_sync::install_or_update_skill_for_agent(kind, req, false).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn update_agent_skill(
    Path((kind_str, skill_name)): Path<(String, String)>,
    Json(req): Json<agent::skills_sync::UpdateLocalSkillRequest>,
) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid agent kind: {}", e) }))).into_response(),
    };

    let spec = agent::discovery::get_agent_discovery_spec(kind);
    let mut cache = agent::skills::ScannerCache::new();
    let detected_skills = agent::skills::scan_skills_for_agent(kind.info().slug, &spec.skill_paths, &mut cache);
    
    let detected = match detected_skills.into_iter().find(|s| s.name.eq_ignore_ascii_case(&skill_name) || s.directory.eq_ignore_ascii_case(&skill_name)) {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Skill '{}' not found", skill_name) }))).into_response(),
    };

    match agent::skills_sync::resolve_update_source(&detected, req) {
        Ok((git_url, skill_path, git_ref)) => {
            let install_req = agent::skills_sync::InstallLocalSkillRequest {
                source_type: agent::skills_sync::LocalSkillSourceType::GitRepo,
                git_url: Some(git_url),
                git_ref,
                skill_path: Some(skill_path),
                local_dir: None,
                target_name: Some(detected.directory),
            };

            match agent::skills_sync::install_or_update_skill_for_agent(kind, install_req, true).await {
                Ok(res) => Json(res).into_response(),
                Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
            }
        },
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn uninstall_agent_skill(
    Path((kind_str, skill_name)): Path<(String, String)>,
) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid agent kind: {}", e) }))).into_response(),
    };

    let req = agent::skills_sync::UninstallSkillRequest { skill_name };

    match agent::skills_sync::uninstall_skill(kind, req).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn list_agent_models(Path(kind_str): Path<String>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid agent kind: {}", e) }))).into_response(),
    };

    match agent::list_models(kind, None).await {
        Ok(models) => Json(json!({ "models": models })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to list models: {}", e) }))).into_response(),
    }
}


