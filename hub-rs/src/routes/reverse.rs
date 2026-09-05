use crate::models::reverse::ReverseEngineeringRequest;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/reverse-engineering", post(reverse_engineering))
        .route(
            "/api/reverse/analyze-requirement",
            post(analyze_requirement),
        )
        .route("/api/reverse/plan-requirement", post(plan_requirement))
}

async fn reverse_engineering(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ReverseEngineeringRequest>,
) -> Response {
    let trace_id = uuid::Uuid::new_v4().to_string();
    let requirement = body.requirement.clone().unwrap_or_default();

    if requirement.is_empty() && body.extracted_schema.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Missing requirement or extracted_schema" })),
        )
            .into_response();
    }

    let reporter = std::sync::Arc::new(crate::services::progress::ConsoleReporter::new());

    // Run the reverse engineering pipeline
    let result = crate::reverse_engineering::engine::execute(
        &trace_id,
        &requirement,
        body.extracted_schema,
        body.project.as_deref(),
        body.options.as_ref(),
        &state.runtime_config,
        &state.http_client,
        Some(reporter),
    )
    .await;

    match result {
        Ok(result) => Json(serde_json::to_value(result).unwrap_or_default()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e, "traceId": trace_id })),
        )
            .into_response(),
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AnalyzeRequirementRequest {
    project: Option<String>,
    requirement: Option<String>,
}

#[axum::debug_handler]
async fn analyze_requirement(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequirementRequest>,
) -> impl IntoResponse {
    let project = match body.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let requirement = match body.requirement {
        Some(r) if !r.is_empty() => r,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing requirement param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    let llm = crate::services::llm_provider::LlmProvider::new(state.http_client.clone());
    let analyzer = crate::services::requirement_analyzer::RequirementAnalyzer::new(llm);

    let reporter = std::sync::Arc::new(crate::services::progress::ConsoleReporter::new());

    match analyzer
        .analyze_requirement(
            std::path::Path::new(&project),
            &db_path_str,
            &requirement,
            Some(reporter),
        )
        .await
    {
        Ok(_) => {
            Json(json!({ "success": true, "message": "Impact report generated" })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[axum::debug_handler]
async fn plan_requirement(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequirementRequest>,
) -> impl IntoResponse {
    let project = match body.project {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let requirement = match body.requirement {
        Some(r) if !r.is_empty() => r,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing requirement param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    let llm = crate::services::llm_provider::LlmProvider::new(state.http_client.clone());
    let analyzer = crate::services::requirement_analyzer::RequirementAnalyzer::new(llm);

    let reporter = std::sync::Arc::new(crate::services::progress::ConsoleReporter::new());

    match analyzer
        .plan_requirement(
            std::path::Path::new(&project),
            &db_path_str,
            &requirement,
            Some(reporter),
        )
        .await
    {
        Ok(_) => {
            Json(json!({ "success": true, "message": "Execution plan generated" })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
