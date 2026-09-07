use crate::AppState;
use crate::services::workspace_guard::WorkspaceGuard;
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
use tokio::process::Command;

#[derive(Deserialize)]
pub struct QaQuery {
    project: Option<String>,
}

#[derive(Deserialize)]
pub struct QaRunRequest {
    project: Option<String>,
    spec: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/qa/playwright/list", get(playwright_list))
        .route("/api/qa/playwright/run", post(playwright_run))
        .route("/api/qa/playwright/record", post(playwright_record))
        .route("/api/qa/k6-run", post(k6_run))
}

async fn playwright_list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QaQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project" })),
            )
                .into_response()
        }
    };
    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e.to_string() }))).into_response(),
    };

    let tests_dir = safe_project.join("tests").join("e2e");
    let mut specs = Vec::new();

    if tests_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&tests_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".spec.ts") || name.ends_with(".test.ts") {
                    specs.push(name);
                }
            }
        }
    }

    Json(json!({ "specs": specs, "project": safe_project.to_string_lossy() })).into_response()
}

async fn playwright_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<QaRunRequest>,
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
    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e.to_string() }))).into_response(),
    };

    let mut cmd = Command::new("npx");
    cmd.arg("playwright").arg("test");
    if let Some(spec) = &body.spec {
        cmd.arg(spec);
    }
    cmd.current_dir(&safe_project);

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Json(json!({
                "success": output.status.success(),
                "stdout": stdout,
                "stderr": stderr,
                "exitCode": output.status.code(),
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to run playwright: {}", e) })),
        )
            .into_response(),
    }
}

async fn playwright_record(
    State(state): State<Arc<AppState>>,
    Json(body): Json<QaRunRequest>,
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
    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e.to_string() }))).into_response(),
    };

    let url = body.spec.unwrap_or_else(|| "http://localhost:3000".into());

    let mut cmd = Command::new("npx");
    cmd.arg("playwright").arg("codegen").arg(&url);
    cmd.current_dir(&safe_project);

    match cmd.spawn() {
        Ok(_) => Json(json!({ "success": true, "message": "Playwright codegen started" }))
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to start codegen: {}", e) })),
        )
            .into_response(),
    }
}

async fn k6_run(State(state): State<Arc<AppState>>, Json(body): Json<QaRunRequest>) -> Response {
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
    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e.to_string() }))).into_response(),
    };

    let guard = match WorkspaceGuard::new(&safe_project) {
        Ok(g) => g,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    };
    let script = body
        .spec
        .unwrap_or_else(|| "tests/k6/analyze-project.js".into());
    let script_path = match guard.resolve_safe(&script) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e.to_string() }))).into_response(),
    };

    if !script_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Script not found: {}", script) })),
        )
            .into_response();
    }

    let mut cmd = Command::new("k6");
    cmd.arg("run")
        .arg(script_path.to_string_lossy().to_string());
    cmd.current_dir(&safe_project);

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Json(json!({
                "success": output.status.success(),
                "stdout": stdout,
                "stderr": stderr,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to run k6: {}", e) })),
        )
            .into_response(),
    }
}
