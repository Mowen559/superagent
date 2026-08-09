use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;
use tokio::io::{AsyncBufReadExt, BufReader};
use crate::config::app_paths::AppPaths;
use crate::config::runtime::RuntimeConfig;
use crate::models::memory::SupermemoryRecord;
use crate::models::task::TaskStatus;
use crate::services::task_manager::TaskManager;
use crate::services::llm_provider::LlmProvider;
use crate::services::supermemory_store::SupermemoryStore;
use crate::services::understand_service;
use crate::services::understanding_cache::UnderstandingCache;

/// Start project analysis by spawning codebase-memory-mcp
pub async fn start_analysis(
    runtime_config: &RuntimeConfig,
    app_paths: &AppPaths,
    task_manager: Arc<TaskManager>,
    http_client: reqwest::Client,
    target_path: &str,
    project_id: Option<String>,
) -> Result<String, String> {
    if !Path::new(target_path).exists() {
        return Err(format!("Target path does not exist: {}", target_path));
    }

    let (task, _) = task_manager.create_task(
        "analysis",
        target_path,
        project_id.map(|id| serde_json::json!({ "projectId": id })),
    );

    let task_id = task.id.clone();
    let executable = runtime_config.codebase_memory_bin.to_string_lossy().to_string();
    let cache_dir = runtime_config.codebase_memory_cache_root.to_string_lossy().to_string();
    let target_path_owned = target_path.to_string();
    let payload = serde_json::json!({ "repo_path": target_path }).to_string();
    let runtime_config_owned = runtime_config.clone();
    let app_paths_owned = app_paths.clone();

    // Spawn analysis in background
    tokio::spawn(async move {
        task_manager.update_status(&task_id, TaskStatus::Running, None);
        task_manager.append_log(&task_id, &format!("[System] Starting Agentic Analysis for: {}", target_path_owned));
        task_manager.append_log(&task_id, "[Codebase-Memory] Starting physical indexer...");

        let mut child = match Command::new(&executable)
            .args(["cli", "index_repository", &payload])
            .current_dir(&target_path_owned)
            .env("CBM_CACHE_DIR", &cache_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                task_manager.update_status(&task_id, TaskStatus::Failed, Some(format!("Failed to spawn CBM: {}", e)));
                return;
            }
        };

        // Stream stdout
        if let Some(stdout) = child.stdout.take() {
            let tm = task_manager.clone();
            let tid = task_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tm.append_log(&tid, &format!("[CBM] {}", line.trim()));
                    }
                }
            });
        }

        // Stream stderr
        if let Some(stderr) = child.stderr.take() {
            let tm = task_manager.clone();
            let tid = task_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        if trimmed.contains("level=info") || trimmed.contains("level=warn") {
                            tm.append_log(&tid, &format!("[CBM] {}", trimmed));
                        } else {
                            tm.append_log(&tid, &format!("[CBM Error] {}", trimmed));
                        }
                    }
                }
            });
        }

        match child.wait().await {
            Ok(status) if status.success() => {
                task_manager.append_log(&task_id, "[System] Analysis completed successfully.");
                task_manager.update_status(&task_id, TaskStatus::Completed, None);
                // Post-index hook: pre-generate the project overview so the
                // frontend gets a cache hit, and sync it into supermemory.
                generate_project_overview(
                    &runtime_config_owned,
                    &app_paths_owned,
                    &http_client,
                    &task_manager,
                    &task_id,
                    &target_path_owned,
                ).await;
            }
            Ok(status) => {
                let msg = format!("Analysis exited with code: {:?}", status.code());
                task_manager.append_log(&task_id, &format!("[System] {}", msg));
                task_manager.update_status(&task_id, TaskStatus::Failed, Some(msg));
            }
            Err(e) => {
                task_manager.update_status(&task_id, TaskStatus::Failed, Some(e.to_string()));
            }
        }
    });

    Ok(task.id)
}

/// Generate the project-level LLM overview right after a successful index:
/// warms the understanding cache and upserts a `project_overview` record in
/// supermemory (fixed id, so re-indexing overwrites instead of duplicating).
async fn generate_project_overview(
    runtime_config: &RuntimeConfig,
    app_paths: &AppPaths,
    http_client: &reqwest::Client,
    task_manager: &Arc<TaskManager>,
    task_id: &str,
    target_path: &str,
) {
    let llm = LlmProvider::new(http_client.clone());
    if !llm.is_available() {
        task_manager.append_log(task_id, "[System] LLM not configured; skipping project overview generation.");
        return;
    }
    task_manager.append_log(task_id, "[System] LLM is configured. Checking db...");

    let db_path = runtime_config.codebase_memory_db_path(Some(target_path));
    let db_path_str = db_path.to_string_lossy().to_string();
    let result = match understand_service::understand_project(&db_path_str) {
        Ok(v) => v,
        Err(e) => {
            task_manager.append_log(task_id, &format!("[System] Project overview skipped: {}", e));
            return;
        }
    };
    
    let is_indexed = result.get("indexed").and_then(|v| v.as_bool());
    if is_indexed != Some(true) {
        task_manager.append_log(task_id, &format!("[System] Database not indexed. Indexed flag: {:?}", is_indexed));
        return;
    }

    let lang = "zh-CN";
    let nodes = result.pointer("/totals/nodes").and_then(|v| v.as_i64()).unwrap_or(0);
    let cache = UnderstandingCache::for_db(app_paths, &db_path_str);
    let key = UnderstandingCache::project_key(lang, &db_path_str, nodes);
    if cache.get(&key).is_some() {
        task_manager.append_log(task_id, "[System] Project overview already cached.");
        return;
    }

    task_manager.append_log(task_id, "[System] Generating project overview via LLM...");
    let structure = understand_service::summarize_project_structure(&result);
    let project_language = understand_service::detect_primary_language(&db_path_str).unwrap_or_else(|_| "unknown".to_string());
    match llm.explain_project(&structure, lang, &project_language).await {
        Ok(text) => {
            cache.put(&key, serde_json::json!({ "understanding": &text }));
            let store = SupermemoryStore::new(runtime_config, Some(target_path));
            store.write_memory(SupermemoryRecord {
                id: "project-overview".into(),
                file_path: String::new(),
                name: "Project Overview".into(),
                summary: text,
                tags: vec!["project_overview".into(), "auto-generated".into()],
                complexity: String::new(),
                node_type: "project_overview".into(),
                source_graph_path: db_path_str,
                source_label: "understand/project".into(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            });
            store.add_log("sync", Some("Auto-generated project overview after indexing".into()), None);
            task_manager.append_log(task_id, "[System] Project overview cached and synced to supermemory.");
        }
        Err(e) => {
            task_manager.append_log(task_id, &format!("[System] Project overview generation failed: {}", e));
        }
    }
}
