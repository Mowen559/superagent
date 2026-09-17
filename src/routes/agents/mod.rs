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


pub mod run;
pub use run::*;
pub mod interact;
pub use interact::*;
pub mod config;
pub use config::*;
pub mod skills;
pub use skills::*;
pub mod models;
pub use models::*;
pub mod diff;
pub use diff::*;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/agents", get(list_agents))
        .route("/api/agents/run", post(run_agent))
        .route("/api/agents/tasks/{task_id}/events", get(stream_events))
        .route(
            "/api/agents/tasks/{task_id}/events/list",
            get(crate::routes::tasks::get_task_events_list),
        )
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
            "/api/agents/tasks/{task_id}/review",
            get(review_task_changes),
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
