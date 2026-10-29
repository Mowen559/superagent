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


use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractBody {
    pub request_id: Option<String>,
    pub response: String,
    pub action: Option<String>,
}

/// Web Interaction Endpoint: Submit answers/approvals/steering to a waiting task
pub(crate) async fn interact_with_task(
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
            crate::domains::project_management::task_manager::TaskControl::UserResponse {
                request_id,
                response: body.response,
            }
        }
        "steer" => crate::domains::project_management::task_manager::TaskControl::Intervene {
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
pub(crate) fn to_sse_event(message: AgentMessage) -> Result<Event, Infallible> {
    let name = match &message {
        AgentMessage::Text { .. } => "text",
        AgentMessage::Thinking { .. } => "thinking",
        AgentMessage::ToolUse { .. } => "tool-use",
        AgentMessage::ToolResult { .. } => "tool-result",
        AgentMessage::Status { .. } => "status",
        AgentMessage::Error { .. } => "error",
        AgentMessage::Log { .. } => "log",
        AgentMessage::InteractionRequest { .. } => "interaction-request",
        AgentMessage::ContextUsage { .. } => "context-usage",
        AgentMessage::TraceSnapshot { .. } => "trace-snapshot",
    };
    let data = serde_json::to_string(&message).unwrap_or_else(|_| "{}".to_string());
    Ok(Event::default().event(name).data(data))
}
