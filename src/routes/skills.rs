use crate::services::agent::skills;
use crate::AppState;
use axum::{
    extract::Path,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

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
    let skills_json: Vec<serde_json::Value> = available_skills
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name,
                "directory": s.directory,
                "description": s.description.unwrap_or_default()
            })
        })
        .collect();

    Json(json!({"skills": skills_json})).into_response()
}

async fn install_skill(Path(name): Path<String>) -> axum::response::Response {
    let home = crate::config::app_paths::AppPaths::home_dir().unwrap_or_default();
    let skill_dir = home.join(".agents").join("skills").join(&name);
    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create skill directory: {}", e)})),
        )
            .into_response();
    }

    let skill_file = skill_dir.join("SKILL.md");
    if !skill_file.exists() {
        let now = chrono::Utc::now().to_rfc3339();
        let content = format!(
            "---\nname: \"{}\"\ndescription: \"Auto-installed skill: {}\"\nversion: \"1.0.0\"\ncreated_at: \"{}\"\n---\n\n# {}\n\nSkill implementation and guidelines.\n",
            name, name, now, name
        );
        let _ = std::fs::write(&skill_file, content);
    }

    let info = skills::AgentsLockInfo {
        source: Some("local-install".to_string()),
        skill_path: Some(skill_dir.to_string_lossy().to_string()),
        branch: None,
        commit_hash: None,
        installed_at: Some(chrono::Utc::now().to_rfc3339()),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
    };

    match skills::write_agents_lock_skill(&name, &info) {
        Ok(_) => Json(json!({
            "success": true,
            "name": name,
            "path": skill_dir.to_string_lossy().to_string(),
            "message": format!("Skill {} installed and locked successfully", name)
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to update lock file: {}", e)})),
        )
            .into_response(),
    }
}

async fn remove_skill(Path(name): Path<String>) -> axum::response::Response {
    match skills::remove_agents_lock_skill(&name) {
        Ok(true) => Json(
            json!({"success": true, "message": format!("Removed {} from .skill-lock.json", name)}),
        )
        .into_response(),
        Ok(false) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Skill {} not found in .skill-lock.json", name)})),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to remove skill: {}", e)})),
        )
            .into_response(),
    }
}
