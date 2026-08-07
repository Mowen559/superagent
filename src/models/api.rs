use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;

/// Unified API response structure matching the original TypeScript implementation.
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub trace_id: String,
    pub project_id: String,
    #[serde(flatten)]
    pub data: T,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub trace_id: String,
    pub project_id: String,
    pub error: String,
}

pub fn success<T: Serialize>(data: T, project_id: &str) -> Json<ApiResponse<T>> {
    Json(ApiResponse {
        trace_id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.to_string(),
        data,
    })
}

pub fn error(status: StatusCode, message: &str, project_id: Option<&str>) -> Response {
    let body = ApiError {
        trace_id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.unwrap_or("unknown").to_string(),
        error: message.to_string(),
    };
    (status, Json(json!(body))).into_response()
}

/// Application error type for unified error handling
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            AppError::Io(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        error(status, &message, None)
    }
}
