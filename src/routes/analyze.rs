use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/analyze/architecture", post(analyze_architecture))
        .route("/api/analyze/domain", post(analyze_domain))
        .route("/api/analyze/impact", post(analyze_impact))
        .route("/api/analyze/tour", post(analyze_tour))
        .route("/api/analyze/wiki", post(analyze_wiki))
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

    let llm = crate::domains::model_gateway::providers::LlmProvider::new(state.http_client.clone());
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

    let llm = crate::domains::model_gateway::providers::LlmProvider::new(state.http_client.clone());
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

    let llm = crate::domains::model_gateway::providers::LlmProvider::new(state.http_client.clone());
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

#[axum::debug_handler]
async fn analyze_wiki(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    let (project, db_path_str) = match get_db_path_and_project(&state, &body).await {
        Ok(res) => res,
        Err(err) => return err,
    };

    let project_overview = crate::services::understand_service::understand_project(&db_path_str)
        .unwrap_or_else(|error| json!({ "indexed": false, "error": error }));
    let indexed = project_overview
        .get("indexed")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let files = collect_top_files(&db_path_str, 24).unwrap_or_default();
    let symbols = collect_top_symbols(&db_path_str, 24).unwrap_or_default();
    let documents = collect_knowledge_documents(&state, &project);
    let git_status = crate::services::git_service::get_status(&project).ok();
    let ai_history = crate::services::git_service::get_ai_history(&project, 12).unwrap_or_default();
    let changed_files = git_status
        .as_ref()
        .map(|status| {
            json!({
                "modified": status.modified,
                "added": status.added,
                "deleted": status.deleted,
                "untracked": status.untracked
            })
        })
        .unwrap_or_else(|| json!({}));

    let trace = json!({
        "project": project,
        "graphDb": db_path_str,
        "indexed": indexed,
        "overview": project_overview,
        "files": files,
        "symbols": symbols,
        "documents": documents,
        "git": {
            "status": git_status,
            "aiHistory": ai_history,
            "changedFiles": changed_files
        }
    });

    let lang = body.lang.as_deref().unwrap_or("zh-CN");
    let wiki = generate_wiki_markdown(&state, &project, &trace, lang)
        .await
        .unwrap_or_else(|error| fallback_wiki_markdown(&project, &trace, &error));

    persist_wiki_memory(&state, &project, &db_path_str, &wiki);

    Json(json!({
        "success": true,
        "project": project,
        "indexed": indexed,
        "wiki": wiki,
        "trace": trace,
        "generatedAt": chrono::Utc::now().to_rfc3339()
    }))
    .into_response()
}

fn collect_top_files(db_path: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
    if !std::path::Path::new(db_path).exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT n.file_path, COUNT(e.source_id) + COUNT(e.target_id) AS refs \
             FROM nodes n \
             LEFT JOIN edges e ON e.source_id = n.id OR e.target_id = n.id \
             WHERE n.label = 'File' AND n.file_path <> '' \
               AND n.file_path NOT LIKE '%/node_modules/%' \
               AND n.file_path NOT LIKE '%/target/%' \
               AND n.file_path NOT LIKE '%/dist/%' \
             GROUP BY n.file_path \
             ORDER BY refs DESC, n.file_path ASC \
             LIMIT ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
            Ok(json!({
                "path": row.get::<_, String>(0)?,
                "references": row.get::<_, i64>(1).unwrap_or(0)
            }))
        })
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

fn collect_top_symbols(db_path: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
    if !std::path::Path::new(db_path).exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT n.name, n.label, n.file_path, COUNT(e.source_id) AS refs \
             FROM nodes n \
             LEFT JOIN edges e ON e.target_id = n.id AND e.type IN ('CALLS','USAGE','IMPORTS') \
             WHERE n.label IN ('Function','Method','Class','Struct','Interface','Trait') \
               AND LENGTH(n.name) >= 3 \
               AND n.file_path NOT LIKE '%/node_modules/%' \
               AND n.file_path NOT LIKE '%/target/%' \
               AND n.file_path NOT LIKE '%/dist/%' \
             GROUP BY n.id \
             ORDER BY refs DESC, n.name ASC \
             LIMIT ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
            Ok(json!({
                "name": row.get::<_, String>(0)?,
                "type": row.get::<_, String>(1)?,
                "filePath": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                "references": row.get::<_, i64>(3).unwrap_or(0)
            }))
        })
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

fn collect_knowledge_documents(state: &Arc<AppState>, project: &str) -> Vec<serde_json::Value> {
    let mut documents = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for store in [
        crate::services::supermemory_store::SupermemoryStore::new(
            &state.runtime_config,
            Some(project),
        ),
        crate::services::supermemory_store::SupermemoryStore::new(&state.runtime_config, None),
    ] {
        if let Ok(items) = store.knowledge_documents() {
            for document in items {
                if !seen.insert(document.document_id.clone()) {
                    continue;
                }
                documents.push(json!({
                    "documentId": document.document_id,
                    "sourcePath": document.source_path,
                    "displayName": document.display_name,
                    "contentHash": document.content_hash,
                    "revision": document.revision,
                    "format": document.format,
                    "chunkCount": document.chunks.len(),
                    "updatedAt": document.updated_at,
                    "citations": document.chunks.iter().take(5).map(|chunk| chunk.citation.clone()).collect::<Vec<_>>()
                }));
            }
        }
    }
    documents
}

async fn generate_wiki_markdown(
    state: &Arc<AppState>,
    project: &str,
    trace: &serde_json::Value,
    lang: &str,
) -> Result<String, String> {
    let llm = crate::domains::model_gateway::providers::LlmProvider::new(state.http_client.clone());
    if !llm.is_available() {
        return Err("LLM not configured".to_string());
    }
    let compact_trace = serde_json::to_string_pretty(trace)
        .unwrap_or_default()
        .chars()
        .take(18_000)
        .collect::<String>();
    let (directive, _) = crate::domains::model_gateway::providers::lang_directive(lang);
    let messages = vec![json!({
        "role": "user",
        "content": format!(
            "请基于下面的项目图谱、知识文档和 Git 链路生成一份项目 Wiki 总览。输出 Markdown，必须包含：\n1) 项目定位；\n2) 架构分层；\n3) 关键代码文件；\n4) 关键符号/入口；\n5) 关联知识文档；\n6) 当前变更与 AI 快照链路；\n7) 下一步维护建议。\n不要编造未提供的文件路径或 commit。{}\n\n项目：{}\n\n统一溯源数据：\n```json\n{}\n```",
            directive, project, compact_trace
        )
    })];
    llm.chat(
        messages,
        Some("你是资深代码知识库架构师，擅长把代码图谱、文档语料和 Git 变更历史编成可维护的项目 Wiki。"),
    )
    .await
    .map(|(content, _)| content)
}

fn fallback_wiki_markdown(project: &str, trace: &serde_json::Value, reason: &str) -> String {
    let file_count = trace
        .get("files")
        .and_then(|value| value.as_array())
        .map_or(0, Vec::len);
    let symbol_count = trace
        .get("symbols")
        .and_then(|value| value.as_array())
        .map_or(0, Vec::len);
    let document_count = trace
        .get("documents")
        .and_then(|value| value.as_array())
        .map_or(0, Vec::len);
    let snapshot_count = trace
        .pointer("/git/aiHistory")
        .and_then(|value| value.as_array())
        .map_or(0, Vec::len);
    format!(
        "# Project Wiki\n\n> LLM Wiki generation skipped: {reason}\n\n## Project\n\n`{project}`\n\n## Linked Evidence\n\n- Indexed files surfaced: {file_count}\n- Linked symbols surfaced: {symbol_count}\n- Knowledge documents surfaced: {document_count}\n- AI history snapshots surfaced: {snapshot_count}\n"
    )
}

fn persist_wiki_memory(state: &Arc<AppState>, project: &str, db_path: &str, wiki: &str) {
    let store = crate::services::supermemory_store::SupermemoryStore::new(
        &state.runtime_config,
        Some(project),
    );
    store.write_memory(crate::models::memory::SupermemoryRecord {
        id: "project-wiki-latest".to_string(),
        file_path: project.to_string(),
        name: "Project Wiki".to_string(),
        summary: wiki.chars().take(12_000).collect(),
        tags: vec![
            "project_wiki".to_string(),
            "auto-generated".to_string(),
            "code-graph".to_string(),
            "git-linked".to_string(),
        ],
        complexity: "wiki_overview".to_string(),
        node_type: "project_wiki".to_string(),
        source_graph_path: db_path.to_string(),
        source_label: "POST /api/analyze/wiki".to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        embedding: None,
    });
}
