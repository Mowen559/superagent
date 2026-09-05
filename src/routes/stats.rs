use axum::{
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::{services::usage_tracker, AppState};

#[derive(Debug, Deserialize)]
struct LogsQuery {
    limit: Option<usize>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/stats/logs", get(list_logs))
        .route("/api/stats/daily", get(daily_stats))
}

async fn list_logs(Query(query): Query<LogsQuery>) -> Result<Response, (StatusCode, String)> {
    let logs = usage_tracker::list_usage_logs(query.limit.unwrap_or(200))
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(logs).into_response())
}

async fn daily_stats() -> Result<Response, (StatusCode, String)> {
    let stats = usage_tracker::daily_usage_stats()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(stats).into_response())
}
