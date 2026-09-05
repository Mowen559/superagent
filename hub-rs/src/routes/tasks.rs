use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/{task_id}", get(get_task))
        .route("/api/tasks/{task_id}/logs", get(get_task_logs))
        .route(
            "/api/tasks/{task_id}/events",
            get(crate::routes::agents::stream_events),
        )
        .route("/api/tasks/{task_id}/cancel", post(cancel_task))
}

async fn list_tasks(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let tasks = state.task_manager.list_tasks();
    Json(json!({ "tasks": tasks }))
}

async fn get_task(State(state): State<Arc<AppState>>, Path(task_id): Path<String>) -> Response {
    match state.task_manager.get_task(&task_id) {
        Some(task) => Json(json!(task)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response(),
    }
}

async fn get_task_logs(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let logs = state.task_manager.get_logs(&task_id);
    Json(json!({ "taskId": task_id, "logs": logs })).into_response()
}

async fn cancel_task(State(state): State<Arc<AppState>>, Path(task_id): Path<String>) -> Response {
    if state.task_manager.get_task(&task_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    }
    if state.task_manager.cancel_task(&task_id) {
        Json(json!({ "taskId": task_id, "status": "cancelled" })).into_response()
    } else {
        let status = state
            .task_manager
            .get_task(&task_id)
            .map(|task| task.status);
        (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "task is already terminal",
                "kind": "terminal_task",
                "status": status,
            })),
        )
            .into_response()
    }
}
