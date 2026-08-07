//! Token-killer compression endpoint. Compresses arbitrary text or a project
//! file to its structural skeleton (imports + signatures + types), cutting the
//! tokens that downstream LLM calls would otherwise spend.

use std::sync::Arc;
use std::str::FromStr;
use axum::{Router, routing::post, extract::State, Json};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use crate::AppState;
use crate::services::{file_service, token_filter};
use crate::services::token_filter::FilterLevel;

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

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/compress", post(compress))
}

async fn compress(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<CompressRequest>,
) -> Response {
    let level = match body.level.as_deref() {
        Some(s) => match FilterLevel::from_str(s) {
            Ok(l) => l,
            Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
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
            None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Provide either `content`, or `project`+`path`" }))).into_response(),
        };
        let path = match body.path {
            Some(p) => p,
            None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing path" }))).into_response(),
        };
        match file_service::read_file_content(&project, &path) {
            Ok(c) => (c, path),
            Err(e) => return (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
        }
    };

    let result = token_filter::compress(&content, &hint, level);
    Json(result).into_response()
}
