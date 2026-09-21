//! Token-killer compression endpoint. Compresses arbitrary text or a project
//! file to its structural skeleton (imports + signatures + types), cutting the
//! tokens that downstream LLM calls would otherwise spend.

use crate::domains::token_compression::FilterLevel;
use crate::services::file_service;
use crate::domains::token_compression;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::{get, post}, Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct CompressRequest {
    /// Raw text to compress. Mutually exclusive with `project`+`path`.
    content: Option<String>,
    /// Project root; combined with `path` to read a file from disk.
    project: Option<String>,
    /// File path relative to `project` (also used for language detection).
    path: Option<String>,
    /// Compression level: none | minimal | aggressive (default aggressive).
    level: Option<String>,
}

#[derive(Deserialize)]
pub struct CompressCommandRequest {
    pub cmd: String,
    pub output: String,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/compress", post(compress))
        .route("/api/compress/stats", get(get_compress_stats))
        .route("/api/compress/command", post(compress_command))
        .route("/api/compress/pipeline", post(compress_pipeline))
}

#[derive(Deserialize)]
pub struct CompressPipelineRequest {
    pub output: String,
    pub rule_name: Option<String>,
    pub custom_rule: Option<crate::services::pipeline_filter::PipelineRule>,
}

async fn compress_pipeline(
    Json(body): Json<CompressPipelineRequest>,
) -> Response {
    let rule = if let Some(custom) = body.custom_rule {
        custom
    } else if let Some(ref name) = body.rule_name {
        match crate::services::pipeline_filter::get_builtin_rules().iter().find(|r| r.name == *name) {
            Some(r) => r.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Built-in rule '{}' not found", name) })),
                ).into_response();
            }
        }
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Either rule_name or custom_rule must be specified" })),
        ).into_response();
    };

    let filtered = crate::services::pipeline_filter::PipelineFilterEngine::filter(&body.output, &rule);
    Json(json!({
        "rule": rule.name,
        "originalBytes": body.output.len(),
        "compressedBytes": filtered.len(),
        "content": filtered,
    })).into_response()
}

async fn get_compress_stats() -> Response {
    let stats = crate::domains::token_compression::TokenSavingsTracker::global().get_stats();
    Json(stats).into_response()
}

async fn compress_command(
    Json(body): Json<CompressCommandRequest>,
) -> Response {
    let result = crate::domains::token_compression::compress_by_command(&body.cmd, &body.output);
    Json(result).into_response()
}

async fn compress(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<CompressRequest>,
) -> Response {
    let level = match body.level.as_deref() {
        Some(s) => match FilterLevel::from_str(s) {
            Ok(l) => l,
            Err(e) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response()
            }
        },
        None => FilterLevel::Aggressive,
    };

    // Resolve content: inline `content` wins, else read from project+path.
    let (content, hint) = if let Some(c) = body.content {
        let hint = body.path.clone().unwrap_or_default();
        (c, hint)
    } else {
        let project = match body.project {
            Some(p) => p,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "Provide either `content`, or `project`+`path`" })),
                )
                    .into_response()
            }
        };
        let path = match body.path {
            Some(p) => p,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "Missing path" })),
                )
                    .into_response()
            }
        };
        match file_service::read_file_content(&project, &path) {
            Ok(c) => (c, path),
            Err(e) => return (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
        }
    };

    let result = token_compression::compress(&content, &hint, level);
    Json(result).into_response()
}

