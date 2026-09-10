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

pub(crate) async fn list_agent_models(Path(kind_str): Path<String>) -> Response {
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

pub(crate) const MAX_DIFF_CHARS: usize = 1_000_000;
pub(crate) const VERIFICATION_TIMEOUT_SECONDS: u64 = 5 * 60;

