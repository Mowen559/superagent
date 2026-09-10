use crate::services::analysis_service;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct AnalyzeRequest {
    path: Option<String>,
    #[serde(rename = "projectId")]
    project_id: Option<String>,
    options: Option<analysis_service::ProjectAnalysisOptions>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/projects/analyze", post(analyze_project))
}

async fn analyze_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequest>,
) -> Response {
    let target_path = match body.path {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing path" })),
            )
                .into_response()
        }
    };

    match analysis_service::start_analysis(
        &state.runtime_config,
        &state.app_paths,
        state.task_manager.clone(),
        state.http_client.clone(),
        &target_path,
        body.project_id,
        body.options.unwrap_or_default(),
    )
    .await
    {
        Ok(task_id) => Json(json!({
            "taskId": task_id,
            "status": "pending",
            "message": "Analysis started"
        }))
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}
