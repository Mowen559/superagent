use crate::models::api::{self, ApiResponse};
use crate::AppState;
use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

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

    let has_error = statuses
        .iter()
        .any(|s| s.state == crate::config::runtime::HealthState::Error);
    let has_warning = statuses
        .iter()
        .any(|s| s.state == crate::config::runtime::HealthState::Warning);
    let llm_settings = crate::config::llm_settings::read_llm_settings();
    let gateway_eval = crate::domains::model_gateway::capabilities::eval::GatewayEvalRuntime
        .evaluate_global(&llm_settings);

    let overall = if has_error {
        "error"
    } else if has_warning || !gateway_eval.passed {
        "warning"
    } else {
        "ok"
    };
    let data = serde_json::json!({
        "overall": overall,
        "statuses": statuses,
        "providerHealth": crate::domains::model_gateway::health::ProviderHealthRuntime::global().snapshots(),
        "gatewayEval": gateway_eval,
        "checkedAt": chrono::Utc::now().to_rfc3339(),
    });

    let project_id = project_path.unwrap_or("default");
    api::success(data, project_id)
}
