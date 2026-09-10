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

pub(crate) struct DevelopmentRunPaths {
    project: std::path::PathBuf,
    run_dir: std::path::PathBuf,
    baseline: crate::services::change_set::BaselineManifest,
}

pub(crate) fn development_run_paths(
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

pub(crate) async fn get_task_diff(
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
pub(crate) struct VerificationCommand {
    label: String,
    program: String,
    args: Vec<String>,
}

pub(crate) fn verification_plan(run_dir: &std::path::Path) -> Vec<VerificationCommand> {
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

pub(crate) async fn verify_task_changes(
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

pub(crate) async fn apply_task_changes(
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
