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
use std::collections::VecDeque;
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
        .route("/api/agents/tasks/{task_id}/interact", post(interact_with_task))
        .route("/api/agents/tasks/{task_id}/diff", get(get_task_diff))
        .route("/api/agents/tasks/{task_id}/apply", post(apply_task_changes))
        .route("/api/agents/{kind}/config/sync", post(sync_agent_config))
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
    let guard = match crate::services::workspace_guard::WorkspaceGuard::new(
        &state.runtime_config.projects_root,
    ) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let project = if project.is_empty() {
        "".to_string()
    } else {
        match guard.resolve_safe(&project, "") {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": e.to_string() })),
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

    // Phase 2: Create a Workspace Baseline for Two-Phase Commit isolation.
    let workspace_baseline = if !project.is_empty() {
        let run_dir = state.runtime_config.projects_root.join(format!(".deepcloud/runs/{}", task_id));
        let _ = crate::services::git_service::record_ai_snapshot(&project, "Creating Baseline Snapshot");
        // Deep copy the project to isolate changes for Two-Phase commit diffing.
        // We use ignore::WalkBuilder to skip .gitignore files (e.g. node_modules, target)
        // and we manually skip the .deepcloud folder to prevent recursive copying.
        let walker = ignore::WalkBuilder::new(&project)
            .hidden(false) // we might want to copy some hidden files like .env, but usually ignore is enough
            .filter_entry(|e| {
                e.file_name() != ".deepcloud" && e.file_name() != ".git"
            })
            .build();

        for result in walker {
            if let Ok(entry) = result {
                if let Ok(rel_path) = entry.path().strip_prefix(&project) {
                    let dest_path = run_dir.join(rel_path);
                    if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                        let _ = std::fs::create_dir_all(&dest_path);
                    } else if entry.file_type().map_or(false, |ft| ft.is_file()) {
                        let _ = std::fs::copy(entry.path(), &dest_path);
                    }
                }
            }
        }

        run_dir.to_string_lossy().to_string()
    } else {
        "".to_string()
    };


    state
        .task_manager
        .update_status(&task_id, TaskStatus::Running, None);

    let cwd_path = if !workspace_baseline.is_empty() {
        workspace_baseline.clone()
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
            state_bg.task_manager.update_status(&task_bg, TaskStatus::Failed, Some(format!("Unknown agent kind: {}", agent_name)));
        }
    });

    Json(json!({ "taskId": task_id, "status": "running", "workspace_baseline": workspace_baseline })).into_response()
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
    let mut session = match backend.execute(prompt, opts, cancel_rx, Some(control_rx)).await {
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

use axum::http::HeaderMap;

/// SSE endpoint: replay the persisted backlog, then follow the live broadcast.
/// A terminal event (or the channel closing) ends the stream.
async fn stream_events(
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
        backlog.into_iter()
            .map(|(id, msg)| to_sse_event(msg).map(|e| e.id(id.to_string())))
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
                            let event = to_sse_event(message)
                                .map(|event| event.id(sequence.to_string()));
                            return Some((event, (receiver, state, task_id, last_seen, pending)));
                        }
                        match receiver.recv().await {
                            Ok((sequence, message)) if sequence > last_seen => {
                                last_seen = sequence;
                                let event = to_sse_event(message)
                                    .map(|event| event.id(sequence.to_string()));
                                return Some((event, (receiver, state, task_id, last_seen, pending)));
                            }
                            Ok(_) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                pending.extend(
                                    state.task_manager.events_after(&task_id, last_seen),
                                );
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
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Task is not running or waiting for interaction." }))).into_response();
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

/// Get the diff between the original project and the isolated workspace baseline (Two-Phase Commit).

fn parse_change_set_from_diff(diff_text: &str) -> serde_json::Value {
    let mut files = Vec::new();
    for line in diff_text.lines() {
        if line.starts_with("+++ b/") {
            let file = &line[6..];
            if file != "dev/null" && !files.contains(&file.to_string()) {
                files.push(file.to_string());
            }
        } else if line.starts_with("--- a/") {
            let file = &line[6..];
            // If the file is only deleted, it might only show --- a/ and +++ b/dev/null.
            // But we typically want to know if it's changed.
            if file != "dev/null" && !files.contains(&file.to_string()) {
                files.push(file.to_string());
            }
        }
    }
    serde_json::json!({ "files": files })
}

async fn get_task_diff(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let task = state.task_manager.get_task(&task_id);
    if task.is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Task not found" }))).into_response();
    }
    let task = task.unwrap();
    let project = task.project_path.clone();
    let run_dir = state.runtime_config.projects_root.join(format!(".deepcloud/runs/{}", task_id));

    if !project.is_empty() && run_dir.exists() {
        // Use `git diff --no-index` to compare the two directories.
        // This is safe even if they aren't git repositories.
        let output = tokio::process::Command::new("git")
            .arg("diff")
            .arg("--no-index")
            .arg(&project)
            .arg(&run_dir)
            .output()
            .await;

        if let Ok(out) = output {
            // git diff --no-index returns exit code 1 if differences were found
            let diff_text = String::from_utf8_lossy(&out.stdout).to_string();
            let change_set = parse_change_set_from_diff(&diff_text);
            state.task_manager.update_development_run(&task_id, Some(change_set), Some("pending".to_string()));
            return Json(json!({ "diff": diff_text })).into_response();
        }
    }

    Json(json!({ "diff": "" })).into_response()
}

/// Apply the isolated workspace baseline changes back to the original project.
#[derive(Deserialize)]
pub struct ApplyBody {
    pub files: Option<Vec<String>>,
}

async fn apply_task_changes(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    payload: Option<Json<ApplyBody>>,
) -> Response {
    let task = state.task_manager.get_task(&task_id);
    if task.is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Task not found" }))).into_response();
    }
    let task = task.unwrap();
    let project = task.project_path.clone();
    let run_dir = state.runtime_config.projects_root.join(format!(".deepcloud/runs/{}", task_id));

    if project.is_empty() || !run_dir.exists() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Workspace baseline not found or empty project." }))).into_response();
    }

    // We can use the same ignore::WalkBuilder trick to copy files back,
    // overwriting the original files.
    let walker = ignore::WalkBuilder::new(&run_dir)
        .hidden(false)
        .build();

    let mut applied_count = 0;
    for result in walker {
        if let Ok(entry) = result {
            if let Ok(rel_path) = entry.path().strip_prefix(&run_dir) {
                if rel_path.as_os_str().is_empty() {
                    continue;
                }
                let dest_path = std::path::Path::new(&project).join(rel_path);
                if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                    let _ = std::fs::create_dir_all(&dest_path);
                } else if entry.file_type().map_or(false, |ft| ft.is_file()) {
                    let _ = std::fs::copy(entry.path(), &dest_path);
                    applied_count += 1;
                }
            }
        }
    }

    // Optionally mark the task as approved
    let _ = crate::services::git_service::record_ai_snapshot(&project, &format!("AI Changes Applied: {}", task_id));

    Json(json!({ "status": "success", "files_applied": applied_count })).into_response()
}
