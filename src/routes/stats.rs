use crate::services::usage_tracker::{self, DailyUsage, UsageLog};
use axum::extract::Query;
use axum::{routing::get, Json, Router};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct StatsQuery {
    pub days: Option<u32>,
    pub limit: Option<u32>,
}

pub fn router() -> Router<std::sync::Arc<crate::server::AppState>> {
    Router::new()
        .route("/api/stats/daily", get(handle_daily_usage))
        .route(
            "/api/stats/logs",
            axum::routing::get(handle_usage_logs).delete(handle_clear_logs),
        )
}

async fn handle_daily_usage(Query(query): Query<StatsQuery>) -> Json<Vec<DailyUsage>> {
    let days = query.days.unwrap_or(30);
    match usage_tracker::get_daily_usage(days) {
        Ok(data) => Json(data),
        Err(e) => {
            tracing::error!("Failed to fetch daily usage: {}", e);
            Json(vec![])
        }
    }
}

async fn handle_usage_logs(Query(query): Query<StatsQuery>) -> Json<Vec<UsageLog>> {
    let limit = query.limit.unwrap_or(100);
    match usage_tracker::get_usage_logs(limit) {
        Ok(data) => Json(data),
        Err(e) => {
            tracing::error!("Failed to fetch usage logs: {}", e);
            Json(vec![])
        }
    }
}

async fn handle_clear_logs() -> Json<serde_json::Value> {
    match usage_tracker::clear_usage_logs() {
        Ok(_) => Json(serde_json::json!({ "success": true })),
        Err(e) => {
            tracing::error!("Failed to clear logs: {}", e);
            Json(serde_json::json!({ "success": false, "error": e.to_string() }))
        }
    }
}
