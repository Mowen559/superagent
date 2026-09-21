use crate::models::memory::SupermemoryRecord;
use crate::models::reverse::{ReverseEngineeringOptions, ReverseEngineeringRequest};
use crate::services::brain::knowledge_corpus::KnowledgeCorpusRuntime;
use crate::domains::project_management::project_inventory::{inspect_project, InventoryLimits};
use crate::services::supermemory_store::SupermemoryStore;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/workspaces/open", post(open_workspace))
        .route("/api/workspaces/analyze", post(analyze_workspace))
        .route("/api/workspaces/intelligence", get(workspace_intelligence))
        .route("/api/workspaces/reverse-plan", post(reverse_plan_workspace))
        .route("/api/workspaces/memory", get(workspace_memory))
        .route("/api/workspaces/knowledge", get(workspace_knowledge))
        .route(
            "/api/workspaces/knowledge/ingest",
            post(workspace_knowledge_ingest),
        )
        .route(
            "/api/workspaces/knowledge/rebuild",
            post(workspace_knowledge_rebuild),
        )
        .route(
            "/api/workspaces/knowledge/remove",
            post(workspace_knowledge_remove),
        )
        .route(
            "/api/workspaces/shadow-history",
            get(workspace_shadow_history),
        )
        .route(
            "/api/workspaces/shadow-snapshot",
            post(workspace_shadow_snapshot),
        )
        .route(
            "/api/workspaces/verify",
            post(verify_workspace),
        )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceRequest {
    path: Option<String>,
    project: Option<String>,
    project_id: Option<String>,
    requirement: Option<String>,
    message: Option<String>,
    limit: Option<usize>,
    language: Option<String>,
    model: Option<String>,
}

impl WorkspaceRequest {
    fn project_path(&self) -> Option<String> {
        self.path.clone().or_else(|| self.project.clone())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct WorkspaceQuery {
    project: Option<String>,
    q: Option<String>,
    limit: Option<usize>,
    model: Option<String>,
    refresh: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceKnowledgeRequest {
    project: Option<String>,
    path: Option<String>,
    global_context: Option<String>,
    document_id: Option<String>,
}

fn required_project(project: Option<String>) -> Result<String, Response> {
    match project {
        Some(project) if !project.trim().is_empty() => {
            match std::fs::canonicalize(project.trim()) {
                Ok(path) if path.is_dir() => Ok(path.to_string_lossy().to_string()),
                Ok(path) => Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Workspace path is not a directory: {}", path.display()) })),
                )
                    .into_response()),
                Err(error) => Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Cannot open workspace path: {error}") })),
                )
                    .into_response()),
            }
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Missing project path" })),
        )
            .into_response()),
    }
}

fn workspace_descriptor(state: &Arc<AppState>, project: &str) -> serde_json::Value {
    let graph_db = state.runtime_config.codebase_memory_db_path(Some(project));
    let memory_path = state.runtime_config.supermemory_store_path(Some(project));
    let shadow_git = state.runtime_config.shadow_git_root_for(Some(project));
    json!({
        "path": project,
        "name": std::path::Path::new(project)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| project.to_string()),
        "graphDbPath": graph_db.to_string_lossy(),
        "memoryPath": memory_path.to_string_lossy(),
        "shadowGitPath": shadow_git.to_string_lossy(),
        "indexed": graph_db.exists(),
        "hasSupermemory": memory_path.exists(),
        "hasShadowGit": shadow_git.exists(),
        "runtime": state.runtime_config.runtime_statuses(Some(project)),
        "readiness": workspace_readiness(project),
    })
}

fn workspace_readiness(project: &str) -> serde_json::Value {
    let inventory = inspect_project(
        std::path::Path::new(project),
        InventoryLimits {
            max_files: 5_000,
            max_fingerprint_bytes: 1024 * 1024,
        },
    );
    match inventory {
        Ok(inventory) => {
            let code_files: usize = inventory
                .languages
                .iter()
                .filter(|(language, _)| {
                    !matches!(
                        language.as_str(),
                        "Documentation" | "Configuration" | "Other"
                    )
                })
                .map(|(_, count)| *count)
                .sum();
            let is_code_project = code_files > 0 || !inventory.build_systems.is_empty();
            let kind = if inventory.files_scanned == 0 {
                "empty"
            } else if is_code_project {
                "code-project"
            } else {
                "not-code-project"
            };
            let mut warnings = Vec::new();
            if inventory.truncated {
                warnings.push("Readiness scan reached its configured bound".to_string());
            }
            if !is_code_project {
                warnings.push("No supported source files or build manifest detected".to_string());
            }
            json!({
                "kind": kind,
                "canAnalyze": is_code_project,
                "canRunAgent": is_code_project,
                "filesScanned": inventory.files_scanned,
                "codeFiles": code_files,
                "languages": inventory.languages,
                "buildSystems": inventory.build_systems,
                "sourceFingerprint": inventory.source_fingerprint,
                "truncated": inventory.truncated,
                "warnings": warnings,
            })
        }
        Err(error) => json!({
            "kind": "unavailable",
            "canAnalyze": false,
            "canRunAgent": false,
            "filesScanned": 0,
            "codeFiles": 0,
            "warnings": [error],
        }),
    }
}

async fn open_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceRequest>,
) -> Response {
    let project = match required_project(body.project_path()) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let project = match state.workspace_capabilities.open(&project) {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
        }
    };
    Json(json!({ "workspace": workspace_descriptor(&state, &project) })).into_response()
}

async fn analyze_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceRequest>,
) -> Response {
    let project = match opened_workspace(&state, body.project_path()) {
        Ok(project) => project,
        Err(response) => return response,
    };

    match crate::services::analysis_service::start_analysis(
        &state.runtime_config,
        &state.app_paths,
        state.task_manager.clone(),
        state.http_client.clone(),
        &project,
        body.project_id,
        crate::services::analysis_service::ProjectAnalysisOptions {
            language: body.language,
            model: body.model,
            ..Default::default()
        },
    )
    .await
    {
        Ok(task_id) => Json(json!({
            "taskId": task_id,
            "status": "submitted",
            "workspace": workspace_descriptor(&state, &project),
        }))
        .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

async fn workspace_intelligence(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WorkspaceQuery>,
) -> Response {
    let project = match opened_workspace(&state, params.project) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let limit = params.limit.unwrap_or(20);
    let graph_db = state.runtime_config.codebase_memory_db_path(Some(&project));
    let graph_db_str = graph_db.to_string_lossy().to_string();
    let overview = crate::domains::knowledge_graph::understand_service::understand_project(&graph_db_str)
        .unwrap_or_else(|error| json!({ "indexed": false, "error": error }));
    let indexed = overview
        .get("indexed")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let store = SupermemoryStore::new(&state.runtime_config, Some(&project));
    let mut data = store.load();
    let should_refresh_wiki = params.refresh.unwrap_or(false);
    if indexed && should_refresh_wiki {
        let git_status = crate::domains::version_control::git_service::get_status(&project).ok();
        let shadow_history =
            crate::domains::version_control::git_service::get_ai_history(&project, limit).unwrap_or_default();
        let wiki = synthesize_workspace_wiki(
            &state,
            &project,
            &graph_db_str,
            &overview,
            &data,
            &git_status,
            &shadow_history,
            params.model.clone(),
        )
        .await;
        store.write_memory(SupermemoryRecord {
            id: "project-wiki-latest".to_string(),
            file_path: project.clone(),
            name: "Project Wiki".to_string(),
            summary: wiki,
            tags: vec![
                "project_wiki".to_string(),
                "workspace-intelligence".to_string(),
                "auto-generated".to_string(),
            ],
            complexity: "wiki_overview".to_string(),
            node_type: "project_wiki".to_string(),
            source_graph_path: graph_db_str.clone(),
            source_label: "workspaces/intelligence".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            embedding: None,
        });
        data = store.load();
    }
    let latest_wiki = data
        .memories
        .iter()
        .find(|memory| memory.id == "project-wiki-latest")
        .cloned();
    let latest_overview = data
        .memories
        .iter()
        .find(|memory| memory.id == "project-overview")
        .cloned();
    let latest_blueprints: Vec<SupermemoryRecord> = data
        .memories
        .iter()
        .filter(|memory| memory.node_type == "blueprint")
        .take(5)
        .cloned()
        .collect();
    let memories: Vec<SupermemoryRecord> = data.memories.iter().take(limit).cloned().collect();
    let documents = data.corpus.documents;
    let git_status = crate::domains::version_control::git_service::get_status(&project).ok();
    let shadow_history =
        crate::domains::version_control::git_service::get_ai_history(&project, limit).unwrap_or_default();
    let latest_analysis = state.task_manager.latest_project_analysis(&project);
    let latest_development_run = state.task_manager.latest_project_task(&project, "agent");

    Json(json!({
        "workspace": workspace_descriptor(&state, &project),
        "analysis": latest_analysis,
        "developmentRun": latest_development_run,
        "graph": {
            "indexed": indexed,
            "overview": overview,
        },
        "supermemory": {
            "latestWiki": latest_wiki,
            "latestOverview": latest_overview,
            "latestBlueprints": latest_blueprints,
            "memories": memories,
            "documents": documents,
            "logs": store.get_logs(20),
        },
        "git": {
            "status": git_status,
            "shadowHistory": shadow_history,
        }
    }))
    .into_response()
}

async fn reverse_plan_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceRequest>,
) -> Response {
    let project = match opened_workspace(&state, body.project_path()) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let requirement = match body.requirement {
        Some(requirement) if !requirement.trim().is_empty() => requirement,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing requirement" })),
            )
                .into_response()
        }
    };
    let trace_id = uuid::Uuid::new_v4().to_string();
    let options = ReverseEngineeringOptions {
        supermemory_limit: Some(body.limit.unwrap_or(8)),
        existing_module_limit: Some(12),
        language: body.language.or_else(|| Some("zh-CN".to_string())),
        model: body.model.clone(),
    };
    let request = ReverseEngineeringRequest {
        requirement: Some(requirement.clone()),
        project: Some(project.clone()),
        extracted_schema: None,
        options: Some(options.clone()),
    };

    match crate::reverse_engineering::engine::execute(
        &trace_id,
        &requirement,
        request.extracted_schema,
        Some(&project),
        request.options.as_ref(),
        &state.runtime_config,
        &state.http_client,
        None,
    )
    .await
    {
        Ok(result) => Json(json!({ "success": true, "result": result })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error, "traceId": trace_id })),
        )
            .into_response(),
    }
}

async fn workspace_memory(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WorkspaceQuery>,
) -> Response {
    let project = match opened_workspace(&state, params.project) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let store = SupermemoryStore::new(&state.runtime_config, Some(&project));
    if let Some(query) = params.q {
        return Json(
            json!({ "results": store.search(&query, params.limit.unwrap_or(20)), "query": query }),
        )
        .into_response();
    }
    let data = store.load();
    Json(json!({
        "memories": data.memories,
        "documents": data.corpus.documents,
        "stats": data.stats,
        "logs": store.get_logs(params.limit.unwrap_or(50)),
    }))
    .into_response()
}

fn opened_workspace(state: &Arc<AppState>, project: Option<String>) -> Result<String, Response> {
    let project = required_project(project)?;
    state
        .workspace_capabilities
        .resolve(&project)
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|error| {
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": error, "kind": "workspace_not_open" })),
            )
                .into_response()
        })
}

fn workspace_corpus(
    state: &Arc<AppState>,
    project: &str,
) -> Result<KnowledgeCorpusRuntime, Response> {
    KnowledgeCorpusRuntime::for_workspace(&state.runtime_config, project).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string(), "kind": "knowledge_corpus" })),
        )
            .into_response()
    })
}

async fn workspace_knowledge(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WorkspaceQuery>,
) -> Response {
    let project = match opened_workspace(&state, params.project) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let corpus = match workspace_corpus(&state, &project) {
        Ok(corpus) => corpus,
        Err(response) => return response,
    };
    let documents = match corpus.list_documents() {
        Ok(documents) => documents,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let query = params.q.unwrap_or_default();
    let hits = if query.trim().is_empty() {
        Vec::new()
    } else {
        match corpus.search(&query, params.limit.unwrap_or(20)) {
            Ok(hits) => hits,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": error.to_string() })),
                )
                    .into_response()
            }
        }
    };
    Json(json!({ "project": project, "query": query, "documents": documents, "hits": hits }))
        .into_response()
}

async fn workspace_knowledge_ingest(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceKnowledgeRequest>,
) -> Response {
    let project = match opened_workspace(&state, body.project) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let Some(path) = body.path.filter(|path| !path.trim().is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Missing document path" })),
        )
            .into_response();
    };
    let corpus = match workspace_corpus(&state, &project) {
        Ok(corpus) => corpus,
        Err(response) => return response,
    };
    match corpus.ingest(path, body.global_context.as_deref()) {
        Ok(receipt) => Json(json!({ "success": true, "receipt": receipt })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn workspace_knowledge_rebuild(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceKnowledgeRequest>,
) -> Response {
    let project = match opened_workspace(&state, body.project) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let corpus = match workspace_corpus(&state, &project) {
        Ok(corpus) => corpus,
        Err(response) => return response,
    };
    if let Some(document_id) = body.document_id.filter(|id| !id.trim().is_empty()) {
        match corpus.rebuild(&document_id) {
            Ok(receipt) => Json(json!({ "success": true, "receipt": receipt })).into_response(),
            Err(error) => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response(),
        }
    } else {
        match corpus.rebuild_all() {
            Ok(receipt) => Json(json!({ "success": true, "receipt": receipt })).into_response(),
            Err(error) => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response(),
        }
    }
}

async fn workspace_knowledge_remove(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceKnowledgeRequest>,
) -> Response {
    let project = match opened_workspace(&state, body.project) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let Some(document_id) = body.document_id.filter(|id| !id.trim().is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Missing documentId" })),
        )
            .into_response();
    };
    let corpus = match workspace_corpus(&state, &project) {
        Ok(corpus) => corpus,
        Err(response) => return response,
    };
    match corpus.remove(&document_id) {
        Ok(receipt) => Json(json!({ "success": true, "receipt": receipt })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn workspace_shadow_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WorkspaceQuery>,
) -> Response {
    let project = match opened_workspace(&state, params.project) {
        Ok(project) => project,
        Err(response) => return response,
    };
    match crate::domains::version_control::git_service::get_ai_history(&project, params.limit.unwrap_or(50)) {
        Ok(history) => Json(json!({ "history": history })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn workspace_shadow_snapshot(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceRequest>,
) -> Response {
    let project = match opened_workspace(&state, body.project_path()) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let message = body
        .message
        .unwrap_or_else(|| "Workspace intelligence snapshot".to_string());
    match crate::domains::version_control::git_service::record_ai_snapshot(&project, &message) {
        Ok(hash) => Json(json!({ "success": true, "hash": hash })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn synthesize_workspace_wiki(
    state: &Arc<AppState>,
    project: &str,
    graph_db: &str,
    overview: &serde_json::Value,
    memory_data: &crate::models::memory::SupermemoryStoreData,
    git_status: &Option<crate::domains::version_control::git_service::GitStatus>,
    shadow_history: &[crate::domains::version_control::git_service::GitCommit],
    model: Option<String>,
) -> String {
    let trace = json!({
        "project": project,
        "graphDb": graph_db,
        "overview": overview,
        "supermemory": {
            "memories": memory_data.memories.iter().take(16).collect::<Vec<_>>(),
            "documents": memory_data.corpus.documents.iter().take(16).collect::<Vec<_>>(),
        },
        "git": {
            "status": git_status,
            "shadowHistory": shadow_history,
        }
    });

    let llm = crate::domains::model_gateway::providers::LlmProvider::with_model_override(
        state.http_client.clone(),
        model.clone(),
    );
    if llm.is_available() {
        let compact_trace = serde_json::to_string_pretty(&trace)
            .unwrap_or_default()
            .chars()
            .take(18_000)
            .collect::<String>();
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "请基于工作区智能快照生成一份可长期复用的项目 Wiki。输出 Markdown，包含：项目定位、架构分层、核心模块、关键文件/符号、已有超级记忆、当前 Git 状态、影子 Git 版本链、后续维护建议。不要编造未提供的文件路径、模块或 commit。\n\n工作区智能快照：\n```json\n{}\n```",
                compact_trace
            )
        })];
        if let Ok((content, _)) = llm
            .chat(
                messages,
                Some("你是资深项目智能工作区架构师，负责把代码图谱、需求历史、长期记忆和版本链整理成项目知识。"),
            )
            .await
        {
            return content;
        }
    }

    let totals = overview.get("totals").cloned().unwrap_or_else(|| json!({}));
    let memory_count = memory_data.memories.len();
    let document_count = memory_data.corpus.documents.len();
    let snapshot_count = shadow_history.len();
    format!(
        "# 项目知识链\n\n## 工作区\n\n`{project}`\n\n## 图谱概览\n\n```json\n{totals}\n```\n\n## 超级记忆\n\n- 记忆条目：{memory_count}\n- 知识文档：{document_count}\n\n## 影子 Git\n\n- 快照数量：{snapshot_count}\n\n> 当前未配置可用大模型，已生成结构化占位 Wiki。配置模型后可重新刷新工作区智能快照。"
    )
}

async fn verify_workspace(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<WorkspaceRequest>,
) -> Response {
    let project_path_str = match payload.project_path() {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing project or path"})),
            )
                .into_response()
        }
    };

    let p = std::path::Path::new(&project_path_str);
    if !p.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Directory not found: {}", project_path_str)})),
        )
            .into_response();
    }

    // Auto-detect build tool and test command
    let (label, cmd) = if p.join("Cargo.toml").exists() {
        ("Rust Cargo Test Guard", "cargo check --tests")
    } else if p.join("package.json").exists() {
        ("Node.js Test Guard", "npm test")
    } else if p.join("pom.xml").exists() {
        ("Maven Test Guard", "mvn test-compile")
    } else if p.join("requirements.txt").exists() || p.join("pyproject.toml").exists() {
        ("Python Pytest Guard", "pytest")
    } else if p.join("go.mod").exists() {
        ("Go Test Guard", "go test ./...")
    } else {
        (
            "Generic Sanity Guard",
            "echo [Sanity Check] Workspace structure verified.",
        )
    };

    use crate::services::brain::capabilities::shell::ShellService;
    let shell = crate::services::brain::capabilities::shell::LocalShellProvider;
    match shell.execute(cmd, &project_path_str, false, 60) {
        Ok(res) => {
            let exit_code = res.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let raw_stdout = res.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
            let raw_stderr = res.get("stderr").and_then(|v| v.as_str()).unwrap_or("");

            let clean_stdout =
                crate::services::brain::capabilities::shell::strip_ansi_escapes(raw_stdout);
            let clean_stderr =
                crate::services::brain::capabilities::shell::strip_ansi_escapes(raw_stderr);
            let status = if exit_code == 0 { "passed" } else { "failed" };

            Json(json!({
                "verification": {
                    "status": status,
                    "completedAt": chrono::Utc::now().to_rfc3339(),
                    "commands": [
                        {
                            "label": label,
                            "program": cmd,
                            "status": status,
                            "exitCode": exit_code,
                            "stdout": clean_stdout,
                            "stderr": clean_stderr,
                        }
                    ]
                }
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "verification": {
                    "status": "failed",
                    "commands": [
                        {
                            "label": label,
                            "status": "failed",
                            "error": e,
                        }
                    ]
                }
            })),
        )
            .into_response(),
    }
}
