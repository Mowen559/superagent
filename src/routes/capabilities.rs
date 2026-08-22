use crate::services::brain::capabilities::certification::CapabilityState;
use crate::AppState;
use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

/// Stable local Gateway discovery surface. Clients use this instead of
/// guessing support from executable names or README claims.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/capabilities", get(list_capabilities))
        .route("/api/capabilities/{name}", get(get_capability))
}

async fn list_capabilities(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let capabilities = state.certification.list_capabilities();
    let certified = capabilities
        .iter()
        .filter(|c| matches!(c.state, CapabilityState::Certified))
        .count();
    let experimental = capabilities
        .iter()
        .filter(|c| matches!(c.state, CapabilityState::Experimental))
        .count();
    Json(json!({
        "capabilities": capabilities,
        "summary": { "total": certified + experimental, "certified": certified, "experimental": experimental },
        "checkedAt": chrono::Utc::now().to_rfc3339(),
    }))
}

async fn get_capability(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    let capability = state
        .certification
        .list_capabilities()
        .into_iter()
        .find(|c| c.name == name);
    match capability {
        Some(capability) => Json(json!({ "capability": capability })),
        None => Json(json!({ "capability": null, "error": "unknown capability" })),
    }
}
