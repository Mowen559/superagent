//! 文件级与项目级结构提取及 AI 语义理解合成 (`/api/understand/file`, `/api/understand/project`)

use crate::domains::knowledge_graph::understand_service;
use crate::domains::knowledge_graph::understanding_cache::UnderstandingCache;
use crate::domains::model_gateway::providers::LlmProvider;
use crate::domains::token_compression::{self, FilterLevel};
use crate::services::{claude_cli, file_service};
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

const CLAUDE_TIMEOUT_SECS: u64 = 240;

fn is_claude_engine(engine: &Option<String>) -> bool {
    engine
        .as_deref()
        .map(|e| e.eq_ignore_ascii_case("claude"))
        .unwrap_or(false)
}

#[derive(Deserialize)]
pub struct UnderstandFileQuery {
    pub project: Option<String>,
    pub path: Option<String>,
    pub explain: Option<bool>,
    pub lang: Option<String>,
    pub refresh: Option<bool>,
    #[serde(rename = "cacheOnly")]
    pub cache_only: Option<bool>,
    pub engine: Option<String>,
}

#[derive(Deserialize)]
pub struct UnderstandProjectQuery {
    pub project: Option<String>,
    pub explain: Option<bool>,
    pub lang: Option<String>,
    pub refresh: Option<bool>,
    #[serde(rename = "cacheOnly")]
    pub cache_only: Option<bool>,
    pub engine: Option<String>,
}

pub async fn understand_file(
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

    if params.explain.unwrap_or(false) {
        let lang = params.lang.unwrap_or_else(|| "zh-CN".to_string());
        let refresh = params.refresh.unwrap_or(false);
        let cache_only = params.cache_only.unwrap_or(false);
        let use_claude = is_claude_engine(&params.engine);
        let cache_lang = if use_claude {
            format!("claude:{}", lang)
        } else {
            lang.clone()
        };
        let structure = understand_service::summarize_file_structure(&result);
        let source = file_service::read_file_content(&project, &path).ok();

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

        let (skeleton, compression) = match &source {
            Some(content) => {
                let compressed =
                    token_compression::compress(content, &path, FilterLevel::Aggressive);
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
            let llm = LlmProvider::new(state.http_client.clone()).with_purpose("understand:file");
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

pub async fn understand_project(
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
            let llm =
                LlmProvider::new(state.http_client.clone()).with_purpose("understand:project");
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

fn build_project_prompt(structure: &str, lang: &str) -> String {
    let (directive, _) = crate::domains::model_gateway::providers::lang_directive(lang);
    format!(
        "你是资深软件架构分析师。请直接基于下面提供的统计信息回答，不要读取文件或调用任何工具。输出简洁的 Markdown，包含：\n1) 项目概述；\n2) 主要模块；\n3) 技术栈；\n4) 核心符号。\n{}\n\n项目统计(来自代码图谱)：\n{}",
        directive, structure
    )
}

fn build_file_prompt(file_path: &str, skeleton: &str, structure: &str, lang: &str) -> String {
    let (directive, _) = crate::domains::model_gateway::providers::lang_directive(lang);
    let skeleton_block = if skeleton.trim().is_empty() {
        String::from("(源码不可用，仅依据代码图谱结构信息分析)")
    } else {
        format!("代码骨架：\n```\n{}\n```", skeleton)
    };
    format!(
        "你是资深代码分析助手。请直接基于下面提供的信息回答，不要读取其他文件或调用任何工具。输出简洁的 Markdown，包含三部分：\n1) 文件职责；\n2) 关键符号；\n3) 依赖关系。\n{}\n\n文件：{}\n\n结构信息(来自代码图谱)：\n{}\n\n{}",
        directive, file_path, structure, skeleton_block
    )
}
