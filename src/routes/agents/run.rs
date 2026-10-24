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


use super::interact::to_sse_event;

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
    /// ZCode-style collaboration mode for the superagent worker:
    /// plan | build | edit | yolo | guarded (defaults to build).
    #[serde(default)]
    pub work_mode: Option<String>,
    /// Image attachments as data URLs (`data:image/png;base64,...`) or absolute
    /// local paths; data URLs are staged to the hub data dir before dispatch.
    #[serde(default)]
    pub images: Vec<String>,
}

pub(crate) const DEFAULT_AGENT_TIMEOUT_SECONDS: u64 = 30 * 60;
pub(crate) const MAX_AGENT_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
/// 单张图片解码后的体积上限（与 worker 侧 load_image_data_urls 对齐）。
const MAX_STAGED_IMAGE_BYTES: usize = 6 * 1024 * 1024;
/// 单次运行最多携带的图片数量（防滥用 + 控制上下文膨胀）。
const MAX_IMAGES_PER_RUN: usize = 6;

/// 将 RunBody.images 中的 data URL 落盘为暂存文件，返回本地路径列表。
/// 已是本地路径的条目原样透传；解码/写入失败的条目跳过（不阻断运行）。
pub(crate) fn stage_images(task_id: &str, images: &[String]) -> Result<Vec<String>, String> {
    let mut staged: Vec<String> = Vec::new();
    for (index, raw) in images.iter().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if staged.len() >= MAX_IMAGES_PER_RUN {
            break;
        }
        // 本地路径直接透传（CLI 场景）
        if !trimmed.starts_with("data:") {
            staged.push(trimmed.to_string());
            continue;
        }
        let Some((mime, payload)) = trimmed.split_once(",") else {
            return Err(format!("images[{index}] is not a valid data URL"));
        };
        let extension = mime
            .strip_prefix("data:image/")
            .and_then(|m| m.split(";").next())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let extension = match extension.as_str() {
            "jpg" | "jpeg" => "jpg",
            other => other,
        };
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .map_err(|e| format!("images[{index}] base64 decode failed: {e}"))?;
        if bytes.len() > MAX_STAGED_IMAGE_BYTES {
            return Err(format!(
                "images[{index}] exceeds {} byte limit",
                MAX_STAGED_IMAGE_BYTES
            ));
        }
        let dir = crate::config::app_paths::AppPaths::new()
            .ensure_data_dir(&["uploads", task_id]);
        let path = dir.join(format!("img_{index:02}.{extension}"));
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("images[{index}] staging failed: {e}"))?;
        staged.push(path.to_string_lossy().to_string());
    }
    Ok(staged)
}

pub(crate) fn resolve_agent_timeout(requested: Option<u64>) -> Result<Duration, String> {
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
pub(crate) async fn run_agent(State(state): State<Arc<AppState>>, Json(body): Json<RunBody>) -> Response {
    if state.task_manager.is_shutting_down() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Server is shutting down" })),
        )
            .into_response();
    }

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

    let work_mode = match body.work_mode.as_deref() {
        None | Some("") => None,
        Some(raw) => match crate::services::brain::tool_policy::WorkMode::parse(raw) {
            Some(mode) => Some(mode.as_str().to_string()),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": format!(
                            "workMode must be one of {}",
                            crate::services::brain::tool_policy::WorkMode::ALL.join(", ")
                        ),
                        "kind": "invalid_work_mode",
                    })),
                )
                    .into_response()
            }
        },
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
            "workMode": work_mode,
        })),
    );
    let task_id = task.id.clone();
    state
        .task_manager
        .update_status(&task_id, TaskStatus::Running, None);

    let state_bg = state.clone();
    let task_bg = task_id.clone();
    let prompt = body.prompt.clone();
    let agent_name = body.agent.clone();
    let project_bg = project.clone();
    let model_bg = body.model.clone().filter(|m| !m.is_empty());
    let resume_bg = body.resume_session_id.clone().filter(|s| !s.is_empty());
    let thinking_bg = body.thinking_level.clone().filter(|t| !t.is_empty());
    let policy_bg = work_mode.clone();
    let images_bg: Vec<String> = body
        .images
        .iter()
        .filter(|i| !i.trim().is_empty())
        .cloned()
        .collect();

    let handle = tokio::spawn(async move {
        // 图片附件暂存：data URL 落盘为本地路径（worker 再读回转 data URL）
        let staged_images = if images_bg.is_empty() {
            Vec::new()
        } else {
            match stage_images(&task_bg, &images_bg) {
                Ok(paths) => paths,
                Err(error) => {
                    state_bg.task_manager.update_status(
                        &task_bg,
                        TaskStatus::Failed,
                        Some(error.clone()),
                    );
                    return;
                }
            }
        };

        // 异步准备隔离工作区与 baseline，绝不阻塞 HTTP 入口响应
        let mut cwd_path = project_bg.clone();
        if !project_bg.is_empty() {
            let run_root = state_bg.app_paths.ensure_data_dir(&["runs", &task_bg]);
            let run_dir = run_root.join("workspace");
            let baseline_path = run_root.join("baseline.json");

            let _ = crate::domains::version_control::git_service::record_ai_snapshot(
                &project_bg,
                "Creating Baseline Snapshot",
            );

            match crate::domains::version_control::change_set::capture_baseline(
                std::path::Path::new(&project_bg),
                &baseline_path,
            )
            .and_then(|_| {
                crate::domains::version_control::change_set::copy_workspace(std::path::Path::new(&project_bg), &run_dir)
            }) {
                Ok(_) => {
                    state_bg
                        .task_manager
                        .update_workspace_baseline(&task_bg, baseline_path.to_string_lossy().to_string());
                    cwd_path = run_dir.to_string_lossy().to_string();
                }
                Err(error) => {
                    tracing::warn!(
                        "[run_agent] Failed to prepare isolated workspace ({error}), falling back to direct project: {project_bg}"
                    );
                }
            }
        }

        let opts = ExecOptions {
            cwd: Some(cwd_path.clone()).filter(|p| !p.is_empty()),
            model: model_bg,
            resume_session_id: resume_bg,
            thinking_level: thinking_bg,
            timeout: Some(timeout),
            tool_policy: policy_bg,
            image_paths: staged_images,
            ..Default::default()
        };

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
    state.task_manager.register_handle(&task_id, handle);

    Json(json!({ "taskId": task_id, "status": "running" }))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub(crate) fn agent_runs_have_a_finite_default_timeout() {
        assert_eq!(
            resolve_agent_timeout(None).unwrap(),
            Duration::from_secs(DEFAULT_AGENT_TIMEOUT_SECONDS)
        );
    }

    #[test]
    pub(crate) fn timeout_override_is_bounded() {
        assert_eq!(
            resolve_agent_timeout(Some(45)).unwrap(),
            Duration::from_secs(45)
        );
        assert!(resolve_agent_timeout(Some(0)).is_err());
        assert!(resolve_agent_timeout(Some(MAX_AGENT_TIMEOUT_SECONDS + 1)).is_err());
    }

    #[test]
    pub(crate) fn stages_data_urls_and_passes_through_paths() {
        // 1x1 透明 PNG
        let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        let data_url = format!("data:image/png;base64,{png_base64}");
        let staged = stage_images("test-task-stage", &[data_url, "C:\\abs\\photo.jpg".to_string()])
            .expect("staging should succeed");
        assert_eq!(staged.len(), 2);
        assert!(staged[0].ends_with("img_00.png"));
        assert!(std::path::Path::new(&staged[0]).exists());
        let bytes = std::fs::read(&staged[0]).unwrap();
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
        assert_eq!(staged[1], "C:\\abs\\photo.jpg");
        let _ = std::fs::remove_file(&staged[0]);
    }

    #[test]
    pub(crate) fn rejects_invalid_base64() {
        let error = stage_images("test-task-bad", &["data:image/png;base64,###".to_string()])
            .expect_err("invalid base64 must fail");
        assert!(error.contains("base64 decode failed"));
    }

    #[test]
    pub(crate) fn caps_images_per_run() {
        let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        let images: Vec<String> = (0..10)
            .map(|_| format!("data:image/png;base64,{png_base64}"))
            .collect();
        let staged = stage_images("test-task-cap", &images).unwrap();
        assert_eq!(staged.len(), MAX_IMAGES_PER_RUN);
        for path in staged.iter().filter(|p| p.contains("img_")) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Execute a backend and pump its event stream into the task's event store,
/// then record the final outcome as the task status. Runs on a background task.
pub(crate) async fn drive_run(
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
