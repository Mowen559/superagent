use axum::{
    extract::Path,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::services::agent::registry_db::{
    delete_mcp, delete_skill, get_all_mcps, get_all_skills, upsert_mcp_server, upsert_skill,
    McpServerRow, SkillRow,
};

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

pub fn router() -> Router<std::sync::Arc<crate::server::AppState>> {
    Router::new()
        .route(
            "/api/registry/mcps",
            get(list_mcps).post(create_or_update_mcp),
        )
        .route("/api/registry/mcps/{id}", delete(remove_mcp))
        .route(
            "/api/registry/skills",
            get(list_skills).post(create_or_update_skill),
        )
        .route("/api/registry/skills/{id}", delete(remove_skill))
}

async fn list_mcps() -> Json<ApiResponse<Vec<McpServerRow>>> {
    match get_all_mcps() {
        Ok(mcps) => Json(ApiResponse {
            success: true,
            data: Some(mcps),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e.to_string()),
        }),
    }
}

async fn create_or_update_mcp(Json(payload): Json<McpServerRow>) -> Json<ApiResponse<()>> {
    match upsert_mcp_server(payload) {
        Ok(_) => Json(ApiResponse {
            success: true,
            data: Some(()),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e.to_string()),
        }),
    }
}

async fn remove_mcp(Path(id): Path<String>) -> Json<ApiResponse<()>> {
    match delete_mcp(&id) {
        Ok(_) => Json(ApiResponse {
            success: true,
            data: Some(()),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e.to_string()),
        }),
    }
}

async fn list_skills() -> Json<ApiResponse<Vec<SkillRow>>> {
    match get_all_skills() {
        Ok(skills) => Json(ApiResponse {
            success: true,
            data: Some(skills),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e.to_string()),
        }),
    }
}

async fn create_or_update_skill(Json(payload): Json<SkillRow>) -> Json<ApiResponse<()>> {
    match upsert_skill(payload) {
        Ok(_) => Json(ApiResponse {
            success: true,
            data: Some(()),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e.to_string()),
        }),
    }
}

async fn remove_skill(Path(id): Path<String>) -> Json<ApiResponse<()>> {
    match delete_skill(&id) {
        Ok(_) => Json(ApiResponse {
            success: true,
            data: Some(()),
            error: None,
        }),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e.to_string()),
        }),
    }
}
