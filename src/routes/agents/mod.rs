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


pub mod config;
pub mod diff;
pub mod interact;
pub mod models;
pub mod run;
pub mod skills;

pub(crate) use run::stream_events;
use config::{
    install_agent_endpoint, install_missing_agents, list_agents, preview_agent_config,
    sync_agent_config,
};
use diff::{apply_task_changes, get_task_diff, review_task_changes, verify_task_changes};
use interact::interact_with_task;
use models::list_agent_models;
use run::run_agent;
use skills::{
    install_agent_skill, list_agent_mcps, list_agent_skills, uninstall_agent_skill,
    update_agent_skill,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Fix 3: Global Concurrency Limiting (Max 100 concurrent agent tasks/requests)
        .layer(tower::limit::ConcurrencyLimitLayer::new(100))
        .route("/api/agents", get(list_agents))
        .route(
            "/api/agents/persona",
            get(crate::routes::settings::get_persona).put(crate::routes::settings::put_persona),
        )
        .route(
            "/api/agents/persona/reset",
            post(crate::routes::settings::reset_persona),
        )
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
        .route("/api/agents/settings", get(settings_api::get_settings).post(settings_api::update_settings))
        .route("/api/agents/{kind}/install", post(install_agent_endpoint))
}


pub mod settings_api;