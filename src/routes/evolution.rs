use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/evolution/strategies", get(list_strategies))
        .route("/api/evolution/lessons", get(list_lessons))
        .route("/api/evolution/lessons/feedback", post(lesson_feedback))
}

async fn list_lessons(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let lessons = state.evolution_memory.list_lessons().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": error.to_string(),
                "kind": "evolution_store_error"
            })),
        )
    })?;
    Ok(Json(json!({
        "total": lessons.len(),
        "lessons": lessons,
    })))
}

async fn list_strategies(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let strategies = state.evolution_memory.list().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": error.to_string(),
                "kind": "evolution_store_error"
            })),
        )
    })?;
    let promoted = strategies
        .iter()
        .filter(|item| {
            item.state == crate::services::brain::evolution_memory::StrategyState::Promoted
        })
        .count();
    Ok(Json(json!({
        "strategies": strategies,
        "summary": {
            "total": strategies.len(),
            "promoted": promoted,
            "nonPromoted": strategies.len().saturating_sub(promoted),
        }
    })))
}

#[derive(Deserialize)]
pub struct LessonFeedbackPayload {
    pub lesson_id: String,
    pub positive: bool,
}

async fn lesson_feedback(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LessonFeedbackPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .evolution_memory
        .update_lesson_feedback(&payload.lesson_id, payload.positive)
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": error.to_string(),
                    "kind": "evolution_store_error"
                })),
            )
        })?;
    let _ = state.evolution_memory.prune_stale_lessons();
    Ok(Json(json!({
        "success": true,
        "lesson_id": payload.lesson_id,
        "positive": payload.positive,
    })))
}
