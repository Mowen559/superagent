use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/analyze/architecture", post(analyze_architecture))
        .route("/api/analyze/domain", post(analyze_domain))
        .route("/api/analyze/impact", post(analyze_impact))
        .route("/api/analyze/tour", post(analyze_tour))
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AnalyzeRequest {
    project: Option<String>,
    lang: Option<String>,
}

async fn get_db_path_and_project(
    state: &Arc<AppState>,
    body: &AnalyzeRequest,
) -> Result<(String, String), Response> {
    let project = match &body.project {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response())
        }
    };
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();
    Ok((project, db_path_str))
}

#[axum::debug_handler]
async fn analyze_architecture(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    let (_, db_path_str) = match get_db_path_and_project(&state, &body).await {
        Ok(res) => res,
        Err(err) => return err,
    };

    let llm = crate::services::llm_provider::LlmProvider::new(state.http_client.clone());
    let analyzer = crate::services::architecture_analyzer::ArchitectureAnalyzer::new(llm);

    match analyzer
        .analyze_layers(&db_path_str, body.lang.as_deref())
        .await
    {
        Ok(report) => Json(json!({ "success": true, "report": report })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[axum::debug_handler]
async fn analyze_domain(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    let (_, db_path_str) = match get_db_path_and_project(&state, &body).await {
        Ok(res) => res,
        Err(err) => return err,
    };

    let llm = crate::services::llm_provider::LlmProvider::new(state.http_client.clone());
    let analyzer = crate::services::domain_analyzer::DomainAnalyzer::new(llm);

    match analyzer
        .analyze_domain_flows(&db_path_str, body.lang.as_deref())
        .await
    {
        Ok(report) => Json(json!({ "success": true, "report": report })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[axum::debug_handler]
async fn analyze_impact(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    let (project, db_path_str) = match get_db_path_and_project(&state, &body).await {
        Ok(res) => res,
        Err(err) => return err,
    };

    let llm = crate::services::llm_provider::LlmProvider::new(state.http_client.clone());
    let analyzer = crate::services::impact_analyzer::ImpactAnalyzer::new(llm);

    match analyzer
        .analyze_impact(
            std::path::Path::new(&project),
            &db_path_str,
            body.lang.as_deref(),
        )
        .await
    {
        Ok(report) => Json(json!({ "success": true, "report": report })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[axum::debug_handler]
async fn analyze_tour(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    let (_, db_path_str) = match get_db_path_and_project(&state, &body).await {
        Ok(res) => res,
        Err(err) => return err,
    };

    let builder = crate::services::tour_builder::TourBuilder::new();

    match builder.build_tour(&db_path_str) {
        Ok(report) => Json(json!({ "success": true, "report": report })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
