use std::sync::Arc;
use axum::{Router, routing::{get, post}, extract::{State, Path}, Json};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/{task_id}", get(get_task))
        .route("/api/tasks/{task_id}/logs", get(get_task_logs))
        .route("/api/tasks/{task_id}/cancel", post(cancel_task))
}

async fn list_tasks(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let tasks = state.task_manager.list_tasks();
    Json(json!({ "tasks": tasks }))
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    match state.task_manager.get_task(&task_id) {
        Some(task) => Json(json!(task)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "Task not found" }))).into_response(),
    }
}

async fn get_task_logs(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let logs = state.task_manager.get_logs(&task_id);
    Json(json!({ "taskId": task_id, "logs": logs })).into_response()
}

async fn cancel_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    if state.task_manager.get_task(&task_id).is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Task not found" }))).into_response();
    }
    state.task_manager.cancel_task(&task_id);
    Json(json!({ "taskId": task_id, "status": "cancelled" })).into_response()
}
