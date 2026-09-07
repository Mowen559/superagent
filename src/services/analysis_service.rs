use crate::config::app_paths::AppPaths;
use crate::config::runtime::RuntimeConfig;
use crate::models::memory::SupermemoryRecord;
use crate::models::task::{Task, TaskStatus};
use crate::services::agent::AgentMessage;
use crate::services::llm_provider::LlmProvider;
use crate::services::project_inventory::{inspect_project, InventoryLimits, ProjectInventory};
use crate::services::supermemory_store::SupermemoryStore;
use crate::services::task_manager::TaskManager;
use crate::services::understand_service;
use crate::services::understanding_cache::UnderstandingCache;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

const DEFAULT_ANALYSIS_TIMEOUT_SECONDS: u64 = 15 * 60;
const MIN_ANALYSIS_TIMEOUT_SECONDS: u64 = 30;
const MAX_ANALYSIS_TIMEOUT_SECONDS: u64 = 60 * 60;
const DEFAULT_MAX_INVENTORY_FILES: usize = 50_000;
const MIN_MAX_INVENTORY_FILES: usize = 100;
const MAX_MAX_INVENTORY_FILES: usize = 200_000;
const DEFAULT_MAX_FINGERPRINT_BYTES: u64 = 64 * 1024 * 1024;
const MIN_MAX_FINGERPRINT_BYTES: u64 = 1024 * 1024;
const MAX_MAX_FINGERPRINT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_INDEXER_LOG_LINES: usize = 20_000;
const MAX_INDEXER_LOG_CHARS: usize = 2_000;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectAnalysisOptions {
    pub language: Option<String>,
    pub model: Option<String>,
    pub include_llm_overview: bool,
    pub timeout_seconds: u64,
    pub max_inventory_files: usize,
    pub max_fingerprint_bytes: u64,
}

impl Default for ProjectAnalysisOptions {
    fn default() -> Self {
        Self {
            language: Some("zh-CN".to_string()),
            model: None,
            include_llm_overview: true,
            timeout_seconds: DEFAULT_ANALYSIS_TIMEOUT_SECONDS,
            max_inventory_files: DEFAULT_MAX_INVENTORY_FILES,
            max_fingerprint_bytes: DEFAULT_MAX_FINGERPRINT_BYTES,
        }
    }
}

impl ProjectAnalysisOptions {
    fn normalized(mut self) -> Self {
        self.timeout_seconds = self
            .timeout_seconds
            .clamp(MIN_ANALYSIS_TIMEOUT_SECONDS, MAX_ANALYSIS_TIMEOUT_SECONDS);
        self.max_inventory_files = self
            .max_inventory_files
            .clamp(MIN_MAX_INVENTORY_FILES, MAX_MAX_INVENTORY_FILES);
        self.max_fingerprint_bytes = self
            .max_fingerprint_bytes
            .clamp(MIN_MAX_FINGERPRINT_BYTES, MAX_MAX_FINGERPRINT_BYTES);
        self.language = bounded_non_empty(self.language, 32).or_else(|| Some("zh-CN".into()));
        self.model = bounded_non_empty(self.model, 128);
        self
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverviewOutcome {
    status: &'static str,
    memory_synced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

enum IndexOutcome {
    Success,
    ExitFailure(String),
    WaitFailure(String),
    TimedOut,
    Cancelled,
}

/// Start one cancellable project analysis using the process-wide TaskManager.
/// A successful indexer exit is only an intermediate phase: graph validation,
/// deterministic inventory and receipt persistence must also succeed before
/// the task may transition to `completed`.
pub async fn start_analysis(
    runtime_config: &RuntimeConfig,
    app_paths: &AppPaths,
    task_manager: Arc<TaskManager>,
    http_client: reqwest::Client,
    target_path: &str,
    project_id: Option<String>,
    options: ProjectAnalysisOptions,
) -> Result<String, String> {
    let project_root = canonical_project_root(target_path)?;
    let project_path = project_root.to_string_lossy().to_string();
    if let Some(active) = task_manager.latest_project_analysis(&project_path) {
        if !active.status.is_terminal() {
            return Ok(active.id);
        }
    }
    let options = options.normalized();
    let payload = json!({
        "projectId": project_id,
        "options": options,
    });
    let (task, cancel_rx) = task_manager.create_task("analysis", &project_path, Some(payload));
    let task_id = task.id.clone();

    let executable = runtime_config.codebase_memory_bin.clone();
    let cache_dir = runtime_config.codebase_memory_cache_root.clone();
    let runtime_config = runtime_config.clone();
    let app_paths = app_paths.clone();
    tokio::spawn(async move {
        run_analysis(
            task_manager,
            task,
            cancel_rx,
            executable,
            cache_dir,
            runtime_config,
            app_paths,
            http_client,
            project_root,
            project_id,
            options,
        )
        .await;
    });

    Ok(task_id)
}

#[allow(clippy::too_many_arguments)]
async fn run_analysis(
    task_manager: Arc<TaskManager>,
    task: Task,
    mut cancel_rx: watch::Receiver<bool>,
    executable: PathBuf,
    cache_dir: PathBuf,
    runtime_config: RuntimeConfig,
    app_paths: AppPaths,
    http_client: reqwest::Client,
    project_root: PathBuf,
    project_id: Option<String>,
    options: ProjectAnalysisOptions,
) {
    let task_id = task.id;
    let project_path = project_root.to_string_lossy().to_string();
    let started_at = chrono::Utc::now();
    if !task_manager.update_status(&task_id, TaskStatus::Running, None) {
        return;
    }
    emit_phase(
        &task_manager,
        &task_id,
        "analysis.indexing",
        &format!("Starting code graph indexing for {project_path}"),
    );

    let payload = json!({ "repo_path": project_path }).to_string();
    let mut child = match Command::new(&executable)
        .args(["cli", "index_repository", &payload])
        .current_dir(&project_root)
        .env("CBM_CACHE_DIR", &cache_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            fail_task(
                &task_manager,
                &task_id,
                format!(
                    "Failed to start codebase-memory indexer '{}': {error}",
                    executable.display()
                ),
            );
            return;
        }
    };

    let indexer_log_lines = Arc::new(AtomicUsize::new(0));
    let stdout_task = child.stdout.take().map(|stdout| {
        let task_manager = task_manager.clone();
        let task_id = task_id.clone();
        let indexer_log_lines = indexer_log_lines.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    append_indexer_log(
                        &task_manager,
                        &task_id,
                        &indexer_log_lines,
                        "CBM",
                        line.trim(),
                    );
                }
            }
        })
    });
    let stderr_task = child.stderr.take().map(|stderr| {
        let task_manager = task_manager.clone();
        let task_id = task_id.clone();
        let indexer_log_lines = indexer_log_lines.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    append_indexer_log(
                        &task_manager,
                        &task_id,
                        &indexer_log_lines,
                        "CBM:stderr",
                        line.trim(),
                    );
                }
            }
        })
    });

    let timeout = Duration::from_secs(options.timeout_seconds);
    let index_outcome = tokio::select! {
        biased;
        _ = wait_for_cancel(&mut cancel_rx) => {
            let _ = child.kill().await;
            IndexOutcome::Cancelled
        }
        waited = tokio::time::timeout(timeout, child.wait()) => {
            match waited {
                Ok(Ok(status)) if status.success() => IndexOutcome::Success,
                Ok(Ok(status)) => IndexOutcome::ExitFailure(format!(
                    "codebase-memory indexer exited with status {status}"
                )),
                Ok(Err(error)) => IndexOutcome::WaitFailure(format!(
                    "Failed while waiting for codebase-memory indexer: {error}"
                )),
                Err(_) => {
                    let _ = child.kill().await;
                    IndexOutcome::TimedOut
                }
            }
        }
    };
    if let Some(handle) = stdout_task {
        let _ = handle.await;
    }
    if let Some(handle) = stderr_task {
        let _ = handle.await;
    }

    match index_outcome {
        IndexOutcome::Success => {}
        IndexOutcome::Cancelled => return,
        IndexOutcome::TimedOut => {
            task_manager.update_status(
                &task_id,
                TaskStatus::TimedOut,
                Some(format!(
                    "Project analysis timed out after {} seconds",
                    options.timeout_seconds
                )),
            );
            return;
        }
        IndexOutcome::ExitFailure(error) | IndexOutcome::WaitFailure(error) => {
            fail_task(&task_manager, &task_id, error);
            return;
        }
    }
    if is_cancelled(&cancel_rx) {
        return;
    }

    emit_phase(
        &task_manager,
        &task_id,
        "analysis.validating",
        "Indexer finished; validating graph schema and structural results",
    );
    let db_path = runtime_config.codebase_memory_db_path(Some(&project_path));
    let db_path_string = db_path.to_string_lossy().to_string();
    let graph_overview = match understand_service::understand_project(&db_path_string) {
        Ok(value) if value.get("indexed").and_then(Value::as_bool) == Some(true) => value,
        Ok(_) => {
            fail_task(
                &task_manager,
                &task_id,
                format!(
                    "Indexer exited successfully but produced no readable graph at {}",
                    db_path.display()
                ),
            );
            return;
        }
        Err(error) => {
            fail_task(
                &task_manager,
                &task_id,
                format!("Graph validation failed: {error}"),
            );
            return;
        }
    };

    emit_phase(
        &task_manager,
        &task_id,
        "analysis.inventory",
        "Building bounded project inventory and source fingerprint",
    );
    let inventory_root = project_root.clone();
    let inventory_limits = InventoryLimits {
        max_files: options.max_inventory_files,
        max_fingerprint_bytes: options.max_fingerprint_bytes,
    };
    let inventory_task =
        tokio::task::spawn_blocking(move || inspect_project(&inventory_root, inventory_limits));
    let inventory = tokio::select! {
        biased;
        _ = wait_for_cancel(&mut cancel_rx) => return,
        result = inventory_task => match result {
            Ok(Ok(inventory)) => inventory,
            Ok(Err(error)) => {
                fail_task(&task_manager, &task_id, format!("Project inventory failed: {error}"));
                return;
            }
            Err(error) => {
                fail_task(&task_manager, &task_id, format!("Project inventory worker failed: {error}"));
                return;
            }
        }
    };

    let overview_future = generate_project_overview(
        &runtime_config,
        &app_paths,
        &http_client,
        &task_manager,
        &task_id,
        &project_path,
        &graph_overview,
        &inventory,
        &options,
    );
    let overview = tokio::select! {
        biased;
        _ = wait_for_cancel(&mut cancel_rx) => return,
        outcome = overview_future => outcome,
    };

    let mut warnings = Vec::new();
    if inventory.truncated {
        warnings.push("Project inventory or fingerprint reached a configured bound".to_string());
    }
    if inventory.traversal_errors > 0 {
        warnings.push(format!(
            "{} files or directories could not be inspected",
            inventory.traversal_errors
        ));
    }
    if let Some(warning) = overview.warning.clone() {
        warnings.push(warning);
    }
    let graph_db_size = std::fs::metadata(&db_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    let completed_at = chrono::Utc::now();
    let receipt = json!({
        "schemaVersion": 1,
        "kind": "project-analysis",
        "taskId": task_id,
        "projectId": project_id,
        "projectPath": project_path,
        "startedAt": started_at.to_rfc3339(),
        "completedAt": completed_at.to_rfc3339(),
        "durationMs": completed_at.signed_duration_since(started_at).num_milliseconds(),
        "options": options,
        "graph": {
            "databasePath": db_path_string,
            "databaseBytes": graph_db_size,
            "overview": graph_overview,
        },
        "inventory": inventory,
        "llmOverview": overview,
        "warnings": warnings,
    });

    emit_phase(
        &task_manager,
        &task_id,
        "analysis.finalizing",
        "Graph and inventory are ready; committing the Project Analysis Receipt",
    );
    let _ = task_manager.complete_with_receipt(&task_id, receipt);
}

fn canonical_project_root(target_path: &str) -> Result<PathBuf, String> {
    let trimmed = target_path.trim();
    if trimmed.is_empty() {
        return Err("Project path must not be empty".to_string());
    }
    let path = Path::new(trimmed);
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("Cannot open project path '{}': {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "Project path is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn bounded_non_empty(value: Option<String>, max_chars: usize) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.chars().take(max_chars).collect())
        }
    })
}

fn emit_phase(task_manager: &TaskManager, task_id: &str, level: &str, content: &str) {
    task_manager.append_event(
        task_id,
        &AgentMessage::Log {
            level: level.to_string(),
            content: content.to_string(),
        },
    );
}

fn append_indexer_log(
    task_manager: &TaskManager,
    task_id: &str,
    line_count: &AtomicUsize,
    source: &str,
    line: &str,
) {
    let position = line_count.fetch_add(1, Ordering::Relaxed);
    if position < MAX_INDEXER_LOG_LINES {
        let bounded = line.chars().take(MAX_INDEXER_LOG_CHARS).collect::<String>();
        task_manager.append_log(task_id, &format!("[{source}] {bounded}"));
    } else if position == MAX_INDEXER_LOG_LINES {
        task_manager.append_log(
            task_id,
            "[Analysis] Further indexer output was suppressed by the log bound.",
        );
    }
}

fn fail_task(task_manager: &TaskManager, task_id: &str, error: String) {
    task_manager.append_log(task_id, &format!("[Analysis] {error}"));
    task_manager.update_status(task_id, TaskStatus::Failed, Some(error));
}

fn is_cancelled(cancel_rx: &watch::Receiver<bool>) -> bool {
    *cancel_rx.borrow()
}

async fn wait_for_cancel(cancel_rx: &mut watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    let _ = cancel_rx.changed().await;
}

#[allow(clippy::too_many_arguments)]
async fn generate_project_overview(
    runtime_config: &RuntimeConfig,
    app_paths: &AppPaths,
    http_client: &reqwest::Client,
    task_manager: &TaskManager,
    task_id: &str,
    target_path: &str,
    graph_overview: &Value,
    inventory: &ProjectInventory,
    options: &ProjectAnalysisOptions,
) -> OverviewOutcome {
    if !options.include_llm_overview {
        return OverviewOutcome {
            status: "disabled",
            memory_synced: false,
            warning: None,
        };
    }
    let llm = LlmProvider::with_model_override(http_client.clone(), options.model.clone());
    if !llm.is_available() {
        emit_phase(
            task_manager,
            task_id,
            "analysis.overview.skipped",
            "LLM is not configured; deterministic project analysis remains available",
        );
        return OverviewOutcome {
            status: "unavailable",
            memory_synced: false,
            warning: Some("LLM overview was skipped because no provider route is available".into()),
        };
    }

    emit_phase(
        task_manager,
        task_id,
        "analysis.overview",
        "Generating project overview from graph and deterministic inventory",
    );
    let lang = options.language.as_deref().unwrap_or("zh-CN");
    let nodes = graph_overview
        .pointer("/totals/nodes")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let db_path = runtime_config.codebase_memory_db_path(Some(target_path));
    let db_path_string = db_path.to_string_lossy().to_string();
    let cache = UnderstandingCache::for_db(app_paths, &db_path_string);
    let key = UnderstandingCache::project_key_with_source(
        lang,
        &db_path_string,
        nodes,
        &inventory.source_fingerprint,
    );
    if let Some(text) = cache.get(&key).and_then(|value| {
        value
            .get("understanding")
            .and_then(Value::as_str)
            .map(str::to_string)
    }) {
        sync_overview_memory(runtime_config, target_path, &db_path_string, text);
        return OverviewOutcome {
            status: "cached",
            memory_synced: true,
            warning: None,
        };
    }

    let structure = format!(
        "{}\n确定性项目清单: {}",
        understand_service::summarize_project_structure(graph_overview),
        serde_json::to_string(inventory).unwrap_or_default()
    );
    let project_language = inventory
        .languages
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(language, _)| language.clone())
        .unwrap_or_else(|| "unknown".to_string());
    match llm
        .explain_project(&structure, lang, &project_language)
        .await
    {
        Ok(text) => {
            cache.put(&key, json!({ "understanding": &text }));
            sync_overview_memory(runtime_config, target_path, &db_path_string, text);
            OverviewOutcome {
                status: "generated",
                memory_synced: true,
                warning: None,
            }
        }
        Err(error) => {
            let warning = format!("LLM overview generation failed: {}", truncate_error(&error));
            emit_phase(task_manager, task_id, "analysis.overview.warning", &warning);
            OverviewOutcome {
                status: "failed",
                memory_synced: false,
                warning: Some(warning),
            }
        }
    }
}

fn sync_overview_memory(
    runtime_config: &RuntimeConfig,
    target_path: &str,
    db_path: &str,
    text: String,
) {
    let store = SupermemoryStore::new(runtime_config, Some(target_path));
    store.write_memory(SupermemoryRecord {
        id: "project-overview".into(),
        file_path: String::new(),
        name: "Project Overview".into(),
        summary: text,
        tags: vec!["project_overview".into(), "auto-generated".into()],
        complexity: String::new(),
        node_type: "project_overview".into(),
        source_graph_path: db_path.to_string(),
        source_label: "project-analysis".into(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        embedding: None,
    });
    store.add_log(
        "sync",
        Some("Project overview synchronized after verified analysis".into()),
        None,
    );
}

fn truncate_error(error: &str) -> String {
    error.chars().take(400).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;

    #[test]
    fn analysis_options_are_bounded() {
        let options = ProjectAnalysisOptions {
            language: Some("x".repeat(100)),
            model: Some("m".repeat(500)),
            include_llm_overview: true,
            timeout_seconds: 1,
            max_inventory_files: usize::MAX,
            max_fingerprint_bytes: u64::MAX,
        }
        .normalized();
        assert_eq!(options.timeout_seconds, MIN_ANALYSIS_TIMEOUT_SECONDS);
        assert_eq!(options.max_inventory_files, MAX_MAX_INVENTORY_FILES);
        assert_eq!(options.max_fingerprint_bytes, MAX_MAX_FINGERPRINT_BYTES);
        assert_eq!(options.language.unwrap().chars().count(), 32);
        assert_eq!(options.model.unwrap().chars().count(), 128);
    }

    #[test]
    fn project_root_must_exist_and_be_a_directory() {
        let directory = tempfile::tempdir().unwrap();
        assert!(canonical_project_root(directory.path().to_str().unwrap()).is_ok());
        let file = directory.path().join("file.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(canonical_project_root(file.to_str().unwrap()).is_err());
        assert!(canonical_project_root("").is_err());
    }

    #[tokio::test]
    async fn successful_analysis_persists_a_verified_receipt_before_completion() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let cache = directory.path().join("cache");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]\nname='fixture'").unwrap();
        fs::write(project.join("src/main.rs"), "fn main() {}").unwrap();
        let indexer = write_success_indexer(directory.path());
        let app_paths = AppPaths {
            bundle_root: directory.path().join("bundle"),
            app_data_root: directory.path().join("data"),
        };
        let runtime = RuntimeConfig {
            workspace_root: directory.path().to_path_buf(),
            understand_anything_root: directory.path().join("understand"),
            shadow_git_root: directory.path().join("shadow"),
            project_root: project.clone(),
            projects_root: directory.path().join("projects"),
            codebase_memory_cache_root: cache,
            codebase_memory_bin: indexer,
            rtk_bin: directory.path().join("rtk"),
            sso_api_url: "http://127.0.0.1".into(),
        };
        create_graph_fixture(
            &runtime.codebase_memory_db_path(Some(
                canonical_project_root(project.to_str().unwrap())
                    .unwrap()
                    .to_string_lossy()
                    .as_ref(),
            )),
        );
        let task_manager = Arc::new(TaskManager::new(&app_paths));
        let task_id = start_analysis(
            &runtime,
            &app_paths,
            task_manager.clone(),
            reqwest::Client::new(),
            project.to_str().unwrap(),
            Some("fixture".into()),
            ProjectAnalysisOptions {
                include_llm_overview: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let task = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let task = task_manager.get_task(&task_id).unwrap();
                if task.status.is_terminal() {
                    break task;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();

        assert_eq!(task.status, TaskStatus::Completed);
        let receipt = task.receipt.unwrap();
        assert_eq!(
            receipt.get("kind").and_then(Value::as_str),
            Some("project-analysis")
        );
        assert_eq!(
            receipt
                .pointer("/graph/overview/indexed")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            receipt
                .pointer("/inventory/buildSystems/0")
                .and_then(Value::as_str),
            Some("Cargo")
        );
        assert_eq!(
            receipt
                .pointer("/llmOverview/status")
                .and_then(Value::as_str),
            Some("disabled")
        );
        assert!(task_manager.get_events(&task_id).iter().any(|(_, event)| {
            matches!(event, AgentMessage::Log { level, .. } if level == "analysis.finalizing")
        }));
    }

    fn create_graph_fixture(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY, project TEXT, label TEXT, name TEXT,
                qualified_name TEXT, file_path TEXT, start_line INTEGER,
                end_line INTEGER, properties TEXT
            );
            CREATE TABLE edges (
                id INTEGER PRIMARY KEY, project TEXT, source_id INTEGER,
                target_id INTEGER, type TEXT, properties TEXT
            );
            CREATE TABLE project_summaries (summary TEXT);
            INSERT INTO nodes VALUES (1, 'p', 'File', 'main.rs', 'main.rs', 'src/main.rs', 1, 2, '{"extension":".rs"}');
            INSERT INTO nodes VALUES (2, 'p', 'Function', 'main', 'main', 'src/main.rs', 1, 2, '{}');
            INSERT INTO project_summaries VALUES ('fixture');
            "#,
        )
        .unwrap();
    }

    #[cfg(windows)]
    fn write_success_indexer(root: &Path) -> PathBuf {
        let path = root.join("success-indexer.cmd");
        fs::write(&path, "@echo off\r\nexit /b 0\r\n").unwrap();
        path
    }

    #[cfg(unix)]
    fn write_success_indexer(root: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("success-indexer.sh");
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }
}
