//! Understanding endpoints: file-level and project-level comprehension
//! backed by the codebase-memory-mcp graph.
//!
//! Two layers: structural facts always come from the graph DB; when the caller
//! passes `explain=true` and an LLM is configured, we synthesize a
//! natural-language comprehension on top. File source is compressed to its
//! structural skeleton (token-killer) before hitting the LLM to keep tokens low.
//!
//! LLM comprehensions are cached on disk (see `understanding_cache`): keys
//! embed the output language and a content signature, so repeated requests do
//! not burn tokens. `refresh=true` bypasses the cache; `cacheOnly=true` never
//! calls the LLM (used by the frontend to probe for a pre-generated overview).

use crate::domains::model_gateway::providers::{self, LlmProvider};
use crate::domains::project_management::adr_service::AdrService;
use crate::domains::version_control::change_classifier::ChangeClassifier;
use crate::domains::code_analysis::code_intelligence::CodeIntelligenceRuntime;
use crate::domains::version_control::diff_impact::DiffImpactAnalyzer;
use crate::domains::project_management::onboard_service::OnboardService;
use crate::domains::token_compression::FilterLevel;
use crate::domains::knowledge_graph::understanding_cache::UnderstandingCache;
use crate::services::{claude_cli, file_service};
use crate::domains::token_compression;
use crate::domains::knowledge_graph::understand_service;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

/// Timeout for a synchronous Claude understanding run. Prompts are
/// self-contained (graph structure text) and instruct the model not to use
/// tools, so this stays well under a full agentic-run budget.
const CLAUDE_TIMEOUT_SECS: u64 = 240;

/// The caller can pick the comprehension engine via `engine=claude`; anything
/// else (including absent) uses the configured OpenAI-compatible LLM.
fn is_claude_engine(engine: &Option<String>) -> bool {
    engine
        .as_deref()
        .map(|e| e.eq_ignore_ascii_case("claude"))
        .unwrap_or(false)
}

#[derive(Deserialize)]
pub struct UnderstandFileQuery {
    project: Option<String>,
    path: Option<String>,
    /// When true, synthesize an LLM natural-language comprehension on top of
    /// the structural facts. Defaults to false to avoid spending tokens.
    explain: Option<bool>,
    /// Output language for the LLM comprehension (e.g. `zh-CN` / `en-US`).
    lang: Option<String>,
    /// Bypass the cache and force a fresh LLM generation.
    refresh: Option<bool>,
    /// Only serve from cache; never call the LLM.
    #[serde(rename = "cacheOnly")]
    cache_only: Option<bool>,
    /// Comprehension engine: `claude` routes through the local Claude CLI.
    engine: Option<String>,
}

#[derive(Deserialize)]
pub struct UnderstandProjectQuery {
    project: Option<String>,
    explain: Option<bool>,
    lang: Option<String>,
    refresh: Option<bool>,
    #[serde(rename = "cacheOnly")]
    cache_only: Option<bool>,
    /// Comprehension engine: `claude` routes through the local Claude CLI.
    engine: Option<String>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/understand/file", get(understand_file))
        .route("/api/understand/project", get(understand_project))
        .route("/api/understand/onboard", get(understand_onboard))
        .route("/api/understand/diff", get(understand_diff))
        .route("/api/understand/trace", get(understand_trace))
        .route("/api/understand/deadcode", get(understand_deadcode))
        .route("/api/understand/search", get(understand_search))
        .route("/api/understand/search_code", get(understand_search_code))
        .route("/api/understand/classify_update", get(understand_classify_update))
        .route("/api/understand/concepts", get(understand_concepts))
        .route("/api/understand/freshness", get(understand_freshness))
        .route("/api/understand/complexity", get(understand_complexity))
        .route("/api/understand/similarity", axum::routing::post(understand_similarity))
        .route("/api/understand/projects", get(understand_projects))
        .route("/api/understand/adr", get(list_or_get_adr).post(create_adr_endpoint))
        .route("/api/understand/repo_map", get(understand_repo_map))
}

/// File-level understanding: symbols, dependencies, inbound references, and an
/// optional LLM comprehension narrative.
async fn understand_file(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandFileQuery>,
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
    let path = match params.path {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing path param" })),
            )
                .into_response()
        }
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    let mut result = match understand_service::understand_file(&db_path_str, &path) {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e })),
            )
                .into_response()
        }
    };

    // Optional LLM comprehension layer (closes the "理解分析" loop).
    if params.explain.unwrap_or(false) {
        let lang = params.lang.unwrap_or_else(|| "zh-CN".to_string());
        let refresh = params.refresh.unwrap_or(false);
        let cache_only = params.cache_only.unwrap_or(false);
        let use_claude = is_claude_engine(&params.engine);
        // Scope cache keys per engine so Claude and LLM outputs never collide.
        let cache_lang = if use_claude {
            format!("claude:{}", lang)
        } else {
            lang.clone()
        };
        let structure = understand_service::summarize_file_structure(&result);
        let source = file_service::read_file_content(&project, &path).ok();

        // Cache key: source content hash when available, otherwise the graph
        // structure signature (still invalidates when the index changes).
        let cache = UnderstandingCache::for_db(&state.app_paths, &db_path_str);
        let key = match &source {
            Some(content) => UnderstandingCache::file_key(&cache_lang, &path, content),
            None => UnderstandingCache::file_key(&cache_lang, &path, &structure),
        };

        if !refresh {
            if let Some(hit) = cache.get(&key) {
                result["understanding"] = hit.get("understanding").cloned().unwrap_or(Value::Null);
                if let Some(c) = hit.get("compression").filter(|c| !c.is_null()) {
                    result["compression"] = c.clone();
                }
                result["cached"] = json!(true);
                return Json(result).into_response();
            }
        }
        if cache_only {
            result["understanding"] = Value::Null;
            result["cached"] = json!(false);
            result["cacheMiss"] = json!(true);
            return Json(result).into_response();
        }

        // Build the compressed skeleton (+ compression stats) once; both engines
        // consume the same token-frugal input.
        let (skeleton, compression) = match &source {
            Some(content) => {
                let compressed = token_compression::compress(content, &path, FilterLevel::Aggressive);
                let stats = json!({
                    "level": compressed.level,
                    "language": compressed.language,
                    "originalBytes": compressed.original_bytes,
                    "compressedBytes": compressed.compressed_bytes,
                    "reductionPercent": compressed.reduction_percent,
                    "originalTokensEst": compressed.original_tokens_est,
                    "compressedTokensEst": compressed.compressed_tokens_est,
                });
                (compressed.content, Some(stats))
            }
            None => (String::new(), None),
        };

        // Source not on disk and file not indexed: nothing to explain.
        if skeleton.trim().is_empty() && structure.trim().is_empty() {
            result["understanding"] = Value::Null;
            result["understandingError"] = json!("Source unavailable and file not indexed");
            return Json(result).into_response();
        }

        let explanation = if use_claude {
            if claude_cli::detect_version().await.is_none() {
                Err("Claude CLI not installed".to_string())
            } else {
                let prompt = build_file_prompt(&path, &skeleton, &structure, &lang);
                claude_cli::run_and_capture(
                    Some(&project),
                    &prompt,
                    CLAUDE_TIMEOUT_SECS,
                    Some(&state.runtime_config.rtk_bin),
                )
                .await
            }
        } else {
            let llm = LlmProvider::new(state.http_client.clone());
            if !llm.is_available() {
                Err("LLM not configured".to_string())
            } else {
                llm.explain_file(&path, &skeleton, &structure, &lang).await
            }
        };

        match explanation {
            Ok(text) => {
                let cache_value = match &compression {
                    Some(stats) => json!({ "understanding": text, "compression": stats }),
                    None => json!({ "understanding": text, "compression": Value::Null }),
                };
                cache.put(&key, cache_value);
                result["understanding"] = json!(text);
                if let Some(stats) = compression {
                    result["compression"] = stats;
                }
                result["cached"] = json!(false);
            }
            Err(e) => {
                result["understanding"] = Value::Null;
                result["understandingError"] = json!(e);
            }
        }
    }

    Json(result).into_response()
}

/// Project-level understanding: summary, distributions, modules, hot symbols,
/// and an optional LLM architecture overview.
async fn understand_project(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandProjectQuery>,
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

    let mut result = match understand_service::understand_project(&db_path_str) {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e })),
            )
                .into_response()
        }
    };

    if params.explain.unwrap_or(false) {
        let lang = params.lang.unwrap_or_else(|| "zh-CN".to_string());
        let refresh = params.refresh.unwrap_or(false);
        let cache_only = params.cache_only.unwrap_or(false);
        let use_claude = is_claude_engine(&params.engine);
        // Scope cache keys per engine so Claude and LLM outputs never collide.
        let cache_lang = if use_claude {
            format!("claude:{}", lang)
        } else {
            lang.clone()
        };

        if result.get("indexed").and_then(|v| v.as_bool()) != Some(true) {
            result["understanding"] = Value::Null;
            result["understandingError"] = json!("Project not indexed");
            return Json(result).into_response();
        }

        // Project key: DB mtime + node count, so a re-index invalidates it.
        let nodes = result
            .pointer("/totals/nodes")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache = UnderstandingCache::for_db(&state.app_paths, &db_path_str);
        let key = UnderstandingCache::project_key(&cache_lang, &db_path_str, nodes);

        if !refresh {
            if let Some(hit) = cache.get(&key) {
                result["understanding"] = hit.get("understanding").cloned().unwrap_or(Value::Null);
                result["cached"] = json!(true);
                return Json(result).into_response();
            }
        }
        if cache_only {
            result["understanding"] = Value::Null;
            result["cached"] = json!(false);
            result["cacheMiss"] = json!(true);
            return Json(result).into_response();
        }

        let structure = understand_service::summarize_project_structure(&result);
        let project_language = understand_service::detect_primary_language(&db_path_str)
            .unwrap_or_else(|_| "unknown".to_string());
        let explanation = if use_claude {
            if claude_cli::detect_version().await.is_none() {
                Err("Claude CLI not installed".to_string())
            } else {
                let prompt = build_project_prompt(&structure, &lang);
                claude_cli::run_and_capture(
                    Some(&project),
                    &prompt,
                    CLAUDE_TIMEOUT_SECS,
                    Some(&state.runtime_config.rtk_bin),
                )
                .await
            }
        } else {
            let llm = LlmProvider::new(state.http_client.clone());
            if !llm.is_available() {
                Err("LLM not configured".to_string())
            } else {
                llm.explain_project(&structure, &lang, &project_language)
                    .await
            }
        };
        match explanation {
            Ok(text) => {
                cache.put(&key, json!({ "understanding": text }));
                result["understanding"] = json!(text);
                result["cached"] = json!(false);
            }
            Err(e) => {
                result["understanding"] = Value::Null;
                result["understandingError"] = json!(e);
            }
        }
    }

    Json(result).into_response()
}

/// Build a single self-contained prompt for a project-level overview, mirroring
/// `LlmProvider::explain_project` but folded into one string for `claude -p`.
/// The model is told to answer from the supplied stats only, so the run stays
/// a fast single-shot completion rather than an agentic exploration.
fn build_project_prompt(structure: &str, lang: &str) -> String {
    let (directive, _) = crate::domains::model_gateway::providers::lang_directive(lang);
    format!(
        "\u{4f60}\u{662f}\u{8d44}\u{6df1}\u{8f6f}\u{4ef6}\u{67b6}\u{6784}\u{5206}\u{6790}\u{5e08}\u{3002}\u{8bf7}\u{76f4}\u{63a5}\u{57fa}\u{4e8e}\u{4e0b}\u{9762}\u{63d0}\u{4f9b}\u{7684}\u{7edf}\u{8ba1}\u{4fe1}\u{606f}\u{56de}\u{7b54}\u{ff0c}\u{4e0d}\u{8981}\u{8bfb}\u{53d6}\u{6587}\u{4ef6}\u{6216}\u{8c03}\u{7528}\u{4efb}\u{4f55}\u{5de5}\u{5177}\u{3002}\u{8f93}\u{51fa}\u{7b80}\u{6d01}\u{7684} Markdown\u{ff0c}\u{5305}\u{542b}\u{ff1a}\n1) \u{9879}\u{76ee}\u{6982}\u{8ff0}\u{ff1b}\n2) \u{4e3b}\u{8981}\u{6a21}\u{5757}\u{ff1b}\n3) \u{6280}\u{672f}\u{6808}\u{ff1b}\n4) \u{6838}\u{5fc3}\u{7b26}\u{53f7}\u{3002}\n{}\n\n\u{9879}\u{76ee}\u{7edf}\u{8ba1}(\u{6765}\u{81ea}\u{4ee3}\u{7801}\u{56fe}\u{8c31})\u{ff1a}\n{}",
        directive, structure
    )
}

/// Build a single self-contained prompt for a file-level comprehension,
/// mirroring `LlmProvider::explain_file` for `claude -p`.
fn build_file_prompt(file_path: &str, skeleton: &str, structure: &str, lang: &str) -> String {
    let (directive, _) = crate::domains::model_gateway::providers::lang_directive(lang);
    let skeleton_block = if skeleton.trim().is_empty() {
        String::from("(\u{6e90}\u{7801}\u{4e0d}\u{53ef}\u{7528}\u{ff0c}\u{4ec5}\u{4f9d}\u{636e}\u{4ee3}\u{7801}\u{56fe}\u{8c31}\u{7ed3}\u{6784}\u{4fe1}\u{606f}\u{5206}\u{6790})")
    } else {
        format!(
            "\u{4ee3}\u{7801}\u{9aa8}\u{67b6}\u{ff1a}\n```\n{}\n```",
            skeleton
        )
    };
    format!(
        "\u{4f60}\u{662f}\u{8d44}\u{6df1}\u{4ee3}\u{7801}\u{5206}\u{6790}\u{52a9}\u{624b}\u{3002}\u{8bf7}\u{76f4}\u{63a5}\u{57fa}\u{4e8e}\u{4e0b}\u{9762}\u{63d0}\u{4f9b}\u{7684}\u{4fe1}\u{606f}\u{56de}\u{7b54}\u{ff0c}\u{4e0d}\u{8981}\u{8bfb}\u{53d6}\u{5176}\u{4ed6}\u{6587}\u{4ef6}\u{6216}\u{8c03}\u{7528}\u{4efb}\u{4f55}\u{5de5}\u{5177}\u{3002}\u{8f93}\u{51fa}\u{7b80}\u{6d01}\u{7684} Markdown\u{ff0c}\u{5305}\u{542b}\u{4e09}\u{90e8}\u{5206}\u{ff1a}\n1) \u{6587}\u{4ef6}\u{804c}\u{8d23}\u{ff1b}\n2) \u{5173}\u{952e}\u{7b26}\u{53f7}\u{ff1b}\n3) \u{4f9d}\u{8d56}\u{5173}\u{7cfb}\u{3002}\n{}\n\n\u{6587}\u{4ef6}\u{ff1a}{}\n\n\u{7ed3}\u{6784}\u{4fe1}\u{606f}(\u{6765}\u{81ea}\u{4ee3}\u{7801}\u{56fe}\u{8c31})\u{ff1a}\n{}\n\n{}",
        directive, file_path, structure, skeleton_block
    )
}

fn resolve_project_path(state: &AppState, project: &str) -> PathBuf {
    let p = std::path::Path::new(project);
    if p.is_absolute() && p.exists() {
        p.to_path_buf()
    } else {
        let joined = state.runtime_config.workspace_root.join(project);
        if joined.exists() {
            joined
        } else {
            p.to_path_buf()
        }
    }
}

#[derive(Deserialize)]
pub struct UnderstandOnboardQuery {
    pub project: Option<String>,
}

async fn understand_onboard(
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

async fn understand_diff(
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

async fn understand_trace(
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

async fn understand_deadcode(
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

async fn list_or_get_adr(
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
            Err(e) => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": e })),
            )
                .into_response(),
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

async fn create_adr_endpoint(
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

async fn understand_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandSearchQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let rt = CodeIntelligenceRuntime::new(db_path);

    let query_term = params.query.as_deref().or(params.name_pattern.as_deref());
    let label = params.label.as_deref();
    let file_pattern = params.file_pattern.as_deref();
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    match rt.search_graph(label, query_term, file_pattern, params.min_degree, params.max_degree, limit, offset) {
        Ok(results) => Json(json!(results)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
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

async fn understand_search_code(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandSearchCodeQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };
    let query_str = match params.query {
        Some(q) if !q.trim().is_empty() => q,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing query param" }))).into_response(),
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let rt = CodeIntelligenceRuntime::new(db_path);
    let limit = params.limit.unwrap_or(50);

    match rt.search_code(&query_str, params.file_pattern.as_deref(), Some(&project_dir), limit) {
        Ok(matches) => Json(json!(matches)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandClassifyUpdateQuery {
    pub project: Option<String>,
}

async fn understand_classify_update(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandClassifyUpdateQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    match ChangeClassifier::classify_changes(&project_dir, Some(&db_path_str)) {
        Ok(decision) => Json(json!(decision)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandConceptsQuery {
    pub project: Option<String>,
    pub limit: Option<usize>,
}

async fn understand_concepts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandConceptsQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();
    let limit = params.limit.unwrap_or(200);

    match crate::services::language_lesson::detect_project_concepts(&db_path_str, limit) {
        Ok(report) => Json(json!(report)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandFreshnessQuery {
    pub project: Option<String>,
}

async fn understand_freshness(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandFreshnessQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };

    let project_dir = resolve_project_path(&state, &project);
    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));

    let res = crate::domains::knowledge_graph::graph_freshness::GraphFreshnessDetector::check_freshness(
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

async fn understand_complexity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandComplexityQuery>,
) -> Response {
    let project = match params.project {
        Some(p) if !p.trim().is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();
    let limit = params.limit.unwrap_or(20);

    match crate::domains::code_analysis::code_complexity::ComplexityAnalyzer::analyze_project_complexity(&db_path_str, limit) {
        Ok(report) => Json(json!(report)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnderstandSimilarityPayload {
    pub code_a: String,
    pub code_b: String,
}

async fn understand_similarity(
    Json(payload): Json<UnderstandSimilarityPayload>,
) -> Response {
    let sim = crate::domains::code_analysis::code_similarity::CodeSimilarityDetector::compare_snippets(
        &payload.code_a,
        &payload.code_b,
    );
    let clone_type = if sim >= 0.90 {
        "EXACT_OR_RENAME"
    } else if sim >= 0.70 {
        "HIGH_SIMILARITY"
    } else if sim >= 0.30 {
        "STRUCTURAL_SIMILARITY"
    } else {
        "DISTINCT"
    };

    Json(json!({
        "similarity": (sim * 1000.0).round() / 1000.0,
        "cloneType": clone_type,
    })).into_response()
}

async fn understand_projects(
    State(state): State<Arc<AppState>>,
) -> Response {
    let root = &state.runtime_config.projects_root;
    let projects = crate::domains::project_management::project_manager::ProjectIndexManager::discover_projects(root);
    Json(json!(projects)).into_response()
}

#[derive(Deserialize)]
pub struct RepoMapQuery {
    project: Option<String>,
    focus_path: Option<String>,
    query: Option<String>,
    max_symbols: Option<usize>,
}

async fn understand_repo_map(
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
    match generator.generate_map(params.focus_path.as_deref(), params.query.as_deref(), max_syms) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_understand_router_construction() {
        // Ensure that all routes are correctly registered without runtime panics
        let state = crate::server::test_state();
        let app: Router = router().with_state(state);
        assert!(format!("{:?}", app).contains("Router"));
    }
}

