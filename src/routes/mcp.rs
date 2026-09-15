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
    let tools = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
        .lock()
        .unwrap()
        .get_all_tools();

    let mut service_map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for t in tools {
        service_map.entry(t.service_name.clone()).or_default().push(t.name);
    }

    let servers: Vec<Value> = service_map
        .into_iter()
        .map(|(service_name, tool_names)| {
            json!({
                "id": format!("mcp-{}", service_name),
                "name": service_name,
                "status": "synced",
                "source": "local",
                "tools": tool_names,
                "toolCount": tool_names.len()
            })
        })
        .collect();

    Json(json!({
        "success": true,
        "count": servers.len(),
        "servers": servers
    }))
    .into_response()
}

async fn install_mcp(Path(name): Path<String>) -> axum::response::Response {
    let tool_name = format!("{}_query", name.replace(|c: char| !c.is_alphanumeric() && c != '_', "_"));
    let raw_tool = json!({
        "name": tool_name,
        "description": format!("Dynamic MCP capability for service {}", name),
        "inputSchema": { "type": "object", "properties": {} }
    });

    let count = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
        .lock()
        .unwrap()
        .register_service_tools(&name, "http://127.0.0.1:3000", vec![raw_tool]);

    Json(json!({
        "success": true,
        "server": name,
        "registeredTools": count,
        "message": format!("MCP server '{}' registered into tool ecosystem successfully", name)
    }))
    .into_response()
}

async fn remove_mcp(Path(name): Path<String>) -> axum::response::Response {
    let removed = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
        .lock()
        .unwrap()
        .remove_service_tools(&name);

    Json(json!({
        "success": true,
        "server": name,
        "removedTools": removed,
        "message": format!("MCP server '{}' removed successfully (unregistered {} tools)", name, removed)
    }))
    .into_response()
}

