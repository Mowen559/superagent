use crate::services::{graph_service, vector_service};
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ContextSearchQuery {
    project: Option<String>,
    q: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct ContextProjectQuery {
    project: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/context/search", get(context_search))
        .route("/api/context/semantic", get(context_semantic))
        .route("/api/context/hybrid", get(context_hybrid))
        .route("/api/context/project", get(context_project))
}

/// Full-text symbol search backed by the codebase-memory-mcp FTS5 index.
async fn context_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ContextSearchQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let query = params.q.unwrap_or_default();
    let limit = params.limit.unwrap_or(30);

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    match graph_service::search_nodes(&db_path_str, &query, limit) {
        Ok(results) => Json(json!({ "query": query, "results": results })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Semantic vector search over node embeddings (CBM int8 vectors, min-cosine ranking).
async fn context_semantic(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ContextSearchQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let query = params.q.unwrap_or_default();
    let limit = params.limit.unwrap_or(20);

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    // Vector scoring is CPU-bound over ~26k embeddings; keep it off the async runtime.
    let q = query.clone();
    let result = tokio::task::spawn_blocking(move || {
        vector_service::semantic_search(&db_path_str, &q, limit)
    })
    .await
    .unwrap_or_else(|e| Err(format!("semantic search task failed: {}", e)));

    match result {
        Ok(results) => Json(json!({ "query": query, "results": results })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Hybrid retrieval: FTS5 + semantic vectors fused with Reciprocal Rank Fusion.
async fn context_hybrid(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ContextSearchQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let query = params.q.unwrap_or_default();
    let limit = params.limit.unwrap_or(20);

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    let q = query.clone();
    let result =
        tokio::task::spawn_blocking(move || vector_service::hybrid_search(&db_path_str, &q, limit))
            .await
            .unwrap_or_else(|e| Err(format!("hybrid search task failed: {}", e)));

    match result {
        Ok(results) => Json(json!({ "query": query, "results": results })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Project-level context: indexed summary + basic node stats.
async fn context_project(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ContextProjectQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    let summary = graph_service::get_project_summary(&db_path_str).unwrap_or(None);
    let indexed = db_path.exists();

    Json(json!({
        "project": project,
        "indexed": indexed,
        "summary": summary,
    }))
    .into_response()
}
