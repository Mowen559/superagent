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

/// Probe every known agent CLI and report install/version/support status.
pub(crate) async fn list_agents() -> Json<serde_json::Value> {
    let agents = agent::detect_all().await;
    Json(json!({ "agents": agents }))
}

/// Install missing agents to the local agents directory.
pub(crate) async fn install_missing_agents() -> Json<serde_json::Value> {
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
pub(crate) async fn install_agent_endpoint(Path(kind_str): Path<String>) -> Response {
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

/// Synchronize the configuration of a specific agent.
pub(crate) async fn sync_agent_config(
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
pub(crate) async fn preview_agent_config(Path(kind_str): Path<String>) -> Response {
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

