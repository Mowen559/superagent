use std::sync::Arc;
use axum::{Router, routing::post, extract::State, Json};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use crate::AppState;
use crate::services::analysis_service;

#[derive(Deserialize)]
pub struct AnalyzeRequest {
    path: Option<String>,
    #[serde(rename = "projectId")]
    project_id: Option<String>,
    options: Option<serde_json::Value>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/projects/analyze", post(analyze_project))
}

async fn analyze_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequest>,
) -> Response {
    let target_path = match body.path {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing path" }))).into_response(),
    };

    let task_manager = Arc::new(crate::services::task_manager::TaskManager::new(&state.app_paths));

    match analysis_service::start_analysis(
        &state.runtime_config,
        &state.app_paths,
        task_manager,
        state.http_client.clone(),
        &target_path,
        body.project_id,
    ).await {
        Ok(task_id) => Json(json!({
            "taskId": task_id,
            "status": "pending",
            "message": "Analysis started"
        })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}
