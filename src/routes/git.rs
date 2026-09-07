use crate::models::task::TaskStatus;
use crate::services::destructive_ops::DestructiveOps;
use crate::services::git_service::{self, RepoKind};
use crate::services::workspace_guard::WorkspaceGuard;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct GitQuery {
    project: Option<String>,
    path: Option<String>,
    /// Legacy frontend alias for `path`.
    #[serde(rename = "filePath")]
    file_path: Option<String>,
    #[serde(rename = "baseHash")]
    base_hash: Option<String>,
    #[serde(rename = "targetHash")]
    target_hash: Option<String>,
    repo: Option<String>,
    limit: Option<usize>,
}

impl GitQuery {
    fn effective_path(&self) -> Option<String> {
        self.path.clone().or_else(|| self.file_path.clone())
    }
}

#[derive(Deserialize)]
pub struct GitRestoreRequest {
    project: Option<String>,
    path: Option<String>,
    #[serde(rename = "filePath")]
    file_path: Option<String>,
    hash: Option<String>,
    repo: Option<String>,
}

#[derive(Deserialize)]
pub struct GitCloneRequest {
    url: Option<String>,
    target: Option<String>,
}

#[derive(Deserialize)]
pub struct ApplyPatchRequest {
    project: Option<String>,
    #[serde(rename = "patchContent")]
    patch_content: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/git", get(git_status))
        .route("/api/git-diff", get(git_diff))
        .route("/api/git-restore", post(git_restore))
        .route("/api/file-git-history", get(file_git_history))
        .route("/api/shadow-git", get(shadow_git))
        .route("/api/git/shadow-snapshot", post(shadow_snapshot))
        .route("/api/git/shadow-history", get(shadow_history))
        .route("/api/git/shadow-rollback", post(shadow_rollback))
        .route("/api/apply-patch", post(apply_patch))
        .route("/api/projects/git-clone", post(git_clone))
}

async fn git_status(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<GitQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    match git_service::get_status(&project) {
        Ok(status) => Json(json!(status)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn git_diff(State(_state): State<Arc<AppState>>, Query(params): Query<GitQuery>) -> Response {
    let project = match params.project.clone() {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let file_path = match params.effective_path() {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing path param" })),
            )
                .into_response()
        }
    };
    let kind = RepoKind::parse(params.repo.as_deref());

    match git_service::get_file_diff(
        &project,
        &file_path,
        params.base_hash.as_deref(),
        params.target_hash.as_deref(),
        kind,
    ) {
        Ok(diff) => Json(json!(diff)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn git_restore(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GitRestoreRequest>,
) -> Response {
    let project = match body.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project" })),
            )
                .into_response()
        }
    };
    let file_path = match body.path.or(body.file_path) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing path" })),
            )
                .into_response()
        }
    };
    let kind = RepoKind::parse(body.repo.as_deref());

    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let guard = match WorkspaceGuard::new(&safe_project) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let ops = DestructiveOps::new(&guard);

    match ops.git_restore(&file_path, body.hash.as_deref(), kind) {
        Ok(()) => Json(json!({ "success": true, "path": file_path })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn file_git_history(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<GitQuery>,
) -> Response {
    let project = match params.project.clone() {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let file_path = match params.effective_path() {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing path param" })),
            )
                .into_response()
        }
    };
    let limit = params.limit.unwrap_or(20);

    let local = match git_service::get_file_history(&project, &file_path, limit, RepoKind::Local) {
        Ok(commits) => commits,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e })),
            )
                .into_response()
        }
    };
    // Shadow history is best-effort: missing/empty shadow repo just yields [].
    let shadow = git_service::get_file_history(&project, &file_path, limit, RepoKind::Shadow)
        .unwrap_or_default();

    Json(json!({ "local": local, "shadow": shadow, "path": file_path })).into_response()
}

async fn shadow_git(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<GitQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    match git_service::shadow_changed_files(&project) {
        Ok(files) => Json(json!({ "files": files, "project": project })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct ShadowSnapshotRequest {
    project: Option<String>,
    message: Option<String>,
}

async fn shadow_snapshot(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ShadowSnapshotRequest>,
) -> Response {
    let project = match body.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let message = body
        .message
        .unwrap_or_else(|| "AI Edit Snapshot".to_string());

    match git_service::record_ai_snapshot(&project, &message) {
        Ok(hash) => Json(json!({ "success": true, "hash": hash })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn shadow_history(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<GitQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let limit = params.limit.unwrap_or(50);

    match git_service::get_ai_history(&project, limit) {
        Ok(history) => Json(json!({ "history": history })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct ShadowRollbackRequest {
    project: Option<String>,
    hash: Option<String>,
}

async fn shadow_rollback(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ShadowRollbackRequest>,
) -> Response {
    let project = match body.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let hash = match body.hash {
        Some(h) => h,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing hash param" })),
            )
                .into_response()
        }
    };

    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let guard = match WorkspaceGuard::new(&safe_project) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let ops = DestructiveOps::new(&guard);

    match ops.shadow_rollback(&hash) {
        Ok(_) => Json(json!({ "success": true, "hash": hash })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn apply_patch(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ApplyPatchRequest>,
) -> Response {
    let project = match body.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project" })),
            )
                .into_response()
        }
    };
    let patch = match body.patch_content {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing patchContent" })),
            )
                .into_response()
        }
    };

    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let guard = match WorkspaceGuard::new(&safe_project) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let ops = DestructiveOps::new(&guard);

    match ops.apply_patch(&patch) {
        Ok(()) => Json(json!({ "success": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn git_clone(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GitCloneRequest>,
) -> Response {
    let url = match body.url {
        Some(u) => u,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing url" })),
            )
                .into_response()
        }
    };

    let target = body.target.unwrap_or_else(|| {
        let name = url
            .split('/')
            .next_back()
            .unwrap_or("repo")
            .trim_end_matches(".git");
        state
            .runtime_config
            .projects_root
            .join(name)
            .to_string_lossy()
            .to_string()
    });

    // Clone in a background task so large repos don't block the request.
    let tm = state.clone();
    let (task, _) =
        state
            .task_manager
            .create_task("git-clone", &target, Some(json!({ "url": url })));
    let task_id = task.id.clone();
    let target_owned = target.clone();
    let url_owned = url.clone();

    tokio::task::spawn_blocking(move || {
        tm.task_manager
            .update_status(&task_id, TaskStatus::Running, None);
        tm.task_manager.append_log(
            &task_id,
            &format!("[Git] Cloning {} into {}", url_owned, target_owned),
        );
        match git_service::clone_repo(&url_owned, &target_owned) {
            Ok(path) => {
                tm.task_manager
                    .append_log(&task_id, &format!("[Git] Clone completed: {}", path));
                tm.task_manager
                    .update_status(&task_id, TaskStatus::Completed, None);
            }
            Err(e) => {
                tm.task_manager
                    .append_log(&task_id, &format!("[Git] Clone failed: {}", e));
                tm.task_manager
                    .update_status(&task_id, TaskStatus::Failed, Some(e));
            }
        }
    });

    Json(json!({ "success": true, "taskId": task.id, "path": target, "status": "pending" }))
        .into_response()
}
