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

use std::sync::Arc;
use axum::{Router, routing::get, extract::{State, Query}, Json};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::AppState;
use crate::services::{understand_service, file_service, token_filter, claude_cli};
use crate::services::token_filter::FilterLevel;
use crate::services::llm_provider::{self, LlmProvider};
use crate::services::understanding_cache::UnderstandingCache;

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
}

/// File-level understanding: symbols, dependencies, inbound references, and an
/// optional LLM comprehension narrative.
async fn understand_file(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UnderstandFileQuery>,
) -> Response {
    let project = match params.project {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };
    let path = match params.path {
        Some(p) if !p.trim().is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing path param" }))).into_response(),
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    let mut result = match understand_service::understand_file(&db_path_str, &path) {
        Ok(result) => result,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    };

    // Optional LLM comprehension layer (closes the "理解分析" loop).
    if params.explain.unwrap_or(false) {
        let lang = params.lang.unwrap_or_else(|| "zh-CN".to_string());
        let refresh = params.refresh.unwrap_or(false);
        let cache_only = params.cache_only.unwrap_or(false);
        let use_claude = is_claude_engine(&params.engine);
        // Scope cache keys per engine so Claude and LLM outputs never collide.
        let cache_lang = if use_claude { format!("claude:{}", lang) } else { lang.clone() };
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
                let compressed = token_filter::compress(content, &path, FilterLevel::Aggressive);
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
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing project param" }))).into_response(),
    };

    let db_path = state.runtime_config.codebase_memory_db_path(Some(&project));
    let db_path_str = db_path.to_string_lossy().to_string();

    let mut result = match understand_service::understand_project(&db_path_str) {
        Ok(result) => result,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    };

    if params.explain.unwrap_or(false) {
        let lang = params.lang.unwrap_or_else(|| "zh-CN".to_string());
        let refresh = params.refresh.unwrap_or(false);
        let cache_only = params.cache_only.unwrap_or(false);
        let use_claude = is_claude_engine(&params.engine);
        // Scope cache keys per engine so Claude and LLM outputs never collide.
        let cache_lang = if use_claude { format!("claude:{}", lang) } else { lang.clone() };

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
        let project_language = understand_service::detect_primary_language(&db_path_str).unwrap_or_else(|_| "unknown".to_string());
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
                llm.explain_project(&structure, &lang, &project_language).await
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
    let (directive, _) = llm_provider::lang_directive(lang);
    format!(
        "\u{4f60}\u{662f}\u{8d44}\u{6df1}\u{8f6f}\u{4ef6}\u{67b6}\u{6784}\u{5206}\u{6790}\u{5e08}\u{3002}\u{8bf7}\u{76f4}\u{63a5}\u{57fa}\u{4e8e}\u{4e0b}\u{9762}\u{63d0}\u{4f9b}\u{7684}\u{7edf}\u{8ba1}\u{4fe1}\u{606f}\u{56de}\u{7b54}\u{ff0c}\u{4e0d}\u{8981}\u{8bfb}\u{53d6}\u{6587}\u{4ef6}\u{6216}\u{8c03}\u{7528}\u{4efb}\u{4f55}\u{5de5}\u{5177}\u{3002}\u{8f93}\u{51fa}\u{7b80}\u{6d01}\u{7684} Markdown\u{ff0c}\u{5305}\u{542b}\u{ff1a}\n1) \u{9879}\u{76ee}\u{6982}\u{8ff0}\u{ff1b}\n2) \u{4e3b}\u{8981}\u{6a21}\u{5757}\u{ff1b}\n3) \u{6280}\u{672f}\u{6808}\u{ff1b}\n4) \u{6838}\u{5fc3}\u{7b26}\u{53f7}\u{3002}\n{}\n\n\u{9879}\u{76ee}\u{7edf}\u{8ba1}(\u{6765}\u{81ea}\u{4ee3}\u{7801}\u{56fe}\u{8c31})\u{ff1a}\n{}",
        directive, structure
    )
}

/// Build a single self-contained prompt for a file-level comprehension,
/// mirroring `LlmProvider::explain_file` for `claude -p`.
fn build_file_prompt(file_path: &str, skeleton: &str, structure: &str, lang: &str) -> String {
    let (directive, _) = llm_provider::lang_directive(lang);
    let skeleton_block = if skeleton.trim().is_empty() {
        String::from("(\u{6e90}\u{7801}\u{4e0d}\u{53ef}\u{7528}\u{ff0c}\u{4ec5}\u{4f9d}\u{636e}\u{4ee3}\u{7801}\u{56fe}\u{8c31}\u{7ed3}\u{6784}\u{4fe1}\u{606f}\u{5206}\u{6790})")
    } else {
        format!("\u{4ee3}\u{7801}\u{9aa8}\u{67b6}\u{ff1a}\n```\n{}\n```", skeleton)
    };
    format!(
        "\u{4f60}\u{662f}\u{8d44}\u{6df1}\u{4ee3}\u{7801}\u{5206}\u{6790}\u{52a9}\u{624b}\u{3002}\u{8bf7}\u{76f4}\u{63a5}\u{57fa}\u{4e8e}\u{4e0b}\u{9762}\u{63d0}\u{4f9b}\u{7684}\u{4fe1}\u{606f}\u{56de}\u{7b54}\u{ff0c}\u{4e0d}\u{8981}\u{8bfb}\u{53d6}\u{5176}\u{4ed6}\u{6587}\u{4ef6}\u{6216}\u{8c03}\u{7528}\u{4efb}\u{4f55}\u{5de5}\u{5177}\u{3002}\u{8f93}\u{51fa}\u{7b80}\u{6d01}\u{7684} Markdown\u{ff0c}\u{5305}\u{542b}\u{4e09}\u{90e8}\u{5206}\u{ff1a}\n1) \u{6587}\u{4ef6}\u{804c}\u{8d23}\u{ff1b}\n2) \u{5173}\u{952e}\u{7b26}\u{53f7}\u{ff1b}\n3) \u{4f9d}\u{8d56}\u{5173}\u{7cfb}\u{3002}\n{}\n\n\u{6587}\u{4ef6}\u{ff1a}{}\n\n\u{7ed3}\u{6784}\u{4fe1}\u{606f}(\u{6765}\u{81ea}\u{4ee3}\u{7801}\u{56fe}\u{8c31})\u{ff1a}\n{}\n\n{}",
        directive, file_path, structure, skeleton_block
    )
}
