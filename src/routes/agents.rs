//! Local coding-agent endpoints (`/api/agents/*`).
//!
//! Surfaces the agent adapter layer (`services::agent`) over HTTP:
//! - `GET  /api/agents`                     — detect every known CLI on PATH.
//! - `POST /api/agents/run`                 — launch a backend as a background
//!   task; structured events land in `{task_id}.events.jsonl` and fan out over
//!   the TaskManager event bus.
//! - `GET  /api/agents/tasks/:id/events`    — SSE stream: replay persisted
//!   events, then follow the live broadcast until the run finishes.

use std::collections::VecDeque;
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

use crate::models::task::TaskStatus;
use crate::services::agent::{
    self, types::SyncConfigPayload, AgentKind, AgentMessage, ExecOptions,
};
use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/agents", get(list_agents))
        .route("/api/agents/run", post(run_agent))
        .route("/api/agents/tasks/{task_id}/events", get(stream_events))
        .route(
            "/api/agents/tasks/{task_id}/interact",
            post(interact_with_task),
        )
        .route("/api/agents/tasks/{task_id}/diff", get(get_task_diff))
        .route(
            "/api/agents/tasks/{task_id}/verify",
            post(verify_task_changes),
        )
        .route(
            "/api/agents/tasks/{task_id}/apply",
            post(apply_task_changes),
        )
        .route("/api/agents/{kind}/config/sync", post(sync_agent_config))
        .route(
            "/api/agents/{kind}/config/preview",
            get(preview_agent_config),
        )
        .route("/api/agents/{kind}/skills", get(list_agent_skills))
        .route(
            "/api/agents/{kind}/skills/install",
            post(install_agent_skill),
        )
        .route(
            "/api/agents/{kind}/skills/{skill_name}/update",
            post(update_agent_skill),
        )
        .route(
            "/api/agents/{kind}/skills/{skill_name}/uninstall",
            post(uninstall_agent_skill),
        )
        .route("/api/agents/{kind}/mcps", get(list_agent_mcps))
        .route("/api/agents/{kind}/models", get(list_agent_models))
        .route("/api/agents/install_missing", post(install_missing_agents))
        .route("/api/agents/{kind}/install", post(install_agent_endpoint))
}

/// Probe every known agent CLI and report install/version/support status.
async fn list_agents() -> Json<serde_json::Value> {
    let agents = agent::detect_all().await;
    Json(json!({ "agents": agents }))
}

/// Install missing agents to the local agents directory.
async fn install_missing_agents() -> Json<serde_json::Value> {
    let mut results = vec![];
    let detections = agent::detect_all().await;

    for det in detections {
        if !det.installed && det.supported {
            if let Ok(kind) = AgentKind::from_str(det.agent) {
                let info = kind.info();
                if matches!(
                    info.install_strategy,
                    agent::registry::InstallStrategy::Unsupported
                ) {
                    continue;
                }

                let result = agent::installer::install_agent(kind);
                results.push(json!({
                    "agent": det.agent,
                    "success": result.is_ok(),
                    "error": result.err().map(|e| e.to_string())
                }));
            }
        }
    }

    Json(json!({ "results": results }))
}

/// Install or update a specific agent.
async fn install_agent_endpoint(Path(kind_str): Path<String>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
    };

    let result = tokio::task::spawn_blocking(move || agent::installer::install_agent(kind))
        .await
        .unwrap_or_else(|e| Err(agent::installer::InstallError::ProcessFailed(e.to_string())));

    match result {
        Ok(_) => Json(json!({ "success": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
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
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

const DEFAULT_AGENT_TIMEOUT_SECONDS: u64 = 30 * 60;
const MAX_AGENT_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

fn resolve_agent_timeout(requested: Option<u64>) -> Result<Duration, String> {
    let seconds = requested.unwrap_or(DEFAULT_AGENT_TIMEOUT_SECONDS);
    if seconds == 0 || seconds > MAX_AGENT_TIMEOUT_SECONDS {
        return Err(format!(
            "timeoutSeconds must be between 1 and {}",
            MAX_AGENT_TIMEOUT_SECONDS
        ));
    }
    Ok(Duration::from_secs(seconds))
}

/// Launch an agent run as a background task. Returns `{ taskId }` immediately;
/// the caller streams progress via `/api/agents/tasks/:id/events`.
async fn run_agent(State(state): State<Arc<AppState>>, Json(body): Json<RunBody>) -> Response {
    if body.prompt.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Missing prompt" })),
        )
            .into_response();
    }
    if body.agent != "superagent" {
        if let Err(e) = AgentKind::from_str(&body.agent) {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
    }
    let timeout = match resolve_agent_timeout(body.timeout_seconds) {
        Ok(timeout) => timeout,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": error, "kind": "invalid_timeout" })),
            )
                .into_response()
        }
    };

    let project = body.project.clone().unwrap_or_default();
    let project = if project.is_empty() {
        "".to_string()
    } else {
        match state.workspace_capabilities.resolve(&project) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": e, "kind": "workspace_not_open" })),
                )
                    .into_response()
            }
        }
    };

    let (task, cancel_rx) = state.task_manager.create_task(
        "agent",
        &project,
        Some(json!({
            "agent": body.agent,
            "timeoutSeconds": timeout.as_secs(),
        })),
    );
    let task_id = task.id.clone();

    // Create an immutable baseline manifest and an isolated working copy.
    let isolated_workspace = if !project.is_empty() {
        // The run directory must never be nested under the selected project:
        // selecting the managed projects root itself would otherwise make the
        // copy operation recursively copy its own destination.
        let run_root = state.app_paths.ensure_data_dir(&["runs", &task_id]);
        let run_dir = run_root.join("workspace");
        let baseline_path = run_root.join("baseline.json");
        let _ = crate::services::git_service::record_ai_snapshot(
            &project,
            "Creating Baseline Snapshot",
        );
        let prepared = crate::services::change_set::capture_baseline(
            std::path::Path::new(&project),
            &baseline_path,
        )
        .and_then(|_| {
            crate::services::change_set::copy_workspace(std::path::Path::new(&project), &run_dir)
        });
        if let Err(error) = prepared {
            state.task_manager.update_status(
                &task_id,
                TaskStatus::Failed,
                Some(format!("Failed to prepare isolated workspace: {error}")),
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error, "taskId": task_id })),
            )
                .into_response();
        }
        state
            .task_manager
            .update_workspace_baseline(&task_id, baseline_path.to_string_lossy().to_string());
        run_dir.to_string_lossy().to_string()
    } else {
        "".to_string()
    };

    state
        .task_manager
        .update_status(&task_id, TaskStatus::Running, None);

    let cwd_path = if !isolated_workspace.is_empty() {
        isolated_workspace.clone()
    } else {
        project.clone()
    };

    let opts = ExecOptions {
        cwd: Some(cwd_path.clone()).filter(|p| !p.is_empty()),
        model: body.model.clone().filter(|m| !m.is_empty()),
        resume_session_id: body.resume_session_id.clone().filter(|s| !s.is_empty()),
        thinking_level: body.thinking_level.clone().filter(|t| !t.is_empty()),
        timeout: Some(timeout),
        ..Default::default()
    };

    let state_bg = state.clone();
    let task_bg = task_id.clone();
    let prompt = body.prompt.clone();
    let agent_name = body.agent.clone();

    tokio::spawn(async move {
        if let Ok(kind) = AgentKind::from_str(&agent_name) {
            drive_run(state_bg, task_bg, kind, prompt, opts, cancel_rx).await;
        } else {
            state_bg.task_manager.update_status(
                &task_bg,
                TaskStatus::Failed,
                Some(format!("Unknown agent kind: {}", agent_name)),
            );
        }
    });

    Json(json!({ "taskId": task_id, "status": "running", "isolatedWorkspace": isolated_workspace }))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_runs_have_a_finite_default_timeout() {
        assert_eq!(
            resolve_agent_timeout(None).unwrap(),
            Duration::from_secs(DEFAULT_AGENT_TIMEOUT_SECONDS)
        );
    }

    #[test]
    fn timeout_override_is_bounded() {
        assert_eq!(
            resolve_agent_timeout(Some(45)).unwrap(),
            Duration::from_secs(45)
        );
        assert!(resolve_agent_timeout(Some(0)).is_err());
        assert!(resolve_agent_timeout(Some(MAX_AGENT_TIMEOUT_SECONDS + 1)).is_err());
    }
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
    let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
    state.task_manager.register_control(&task_id, control_tx);

    let backend = agent::new(kind);
    let mut session = match backend
        .execute(prompt, opts, cancel_rx, Some(control_rx))
        .await
    {
        Ok(s) => s,
        Err(e) => {
            let msg = e.to_string();
            state.task_manager.append_event(
                &task_id,
                &AgentMessage::Error {
                    content: msg.clone(),
                },
            );
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
            let status = TaskStatus::from_agent_status(&result.status);
            let error = if result.error.is_empty() {
                None
            } else {
                Some(result.error.clone())
            };
            if status == TaskStatus::Completed {
                let output = result.output.chars().take(120_000).collect::<String>();
                let receipt = json!({
                    "schemaVersion": 1,
                    "kind": "agent-run",
                    "status": result.status,
                    "output": output,
                    "outputTruncated": result.output.chars().count() > 120_000,
                    "durationMs": result.duration_ms,
                    "sessionId": result.session_id,
                    "usage": result.usage,
                });
                state.task_manager.complete_with_receipt(&task_id, receipt);
            } else {
                state.task_manager.update_status(&task_id, status, error);
            }
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

use axum::http::HeaderMap;

/// SSE endpoint: replay the persisted backlog, then follow the live broadcast.
/// A terminal event (or the channel closing) ends the stream.
pub(crate) async fn stream_events(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let (backlog, receiver_opt) = state
        .task_manager
        .snapshot_and_subscribe(&task_id, last_event_id);
    let last_backlog_id = backlog.last().map(|(id, _)| *id).unwrap_or(0);
    let backlog_stream = stream::iter(
        backlog
            .into_iter()
            .map(|(id, msg)| to_sse_event(msg).map(|e| e.id(id.to_string()))),
    );

    let boxed_stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        if let Some(receiver) = receiver_opt {
            let live_state = (
                receiver,
                state.clone(),
                task_id.clone(),
                last_event_id.max(last_backlog_id),
                VecDeque::<(usize, AgentMessage)>::new(),
            );
            let live_stream = stream::unfold(
                live_state,
                |(mut receiver, state, task_id, mut last_seen, mut pending)| async move {
                    loop {
                        if let Some((sequence, message)) = pending.pop_front() {
                            if sequence <= last_seen {
                                continue;
                            }
                            last_seen = sequence;
                            let event =
                                to_sse_event(message).map(|event| event.id(sequence.to_string()));
                            return Some((event, (receiver, state, task_id, last_seen, pending)));
                        }
                        match receiver.recv().await {
                            Ok((sequence, message)) if sequence > last_seen => {
                                last_seen = sequence;
                                let event = to_sse_event(message)
                                    .map(|event| event.id(sequence.to_string()));
                                return Some((
                                    event,
                                    (receiver, state, task_id, last_seen, pending),
                                ));
                            }
                            Ok(_) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                pending
                                    .extend(state.task_manager.events_after(&task_id, last_seen));
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                },
            );
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractBody {
    pub request_id: Option<String>,
    pub response: String,
    pub action: Option<String>,
}

/// Web Interaction Endpoint: Submit answers/approvals/steering to a waiting task
async fn interact_with_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    Json(body): Json<InteractBody>,
) -> Response {
    // Look up the task to ensure it's still running
    let task = state.task_manager.get_task(&task_id);
    if task.map_or(true, |t| t.status.is_terminal()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Task is not running or waiting for interaction." })),
        )
            .into_response();
    }

    let control = match body.action.as_deref().unwrap_or("respond") {
        "respond" => {
            let Some(request_id) = body.request_id.filter(|value| !value.trim().is_empty()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "requestId is required for respond" })),
                )
                    .into_response();
            };
            crate::services::task_manager::TaskControl::UserResponse {
                request_id,
                response: body.response,
            }
        }
        "steer" => crate::services::task_manager::TaskControl::Intervene {
            instruction: body.response,
        },
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "action must be respond or steer" })),
            )
                .into_response()
        }
    };
    if state.task_manager.send_control(&task_id, control) {
        Json(json!({ "success": true, "message": "Interaction delivered." })).into_response()
    } else {
        (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Task has no active interactive Adapter" })),
        )
            .into_response()
    }
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
        AgentMessage::InteractionRequest { .. } => "interaction-request",
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
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
    };

    let backend = agent::new(kind);
    match backend.sync_config(&payload, None).await {
        Ok(_) => Json(json!({ "success": true })).into_response(),
        Err(e) => {
            let err_msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": err_msg })),
            )
                .into_response()
        }
    }
}

/// Preview the local configuration of a specific agent.
async fn preview_agent_config(Path(kind_str): Path<String>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
    };

    let backend = agent::new(kind);
    match backend.get_config_content(None).await {
        Ok(Some(content)) => Json(json!({ "content": content })).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Configuration file not found" })),
        )
            .into_response(),
        Err(e) => {
            let err_msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": err_msg })),
            )
                .into_response()
        }
    }
}

async fn list_agent_skills(Path(kind_str): Path<String>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
    };

    let spec = agent::discovery::get_agent_discovery_spec(kind);
    let mut cache = agent::skills::ScannerCache::new();
    let skills =
        agent::skills::scan_skills_for_agent(kind.info().slug, &spec.skill_paths, &mut cache);

    Json(json!({ "skills": skills })).into_response()
}

async fn list_agent_mcps(Path(kind_str): Path<String>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
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
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
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
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
    };

    let spec = agent::discovery::get_agent_discovery_spec(kind);
    let mut cache = agent::skills::ScannerCache::new();
    let detected_skills =
        agent::skills::scan_skills_for_agent(kind.info().slug, &spec.skill_paths, &mut cache);

    let detected = match detected_skills.into_iter().find(|s| {
        s.name.eq_ignore_ascii_case(&skill_name) || s.directory.eq_ignore_ascii_case(&skill_name)
    }) {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Skill '{}' not found", skill_name) })),
            )
                .into_response()
        }
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

            match agent::skills_sync::install_or_update_skill_for_agent(kind, install_req, true)
                .await
            {
                Ok(res) => Json(res).into_response(),
                Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
            }
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn uninstall_agent_skill(Path((kind_str, skill_name)): Path<(String, String)>) -> Response {
    let kind = match AgentKind::from_str(&kind_str) {
        Ok(k) => k,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
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
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid agent kind: {}", e) })),
            )
                .into_response()
        }
    };

    match agent::list_models(kind, None).await {
        Ok(models) => Json(json!({ "models": models })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to list models: {}", e) })),
        )
            .into_response(),
    }
}

const MAX_DIFF_CHARS: usize = 1_000_000;
const VERIFICATION_TIMEOUT_SECONDS: u64 = 5 * 60;

struct DevelopmentRunPaths {
    project: std::path::PathBuf,
    run_dir: std::path::PathBuf,
    baseline: crate::services::change_set::BaselineManifest,
}

fn development_run_paths(
    state: &Arc<AppState>,
    task: &crate::models::task::Task,
) -> Result<DevelopmentRunPaths, String> {
    if task.task_type != "agent" {
        return Err("Task is not an Agent development run".to_string());
    }
    if task.status != TaskStatus::Completed {
        return Err(format!(
            "Agent task must be completed before reviewing changes; current status is {}",
            task.status.as_str()
        ));
    }
    if task.project_path.is_empty() {
        return Err("Agent task has no workspace".to_string());
    }
    let project = state.workspace_capabilities.resolve(&task.project_path)?;
    let baseline_path = task
        .workspace_baseline
        .as_deref()
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| {
            state
                .app_paths
                .data_path(&["runs", &task.id, "baseline.json"])
        });
    let run_dir = baseline_path
        .parent()
        .map(|parent| parent.join("workspace"))
        .filter(|path| path.is_dir())
        // Compatibility with development runs created by the previous layout.
        .unwrap_or_else(|| {
            state
                .runtime_config
                .projects_root
                .join(format!(".deepcloud/runs/{}", task.id))
        });
    if !run_dir.is_dir() {
        return Err("Isolated Agent workspace is unavailable".to_string());
    }
    let baseline = crate::services::change_set::load_baseline(&baseline_path)?;
    Ok(DevelopmentRunPaths {
        project,
        run_dir,
        baseline,
    })
}

async fn get_task_diff(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.task_manager.get_task(&task_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    };
    let paths = match development_run_paths(&state, &task) {
        Ok(paths) => paths,
        Err(error) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": error, "kind": "development_run_not_ready" })),
            )
                .into_response()
        }
    };
    let change_set = match crate::services::change_set::inspect_changes(
        &paths.baseline,
        &paths.project,
        &paths.run_dir,
    ) {
        Ok(change_set) => change_set,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error })),
            )
                .into_response()
        }
    };
    state.task_manager.update_development_run(
        &task_id,
        Some(json!(change_set.clone())),
        Some("pending".to_string()),
    );

    let output = tokio::process::Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--")
        .arg(&paths.project)
        .arg(&paths.run_dir)
        .output()
        .await;
    let diff = output
        .as_ref()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let truncated = diff.chars().count() > MAX_DIFF_CHARS;
    let diff = diff.chars().take(MAX_DIFF_CHARS).collect::<String>();
    Json(json!({
        "diff": diff,
        "diffTruncated": truncated,
        "files": change_set.files,
        "conflicts": change_set.conflicts,
    }))
    .into_response()
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyBody {
    pub files: Option<Vec<String>>,
    pub force: Option<bool>,
}

#[derive(Clone)]
struct VerificationCommand {
    label: String,
    program: String,
    args: Vec<String>,
}

fn verification_plan(run_dir: &std::path::Path) -> Vec<VerificationCommand> {
    let mut plan = Vec::new();
    if run_dir.join("Cargo.toml").is_file() {
        plan.push(VerificationCommand {
            label: "Rust cargo check".to_string(),
            program: "cargo".to_string(),
            args: vec!["check".to_string(), "--quiet".to_string()],
        });
    }
    let package_json = run_dir.join("package.json");
    if let Ok(payload) = std::fs::read_to_string(package_json) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) {
            if let Some(scripts) = value.get("scripts").and_then(|value| value.as_object()) {
                for name in ["typecheck", "lint", "test", "build"] {
                    if scripts.contains_key(name) && plan.len() < 3 {
                        plan.push(VerificationCommand {
                            label: format!("Node.js npm run {name}"),
                            program: if cfg!(windows) { "npm.cmd" } else { "npm" }.to_string(),
                            args: vec!["run".to_string(), name.to_string()],
                        });
                    }
                }
            }
        }
    }
    if run_dir.join("pom.xml").is_file() && plan.len() < 3 {
        plan.push(VerificationCommand {
            label: "Maven compile".to_string(),
            program: if cfg!(windows) { "mvn.cmd" } else { "mvn" }.to_string(),
            args: vec![
                "-q".to_string(),
                "-DskipTests".to_string(),
                "compile".to_string(),
            ],
        });
    }
    if (run_dir.join("pyproject.toml").is_file() || run_dir.join("requirements.txt").is_file())
        && plan.len() < 3
    {
        plan.push(VerificationCommand {
            label: "Python compileall".to_string(),
            program: "python".to_string(),
            args: vec![
                "-m".to_string(),
                "compileall".to_string(),
                "-q".to_string(),
                ".".to_string(),
            ],
        });
    }
    plan
}

async fn verify_task_changes(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.task_manager.get_task(&task_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    };
    let paths = match development_run_paths(&state, &task) {
        Ok(paths) => paths,
        Err(error) => {
            return (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response()
        }
    };
    let fingerprint = match crate::services::change_set::workspace_fingerprint(&paths.run_dir) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error })),
            )
                .into_response()
        }
    };
    let plan = verification_plan(&paths.run_dir);
    let mut results = Vec::new();
    let mut passed = !plan.is_empty();
    for spec in plan {
        let started = std::time::Instant::now();
        let mut command = tokio::process::Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&paths.run_dir)
            .kill_on_drop(true);
        let outcome = tokio::time::timeout(
            Duration::from_secs(VERIFICATION_TIMEOUT_SECONDS),
            command.output(),
        )
        .await;
        let result = match outcome {
            Ok(Ok(output)) => {
                let success = output.status.success();
                passed &= success;
                json!({
                    "label": spec.label,
                    "program": spec.program,
                    "args": spec.args,
                    "status": if success { "passed" } else { "failed" },
                    "exitCode": output.status.code(),
                    "stdout": String::from_utf8_lossy(&output.stdout).chars().take(20_000).collect::<String>(),
                    "stderr": String::from_utf8_lossy(&output.stderr).chars().take(20_000).collect::<String>(),
                    "durationMs": started.elapsed().as_millis(),
                })
            }
            Ok(Err(error)) => {
                passed = false;
                json!({ "label": spec.label, "program": spec.program, "args": spec.args, "status": "failed", "error": error.to_string(), "durationMs": started.elapsed().as_millis() })
            }
            Err(_) => {
                passed = false;
                json!({ "label": spec.label, "program": spec.program, "args": spec.args, "status": "timeout", "durationMs": started.elapsed().as_millis() })
            }
        };
        results.push(result);
    }
    let status = if results.is_empty() {
        "skipped"
    } else if passed {
        "passed"
    } else {
        "failed"
    };
    let receipt = json!({
        "schemaVersion": 1,
        "kind": "verification",
        "status": status,
        "sourceFingerprint": fingerprint,
        "completedAt": chrono::Utc::now().to_rfc3339(),
        "commands": results,
    });
    state
        .task_manager
        .update_verification(&task_id, receipt.clone());
    Json(json!({ "success": status == "passed", "verification": receipt })).into_response()
}

async fn apply_task_changes(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    payload: Option<Json<ApplyBody>>,
) -> Response {
    let Some(task) = state.task_manager.get_task(&task_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    };
    if task.approval.as_deref() == Some("approved") {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Change set has already been applied", "kind": "already_applied" })),
        )
            .into_response();
    }
    let paths = match development_run_paths(&state, &task) {
        Ok(paths) => paths,
        Err(error) => {
            return (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response()
        }
    };
    let body = payload.map(|Json(body)| body).unwrap_or_default();
    let forced = body.force.unwrap_or(false);
    let proposed_fingerprint =
        match crate::services::change_set::workspace_fingerprint(&paths.run_dir) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response()
            }
        };
    let verification_passed = task.verification.as_ref().is_some_and(|receipt| {
        receipt.get("status").and_then(|value| value.as_str()) == Some("passed")
            && receipt
                .get("sourceFingerprint")
                .and_then(|value| value.as_str())
                == Some(proposed_fingerprint.as_str())
    });
    if !verification_passed && !forced {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "A passing, current Verification Receipt is required before Apply",
                "kind": "verification_required",
            })),
        )
            .into_response();
    }
    let change_set = match crate::services::change_set::inspect_changes(
        &paths.baseline,
        &paths.project,
        &paths.run_dir,
    ) {
        Ok(change_set) => change_set,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error })),
            )
                .into_response()
        }
    };
    if change_set.files.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Agent task produced no file changes", "kind": "empty_change_set" })),
        )
            .into_response();
    }
    if !change_set.conflicts.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Original workspace has conflicting changes", "kind": "workspace_conflict", "conflicts": change_set.conflicts })),
        )
            .into_response();
    }
    let snapshot_hash =
        match crate::services::git_service::record_ai_snapshot(
            &task.project_path,
            &format!("Before applying AI task {task_id}"),
        ) {
            Ok(hash) => hash,
            Err(error) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    json!({ "error": format!("Cannot create pre-apply shadow snapshot: {error}") }),
                ),
            )
                .into_response(),
        };
    let apply_receipt = match crate::services::change_set::apply_changes(
        &change_set,
        &paths.project,
        &paths.run_dir,
        body.files.as_deref(),
        forced,
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": error, "kind": "apply_rejected" })),
            )
                .into_response()
        }
    };
    state.task_manager.update_development_run(
        &task_id,
        Some(json!(change_set)),
        Some("approved".to_string()),
    );

    let applied_count = apply_receipt.applied_files.len() + apply_receipt.deleted_files.len();
    let summary =
        format!("AI Changes Applied: {task_id}. Files changed: {applied_count}. Forced: {forced}");

    // Save to supermemory
    let store = crate::services::supermemory_store::SupermemoryStore::new(
        &state.runtime_config,
        Some(&task.project_path),
    );
    let now = chrono::Utc::now().to_rfc3339();
    let record = crate::models::memory::SupermemoryRecord {
        id: uuid::Uuid::new_v4().to_string(),
        file_path: "".to_string(),
        name: format!("AI Application Task: {}", task_id),
        summary: serde_json::to_string(&json!({
            "summary": summary,
            "snapshotHash": snapshot_hash,
            "apply": apply_receipt,
        }))
        .unwrap_or(summary),
        tags: vec!["ai_applied_change".to_string(), task_id.clone()],
        complexity: "low".to_string(),
        node_type: "ai_applied_change".to_string(),
        source_graph_path: "".to_string(),
        source_label: "".to_string(),
        updated_at: now,
        embedding: None,
    };
    let memory_synced = store.write_memory_result(record).is_ok();

    Json(json!({
        "status": "success",
        "files_applied": applied_count,
        "snapshotHash": snapshot_hash,
        "memorySynced": memory_synced,
        "receipt": apply_receipt,
    }))
    .into_response()
}
