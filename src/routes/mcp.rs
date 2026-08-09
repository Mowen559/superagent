use std::sync::Arc;
use axum::{Router, routing::{get, post, delete}, Json, extract::Path, response::IntoResponse};
use serde_json::json;
use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/mcp", get(list_mcp))
        .route("/api/mcp/{name}/install", post(install_mcp))
        .route("/api/mcp/{name}", delete(remove_mcp))
}

async fn list_mcp() -> axum::response::Response {
    // Stub implementation mirroring CLI
    Json(json!({
        "success": true, 
        "message": "Listing MCP servers (Stub)",
        "servers": []
    })).into_response()
}

async fn install_mcp(Path(name): Path<String>) -> axum::response::Response {
    // Stub implementation mirroring CLI
    Json(json!({
        "success": true, 
        "message": format!("Installing MCP server: {} (Stub)", name)
    })).into_response()
}

async fn remove_mcp(Path(name): Path<String>) -> axum::response::Response {
    // Stub implementation mirroring CLI
    Json(json!({
        "success": true, 
        "message": format!("Removing MCP server: {} (Stub)", name)
    })).into_response()
}
