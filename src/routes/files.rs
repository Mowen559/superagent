use crate::services::token_filter::{self, FilterLevel};
use crate::services::{file_service, graph_service};
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
use std::str::FromStr;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct FilesQuery {
    project: Option<String>,
    depth: Option<usize>,
    labels: Option<String>,
}

#[derive(Deserialize)]
pub struct FileContentQuery {
    project: Option<String>,
    path: Option<String>,
    /// Legacy frontend alias for `path`.
    #[serde(rename = "filePath")]
    file_path: Option<String>,
    /// Optional token-killer compression: none | minimal | aggressive.
    filter: Option<String>,
}

#[derive(Deserialize)]
pub struct AnalyzeFileRequest {
    project: Option<String>,
    path: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/files", get(list_files))
        .route("/api/file-content", get(get_file_content))
        .route("/api/analyze-file", post(analyze_file))
        .route("/api/project-graph", get(get_project_graph))
}

async fn list_files(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FilesQuery>,
) -> Response {
    let project_path = match params.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let safe_project_path = match state.workspace_capabilities.resolve(&project_path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": format!("Workspace access denied: {}", e) })),
            )
                .into_response()
        }
    };

    let tree = file_service::build_file_tree(&safe_project_path, params.depth);
    Json(json!({ "files": tree, "project": project_path })).into_response()
}

async fn get_file_content(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FileContentQuery>,
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
    let file_path = match params.path.or(params.file_path) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing path param" })),
            )
                .into_response()
        }
    };

    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": format!("Workspace access denied: {}", e) })),
            )
                .into_response()
        }
    };
    let guard = match crate::services::workspace_guard::WorkspaceGuard::new(&safe_project) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Invalid workspace root: {}", e) })),
            )
                .into_response()
        }
    };
    if let Err(e) = guard.resolve_safe(&file_path) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": format!("Path traversal detected: {}", e) })),
        )
            .into_response();
    }

    match file_service::read_file_content(&safe_project, &file_path) {
        Ok(content) => {
            // Optional token-killer compression before returning content.
            match params.filter.as_deref().map(FilterLevel::from_str) {
                Some(Ok(level)) if level != FilterLevel::None => {
                    let r = token_filter::compress(&content, &file_path, level);
                    Json(json!({
                        "content": r.content,
                        "path": file_path,
                        "filter": r.level,
                        "language": r.language,
                        "originalLines": r.original_lines,
                        "compressedLines": r.compressed_lines,
                        "originalBytes": r.original_bytes,
                        "compressedBytes": r.compressed_bytes,
                        "reductionPercent": r.reduction_percent,
                    }))
                    .into_response()
                }
                Some(Err(e)) => {
                    (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response()
                }
                _ => Json(json!({ "content": content, "path": file_path })).into_response(),
            }
        }
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
    }
}

async fn analyze_file(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeFileRequest>,
) -> Response {
    let project = match body.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project" })),
            )
                .into_response()
        }
    };
    let file_path = match body.path {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing path" })),
            )
                .into_response()
        }
    };

    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": format!("Workspace access denied: {}", e) })),
            )
                .into_response()
        }
    };
    let guard = match crate::services::workspace_guard::WorkspaceGuard::new(&safe_project) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Invalid workspace root: {}", e) })),
            )
                .into_response()
        }
    };
    let safe_file_path = match guard.resolve_safe(&file_path) {
        Ok(_) => file_path.clone(), // keeping the original string for graph/LLM calls
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": format!("Path traversal detected: {}", e) })),
            )
                .into_response()
        }
    };

    let content = match file_service::read_file_content(&safe_project, &safe_file_path) {
        Ok(c) => c,
        Err(e) => return (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
    };

    // Token-killer: compress the file to its structural skeleton before spending
    // LLM tokens on it. Falls back to raw content if compression would not help.
    let compressed = token_filter::compress(&content, &file_path, FilterLevel::Aggressive);

    // Try graph DB enrichment
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();
    let graph_nodes = graph_service::get_file_nodes(&db_path_str, &file_path).unwrap_or_default();

    // Try LLM analysis (on the compressed content to save tokens)
    let llm = crate::domains::model_gateway::providers::LlmProvider::new(state.http_client.clone());
    let analysis = if llm.is_available() {
        llm.analyze_file(&file_path, &compressed.content)
            .await
            .unwrap_or(json!({
                "fileSummary": "LLM analysis unavailable",
                "tags": [],
                "complexity": "moderate"
            }))
    } else {
        json!({
            "fileSummary": "LLM not configured",
            "tags": [],
            "complexity": "moderate"
        })
    };

    Json(json!({
        "path": file_path,
        "analysis": analysis,
        "graphNodes": graph_nodes,
        "lines": content.lines().count(),
        "compression": {
            "level": compressed.level,
            "language": compressed.language,
            "originalBytes": compressed.original_bytes,
            "compressedBytes": compressed.compressed_bytes,
            "reductionPercent": compressed.reduction_percent,
            "originalTokensEst": compressed.original_tokens_est,
            "compressedTokensEst": compressed.compressed_tokens_est,
        },
    }))
    .into_response()
}

async fn get_project_graph(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FilesQuery>,
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

    let safe_project = match state.workspace_capabilities.resolve(&project) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": format!("Workspace access denied: {}", e) })),
            )
                .into_response()
        }
    };

    let db_path = state
        .runtime_config
        .codebase_memory_db_path(Some(&safe_project));
    let db_path_str = db_path.to_string_lossy().to_string();
    let max_nodes = params.depth.unwrap_or(10000);
    // Optional comma-separated node kind filter, e.g. labels=Function,Class
    let labels: Option<Vec<String>> = params.labels.map(|s| {
        s.split(',')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    });

    match graph_service::get_project_graph(&db_path_str, max_nodes, labels.as_deref()) {
        Ok(graph) => Json(json!(graph)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}
