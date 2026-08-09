use std::sync::Arc;
use axum::{Router, routing::{get, post, delete}, Json, extract::Path, response::IntoResponse};
use serde_json::json;
use crate::AppState;
use crate::services::agent::skills;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/skills", get(list_skills))
        .route("/api/skills/{name}/install", post(install_skill))
        .route("/api/skills/{name}", delete(remove_skill))
}

async fn list_skills() -> axum::response::Response {
    let mut cache = skills::ScannerCache::new();
    let mut paths = vec![];
    if let Some(dir) = crate::config::app_paths::AppPaths::home_dir() {
        paths.push(dir.join(".agents").join("skills"));
    }
    let available_skills = skills::scan_skills_for_agent("cli", &paths, &mut cache);
    
    // Map to simple JSON representation
    let skills_json: Vec<serde_json::Value> = available_skills.into_iter().map(|s| {
        json!({
            "name": s.name,
            "directory": s.directory,
            "description": s.description.unwrap_or_default()
        })
    }).collect();
    
    Json(json!({"skills": skills_json})).into_response()
}

async fn install_skill(Path(name): Path<String>) -> axum::response::Response {
    // Stub implementation mirroring CLI
    Json(json!({
        "success": true, 
        "message": format!("Installing skill: {} (Stub)", name)
    })).into_response()
}

async fn remove_skill(Path(name): Path<String>) -> axum::response::Response {
    match skills::remove_agents_lock_skill(&name) {
        Ok(true) => Json(json!({"success": true, "message": format!("Removed {} from .skill-lock.json", name)})).into_response(),
        Ok(false) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Skill {} not found in .skill-lock.json", name)}))
        ).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to remove skill: {}", e)}))
        ).into_response()
    }
}
