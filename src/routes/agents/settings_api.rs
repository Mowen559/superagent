use axum::Json;
use crate::config::agent_settings::AgentSettings;

pub async fn get_settings() -> Json<AgentSettings> {
    Json(AgentSettings::load())
}

pub async fn update_settings(Json(payload): Json<AgentSettings>) -> Json<serde_json::Value> {
    match payload.save() {
        Ok(_) => Json(serde_json::json!({ "success": true })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e }))
    }
}

