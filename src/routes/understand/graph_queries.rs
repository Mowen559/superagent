//! 代码图谱结构检索、调用链追踪与工程质量治理 HTTP 端点

use super::resolve_project_path;
use crate::domains::code_analysis::code_intelligence::CodeIntelligenceRuntime;
use crate::domains::code_analysis::code_similarity::CodeSimilarityDetector;
use crate::domains::project_management::adr_service::AdrService;
use crate::domains::project_management::onboard_service::OnboardService;
use crate::domains::version_control::change_classifier::ChangeClassifier;
use crate::domains::version_control::diff_impact::DiffImpactAnalyzer;
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct UnderstandOnboardQuery {
    pub project: Option<String>,
}

pub async fn understand_onboard(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandOnboardQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    match OnboardService::generate_guide(&project_dir, Some(&db_path_str)) {
        Ok(guide) => Json(json!(guide)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandDiffQuery {
    pub project: Option<String>,
}

pub async fn understand_diff(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandDiffQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    match DiffImpactAnalyzer::detect_changes(&project_dir, &db_path_str) {
        Ok(report) => Json(json!(report)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandTraceQuery {
    pub project: Option<String>,
    #[serde(rename = "functionName")]
    pub function_name: Option<String>,
    pub direction: Option<String>,
    pub depth: Option<usize>,
    pub mode: Option<String>,
}

pub async fn understand_trace(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandTraceQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let fn_name = match params.function_name {
        Some(f) if !f.trim().is_empty() => f,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing functionName param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let rt = CodeIntelligenceRuntime::new(db_path);
    let direction = params.direction.as_deref().unwrap_or("both");
    let depth = params.depth.unwrap_or(3);
    let mode = params.mode.as_deref().unwrap_or("calls");

    match rt.trace_path(&fn_name, direction, depth, mode) {
        Ok(trace) => Json(json!(trace)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandDeadcodeQuery {
    pub project: Option<String>,
    pub limit: Option<usize>,
}

pub async fn understand_deadcode(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandDeadcodeQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let rt = CodeIntelligenceRuntime::new(db_path);
    let limit = params.limit.unwrap_or(50);

    match rt.get_dead_code(limit) {
        Ok(dead) => Json(json!(dead)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandAdrQuery {
    pub project: Option<String>,
    pub id: Option<String>,
}

pub async fn list_or_get_adr(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandAdrQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let project_dir = resolve_project_path(&state, &project);

    if let Some(id) = params.id.filter(|s| !s.trim().is_empty()) {
        match AdrService::get_adr(&project_dir, &id) {
            Ok(adr) => Json(json!(adr)).into_response(),
            Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
        }
    } else {
        match AdrService::list_adrs(&project_dir) {
            Ok(adrs) => Json(json!(adrs)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e })),
            )
                .into_response(),
        }
    }
}

#[derive(Deserialize)]
pub struct CreateAdrPayload {
    pub project: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub context: Option<String>,
    pub decision: Option<String>,
    pub consequences: Option<String>,
}

pub async fn create_adr_endpoint(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateAdrPayload>,
) -> Response {
    let project = match payload.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project in request body" })),
            )
                .into_response()
        }
    };

    let project_dir = resolve_project_path(&state, &project);
    let status = payload.status.as_deref().unwrap_or("Accepted");
    let context = payload.context.as_deref().unwrap_or("");
    let decision = payload.decision.as_deref().unwrap_or("");
    let consequences = payload.consequences.as_deref().unwrap_or("");

    match AdrService::create_adr(
        &project_dir,
        &payload.title,
        status,
        context,
        decision,
        consequences,
    ) {
        Ok(created) => (StatusCode::CREATED, Json(json!(created))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandSearchQuery {
    pub project: Option<String>,
    pub query: Option<String>,
    pub label: Option<String>,
    #[serde(rename = "namePattern")]
    pub name_pattern: Option<String>,
    #[serde(rename = "filePattern")]
    pub file_pattern: Option<String>,
    #[serde(rename = "minDegree")]
    pub min_degree: Option<usize>,
    #[serde(rename = "maxDegree")]
    pub max_degree: Option<usize>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn understand_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandSearchQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let rt = CodeIntelligenceRuntime::new(db_path);

    let query_term = params.query.as_deref().or(params.name_pattern.as_deref());
    let label = params.label.as_deref();
    let file_pattern = params.file_pattern.as_deref();
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    match rt.search_graph(
        label,
        query_term,
        file_pattern,
        params.min_degree,
        params.max_degree,
        limit,
        offset,
    ) {
        Ok(results) => Json(json!(results)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandSearchCodeQuery {
    pub project: Option<String>,
    pub query: Option<String>,
    #[serde(rename = "filePattern")]
    pub file_pattern: Option<String>,
    pub limit: Option<usize>,
}

pub async fn understand_search_code(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandSearchCodeQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };
    let query_str = match params.query {
        Some(q) if !q.trim().is_empty() => q,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing query param" })),
            )
                .into_response()
        }
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let rt = CodeIntelligenceRuntime::new(db_path);
    let limit = params.limit.unwrap_or(50);

    match rt.search_code(
        &query_str,
        params.file_pattern.as_deref(),
        Some(&project_dir),
        limit,
    ) {
        Ok(matches) => Json(json!(matches)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandClassifyUpdateQuery {
    pub project: Option<String>,
}

pub async fn understand_classify_update(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandClassifyUpdateQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    match ChangeClassifier::classify_changes(&project_dir, Some(&db_path_str)) {
        Ok(decision) => Json(json!(decision)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandConceptsQuery {
    pub project: Option<String>,
    pub limit: Option<usize>,
}

pub async fn understand_concepts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandConceptsQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();
    let limit = params.limit.unwrap_or(200);

    match crate::services::language_lesson::detect_project_concepts(&db_path_str, limit) {
        Ok(report) => Json(json!(report)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandFreshnessQuery {
    pub project: Option<String>,
}

pub async fn understand_freshness(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandFreshnessQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));

    let res =
        crate::domains::knowledge_graph::graph_freshness::GraphFreshnessDetector::check_freshness(
            &db_path,
            &project_dir,
        );
    Json(json!(res)).into_response()
}

#[derive(Deserialize)]
pub struct UnderstandComplexityQuery {
    pub project: Option<String>,
    pub limit: Option<usize>,
}

pub async fn understand_complexity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandComplexityQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing project param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();
    let limit = params.limit.unwrap_or(20);

    match crate::domains::code_analysis::code_complexity::ComplexityAnalyzer::analyze_project_complexity(
        &db_path_str,
        limit,
    ) {
        Ok(report) => Json(json!(report)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandSimilarityPayload {
    pub code_a: String,
    pub code_b: String,
}

pub async fn understand_similarity(
    Json(payload): Json<UnderstandSimilarityPayload>,
) -> Response {
    Json(CodeSimilarityDetector::compare_and_classify(
        &payload.code_a,
        &payload.code_b,
    ))
    .into_response()
}

pub async fn understand_projects(State(state): State<Arc<AppState>>) -> Response {
    let root = &state.runtime_config.projects_root;
    let projects =
        crate::domains::project_management::project_manager::ProjectIndexManager::discover_projects(
            root,
        );
    Json(json!(projects)).into_response()
}

#[derive(Deserialize)]
pub struct RepoMapQuery {
    pub project: Option<String>,
    pub focus_path: Option<String>,
    pub query: Option<String>,
    pub max_symbols: Option<usize>,
}

pub async fn understand_repo_map(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RepoMapQuery>,
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
    if !db_path.exists() {
        return Json(json!({
            "repo_map": "",
            "message": "Database not found. Please index workspace first.",
            "symbols_count": 0,
            "files_count": 0
        }))
        .into_response();
    }

    let generator = crate::domains::version_control::repo_map::RepoMapEngine::new(db_path);
    let max_syms = params.max_symbols.unwrap_or(60);
    match generator.generate_map(
        params.focus_path.as_deref(),
        params.query.as_deref(),
        max_syms,
    ) {
        Ok(repo_map) => {
            let files_count = repo_map
                .lines()
                .filter(|l| !l.starts_with("  ") && !l.trim().is_empty())
                .count();
            let symbols_count = repo_map
                .lines()
                .filter(|l| l.starts_with("  ") && !l.trim().is_empty())
                .count();
            Json(json!({
                "repo_map": repo_map,
                "files_count": files_count,
                "symbols_count": symbols_count,
                "project": project,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}
