use std::sync::Arc;
use axum::{Router, routing::get, extract::{State, Query}, Json};
use serde::Deserialize;
use crate::AppState;
use crate::models::api::{self, ApiResponse};

#[derive(Deserialize)]
pub struct HealthQuery {
    project: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/health", get(health_check))
}

async fn health_check(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HealthQuery>,
) -> Json<ApiResponse<serde_json::Value>> {
    let project_path = params.project.as_deref();
    let statuses = state.runtime_config.runtime_statuses(project_path);

    let has_error = statuses.iter().any(|s| s.state == crate::config::runtime::HealthState::Error);
    let has_warning = statuses.iter().any(|s| s.state == crate::config::runtime::HealthState::Warning);

    let overall = if has_error { "error" } else if has_warning { "warning" } else { "ok" };

    let data = serde_json::json!({
        "overall": overall,
        "statuses": statuses,
        "checkedAt": chrono::Utc::now().to_rfc3339(),
    });

    let project_id = project_path.unwrap_or("default");
    api::success(data, project_id)
}
