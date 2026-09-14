use crate::AppState;
use axum::{
    extract::Path,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct RegisterToolsRequest {
    #[serde(alias = "service_name")]
    pub service_name: String,
    #[serde(alias = "callback_url", alias = "endpoint_url")]
    pub callback_url: String,
    pub tools: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub struct InvokeToolRequest {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/mcp", get(list_mcp))
        .route("/api/mcp/{name}/install", post(install_mcp))
        .route("/api/mcp/{name}", delete(remove_mcp))
        .route("/api/mcp/tools", get(list_mcp_tools))
        .route("/api/mcp/tools/register", post(register_mcp_tools))
        .route("/api/mcp/tools/invoke", post(invoke_mcp_tool))
}

async fn list_mcp_tools() -> axum::response::Response {
    let tools = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
        .lock()
        .unwrap()
        .get_all_tools();

    Json(json!({
        "success": true,
        "tools": tools,
        "count": tools.len()
    }))
    .into_response()
}

async fn register_mcp_tools(Json(req): Json<RegisterToolsRequest>) -> axum::response::Response {
    let count = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
        .lock()
        .unwrap()
        .register_service_tools(&req.service_name, &req.callback_url, req.tools);

    Json(json!({
        "success": true,
        "serviceName": req.service_name,
        "registeredCount": count,
        "message": format!("Successfully registered {} tools for service '{}'", count, req.service_name)
    }))
    .into_response()
}

async fn invoke_mcp_tool(Json(req): Json<InvokeToolRequest>) -> axum::response::Response {
    let result = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
        .lock()
        .unwrap()
        .invoke_tool(&req.tool, req.arguments);

    match result {
        Ok(data) => Json(json!({
            "success": true,
            "tool": req.tool,
            "result": data
        }))
        .into_response(),
        Err(err) => Json(json!({
            "success": false,
            "tool": req.tool,
            "error": err
        }))
        .into_response(),
    }
}

async fn list_mcp() -> axum::response::Response {
    // Stub implementation mirroring CLI
    Json(json!({
        "success": true,
        "message": "Listing MCP servers (Stub)",
        "servers": []
    }))
    .into_response()
}

async fn install_mcp(Path(name): Path<String>) -> axum::response::Response {
    // Stub implementation mirroring CLI
    Json(json!({
        "success": true,
        "message": format!("Installing MCP server: {} (Stub)", name)
    }))
    .into_response()
}

async fn remove_mcp(Path(name): Path<String>) -> axum::response::Response {
    // Stub implementation mirroring CLI
    Json(json!({
        "success": true,
        "message": format!("Removing MCP server: {} (Stub)", name)
    }))
    .into_response()
}

