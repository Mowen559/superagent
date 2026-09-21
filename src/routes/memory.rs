use crate::models::memory::SupermemoryRecord;
use crate::services::supermemory_store::SupermemoryStore;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct MemoryQuery {
    project: Option<String>,
    q: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct MemoryWriteRequest {
    project: Option<String>,
    record: Option<SupermemoryRecord>,
}

#[derive(Deserialize)]
pub struct ChatRequest {
    project: Option<String>,
    message: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/memory/search", get(memory_search))
        .route("/api/memory/write", post(memory_write))
        .route("/api/supermemory-chat", post(supermemory_chat))
        .route("/api/supermemory-logs", get(supermemory_logs))
}

async fn memory_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemoryQuery>,
) -> Response {
    let query = params.q.unwrap_or_default();
    let limit = params.limit.unwrap_or(20);
    let store = SupermemoryStore::new(&state.runtime_config, params.project.as_deref());
    let results = store.search(&query, limit);
    Json(json!({ "results": results, "query": query })).into_response()
}

async fn memory_write(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MemoryWriteRequest>,
) -> Response {
    let record = match body.record {
        Some(r) => r,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing record" })),
            )
                .into_response()
        }
    };
    let store = SupermemoryStore::new(&state.runtime_config, body.project.as_deref());
    store.write_memory(record.clone());
    Json(json!({ "success": true, "id": record.id })).into_response()
}

async fn supermemory_chat(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ChatRequest>,
) -> Response {
    let message = body.message.unwrap_or_default();
    let store = SupermemoryStore::new(&state.runtime_config, body.project.as_deref());

    // Search relevant memories
    let memories = store.search(&message, 5);

    // Hybrid recall from the code graph: FTS keywords + semantic vectors (RRF fused)
    let db_path = state
        .runtime_config
        .codebase_memory_db_path(body.project.as_deref());
    let db_path_str = db_path.to_string_lossy().to_string();
    let msg_for_search = message.clone();
    let graph_hits = tokio::task::spawn_blocking(move || {
        crate::domains::knowledge_graph::vector_service::hybrid_search(&db_path_str, &msg_for_search, 8)
    })
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();

    // Try LLM
    let llm = crate::domains::model_gateway::providers::LlmProvider::new(state.http_client.clone());
    let response = if llm.is_available() && !message.is_empty() {
        let memory_ctx: Vec<String> = memories
            .iter()
            .map(|m| format!("- {} ({})", m.summary, m.file_path))
            .collect();
        let graph_ctx: Vec<String> = graph_hits
            .iter()
            .map(|n| {
                format!(
                    "- [{}] {} ({}) via {}",
                    n.label,
                    n.name,
                    n.file_path,
                    n.sources.join("+")
                )
            })
            .collect();
        let system = format!(
            "You are a codebase assistant. Use the following project context. Always answer in the same language the user's question is written in.\n\nProject memories:\n{}\n\nRelated code symbols (from indexed graph):\n{}",
            memory_ctx.join("\n"),
            graph_ctx.join("\n")
        );
        let messages = vec![json!({ "role": "user", "content": message })];
        llm.chat(messages, Some(&system))
            .await
            .unwrap_or_else(|e| (format!("LLM error: {}", e), Default::default()))
    } else {
        (format!("LLM not configured. Memory search returned {} results, graph search returned {} symbols.", memories.len(), graph_hits.len()), Default::default())
    };

    let response_str = response.0;

    store.add_log("query", Some(response_str.clone()), Some(message));
    Json(json!({
        "response": response_str,
        "memoriesUsed": memories.len(),
        "graphSymbolsUsed": graph_hits.len(),
    }))
    .into_response()
}

async fn supermemory_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemoryQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50);
    let store = SupermemoryStore::new(&state.runtime_config, params.project.as_deref());
    let logs = store.get_logs(limit);
    Json(json!({ "logs": logs })).into_response()
}
