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

pub(crate) async fn list_agent_skills(Path(kind_str): Path<String>) -> Response {
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

pub(crate) async fn list_agent_mcps(Path(kind_str): Path<String>) -> Response {
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

pub(crate) async fn install_agent_skill(
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

pub(crate) async fn update_agent_skill(
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

pub(crate) async fn uninstall_agent_skill(Path((kind_str, skill_name)): Path<(String, String)>) -> Response {
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

