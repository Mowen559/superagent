use std::sync::Arc;
use axum::{Router, routing::get, Json};
use crate::AppState;
use crate::config::llm_settings;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/settings/llm", get(get_llm_settings).put(put_llm_settings))
}

async fn get_llm_settings() -> Json<serde_json::Value> {
    let settings = llm_settings::read_llm_settings();
    Json(serde_json::to_value(settings).unwrap_or_default())
}

async fn put_llm_settings(
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let updated = llm_settings::write_llm_settings(body);
    Json(serde_json::to_value(updated).unwrap_or_default())
}
