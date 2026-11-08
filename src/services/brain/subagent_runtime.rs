use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use super::ipc::IpcMessage;
use super::steering_queue::{SteeringQueue, SteeringSource};
use super::tool_policy::{ToolPolicyMode, WorkMode};

const MAX_PROMPT_CHARS: usize = 16_000;
const MAX_SYSTEM_PROMPT_CHARS: usize = 32_000;
const MAX_ROLE_CHARS: usize = 120;
const MAX_RESULT_CHARS: usize = 32_000;
const MAX_EVENT_CHARS: usize = 2_000;
const MAX_EVENTS_PER_CHILD: usize = 200;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30 * 60;
const MAX_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
const MAX_GLOBAL_RUNNING: usize = 8;
const MAX_PARENT_RUNNING: usize = 2;
const MAX_DELEGATION_DEPTH: u8 = 1;

#[derive(Debug, Clone)]
struct ParentRun {
    id: String,
    workspace: PathBuf,
    origin: PathBuf,
    policy: WorkMode,
    child_read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SubagentControl {
    Collect {
        parent_run_id: String,
        subagent_id: String,
        operation: String,
        expected_digest: Option<String>,
    },
    Launch {
        parent_run_id: String,
        correlation_id: String,
        role: String,
        prompt: String,
        workspace_mode: Option<String>,
        timeout_seconds: Option<u64>,
    },
    Retry { parent_run_id:String, subagent_id:String },
    Cancel {
        parent_run_id: String,
        subagent_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentState {
    Starting,
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl SubagentState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

#[derive(Debug, Clone)]
pub struct SubagentLaunchRequest {
    pub(crate) replaces_subagent_id: Option<String>,
    pub expected_parent_run: Option<String>,
    pub parent_session_id: String,
    pub correlation_id: Option<String>,
    pub role: String,
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub parent_depth: u8,
    pub timeout: Duration,
    pub workspace_mode: Option<String>,
    pub workspace_path: Option<String>,
}

impl SubagentLaunchRequest {
    pub fn new(
        parent_session_id: impl Into<String>,
        role: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            replaces_subagent_id:None,
            expected_parent_run: None,
            parent_session_id: parent_session_id.into(),
            correlation_id: None,
            role: role.into(),
            prompt: prompt.into(),
            system_prompt: None,
            parent_depth: 0,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
            workspace_mode: None,
            workspace_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentHandle {
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub replaces_subagent_id:Option<String>,
    pub subagent_id: String,
    pub parent_session_id: String,
    pub correlation_id: Option<String>,
    pub role: String,
    pub depth: u8,
    pub created_at: i64,
    pub parent_run_id: String,
    pub workspace_path: PathBuf,
    pub origin_project: PathBuf,
    pub tool_policy: WorkMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    pub at: i64,
    pub kind: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSnapshot {
    #[serde(default)]
    pub collection_receipt: Option<serde_json::Value>,
    #[serde(default)]
    pub process_birth: Option<String>,
    pub handle: SubagentHandle,
    pub state: SubagentState,
    pub updated_at: i64,
    pub result: Option<String>,
    pub error: Option<String>,
    pub recent_events: Vec<SubagentEvent>,
    pub process_id: Option<u32>,
    pub execution_workspace: Option<PathBuf>,
    pub last_heartbeat_at: Option<i64>,
    pub process_reaped: bool,
    pub business_verified: bool,
    #[serde(default)]
    pub workspace_identity: Option<super::subagent_workspace::WorkspaceIdentity>,
}

struct Record {
    recipe:Option<SubagentLaunchRequest>,
    process_birth: Option<String>,
    workspace: Option<Arc<super::subagent_workspace::OwnedWorkspace>>,
    handle: SubagentHandle,
    state: SubagentState,
    updated_at: i64,
    result: Option<String>,
    error: Option<String>,
    events: Vec<SubagentEvent>,
    stdin_tx: Option<mpsc::UnboundedSender<IpcMessage>>,
    cancel_tx: watch::Sender<bool>,
    request_identity: String,
    process_id: Option<u32>,
    execution_workspace: Option<PathBuf>,
    last_heartbeat_at: Option<i64>,
    process_reaped: bool,
}

/// Owns child admission and lifecycle. Process execution is an internal
/// Adapter; callers only use handles and lifecycle operations.
pub struct SubagentRuntime {
    journal_root: PathBuf,
    journal: RwLock<Option<Arc<super::subagent_journal::ChildJournal>>>,
    lifecycle_failed: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    _test_root: tempfile::TempDir,
    workspace_root: PathBuf,
    delivery_progress: RwLock<Option<std::sync::Weak<Mutex<super::task_progress::TaskProgress>>>>,
    records: RwLock<HashMap<String, Arc<Mutex<Record>>>>,
    owner_session_id: String,
    owner_queue: Arc<SteeringQueue>,
    parent_run: RwLock<Option<ParentRun>>,
    model: Option<String>,
    owner_depth: u8,
    emit_events: bool,
    scheduler: RwLock<Option<tokio::runtime::Handle>>,
    #[cfg(test)]
    worker_fixture: Option<PathBuf>,
}

impl Default for SubagentRuntime {
    fn default() -> Self {
        let owner = std::env::var("HUB_AGENT_SESSION_ID")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "default".into());
        Self::new(&owner, Arc::new(SteeringQueue::for_session(&owner)))
    }
}

impl SubagentRuntime {
    pub fn new(owner_session_id: impl Into<String>, owner_queue: Arc<SteeringQueue>) -> Self {
        #[cfg(test)]
        let test_root = tempfile::tempdir().expect("owned runtime test root");
        #[cfg(test)]
        let data_root = test_root.path().to_path_buf();
        #[cfg(not(test))]
        let data_root = crate::config::app_paths::AppPaths::new().app_data_root;
        Self {
            records: RwLock::new(HashMap::new()),
            owner_session_id: owner_session_id.into(),
            owner_queue,
            parent_run: RwLock::new(None),
            model: None,
            owner_depth: delegation_depth(),
            emit_events: false,
            scheduler: RwLock::new(tokio::runtime::Handle::try_current().ok()),
            workspace_root: data_root.join("subagent-workspaces"),
            journal_root: data_root.join("agent-subagents"),
            delivery_progress: RwLock::new(None),
            journal: RwLock::new(None),
            lifecycle_failed: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            worker_fixture: None,
            #[cfg(test)]
            _test_root: test_root,
        }
    }

    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model = model;
        self
    }

    pub fn with_events(mut self, enabled: bool) -> Self {
        self.emit_events = enabled;
        self
    }

    fn ensure_journal(&self) -> Result<(), String> {
        let mut slot = self.journal.write().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Ok(());
        }
        let journal = Arc::new(super::subagent_journal::ChildJournal::open(
            &self.journal_root,
            &self.owner_session_id,
        )?);
        journal.reconcile("interrupted")?;
        let mut records = self.records.write().unwrap_or_else(|e| e.into_inner());
        for entry in journal.entries() {
            let s = entry.snapshot;
            let (cancel_tx, _) = watch::channel(false);
            let workspace = s
                .workspace_identity
                .clone()
                .and_then(|identity| {
                    super::subagent_workspace::OwnedWorkspace::reopen(
                        identity,
                        &s.handle,
                        &self.workspace_root,
                    )
                    .ok()
                })
                .map(Arc::new);
            if let (Some(workspace), Some(receipt)) = (&workspace, s.collection_receipt.clone()) {
                workspace.restore_receipt(receipt)?;
            }
            records.insert(
                s.handle.subagent_id.clone(),
                Arc::new(Mutex::new(Record {
                    recipe:None,
                    handle: s.handle,
                    state: s.state,
                    updated_at: s.updated_at,
                    result: s.result,
                    error: s.error,
                    events: s.recent_events,
                    stdin_tx: None,
                    cancel_tx,
                    request_identity: entry.request_identity,
                    process_id: s.process_id,
                    process_birth: s.process_birth,
                    execution_workspace: s.execution_workspace,
                    last_heartbeat_at: s.last_heartbeat_at,
                    process_reaped: s.process_reaped,
                    workspace,
                })),
            );
        }
        *slot = Some(journal);
        Ok(())
    }

    fn publish(&self, record: &Arc<Mutex<Record>>) -> Result<(), String> {
        let journal = self
            .journal
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let result = persist_snapshot(record, self.emit_events, journal.as_deref());
        if result.is_err() {
            self.lifecycle_failed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        result
    }

    pub fn handle_control(self: &Arc<Self>, control: SubagentControl) {
        let key = match &control {
            SubagentControl::Collect {
                subagent_id,
                operation,
                ..
            } => format!("changes:{subagent_id}:{operation}"),
            SubagentControl::Launch { correlation_id, .. } => correlation_id.clone(),
            SubagentControl::Cancel { subagent_id, .. } => format!("cancel:{subagent_id}"),
            SubagentControl::Retry { subagent_id, .. } => format!("retry:{subagent_id}"),
        };
        let outcome = match control {
            SubagentControl::Retry {parent_run_id,subagent_id} => self.retry(&parent_run_id,&subagent_id).map(|h|serde_json::json!({"handle":h})),
            SubagentControl::Collect {
                parent_run_id,
                subagent_id,
                operation,
                expected_digest,
            } => self.collect_changes(
                &parent_run_id,
                &subagent_id,
                &operation,
                expected_digest.as_deref(),
            ),
            SubagentControl::Launch {
                parent_run_id,
                correlation_id,
                role,
                prompt,
                workspace_mode,
                timeout_seconds,
            } => {
                let allowed = self
                    .parent_run
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .is_some_and(|p| {
                        super::tool_policy::ToolPolicy::new(p.policy.policy_mode())
                            .evaluate("invoke_subagent", &serde_json::json!({}))
                            .1
                            == super::tool_policy::ToolPolicyDecision::Allow
                    });
                if !allowed {
                    Err(
                        "parent tool policy does not allow managed delegation without approval"
                            .into(),
                    )
                } else {
                    let mut request =
                        SubagentLaunchRequest::new(&self.owner_session_id, role, prompt);
                    request.correlation_id = Some(correlation_id);
                    request.expected_parent_run = Some(parent_run_id);
                    request.workspace_mode = workspace_mode;
                    if let Some(seconds) = timeout_seconds {
                        request.timeout = Duration::from_secs(seconds);
                    }
                    self.launch(request).map(|h| serde_json::json!({"subagentId":h.subagent_id,"parentRunId":h.parent_run_id}))
                }
            }
            SubagentControl::Cancel {
                subagent_id,
                parent_run_id,
            } => {
                if self
                    .status(&subagent_id)
                    .is_some_and(|s| s.handle.parent_run_id == parent_run_id)
                {
                    self.cancel(&subagent_id).map(|changed|serde_json::json!({"subagentId":subagent_id,"cancelRequested":changed}))
                } else {
                    Err("child is not owned by the requested parent Run".into())
                }
            }
        };
        if self.emit_events {
            let value = match outcome {
                Ok(result) => serde_json::json!({"ok":true,"result":result}),
                Err(error) => serde_json::json!({"ok":false,"error":error}),
            };
            println!(
                "{}",
                serde_json::json!({"type":"subagent_runtime","schemaVersion":1,"event":"control_receipt","correlationId":key,"receipt":value})
            );
        }
    }

    /// Called by AgentLoop, never from model arguments. Keep this lock order
    /// (parent then records) identical to launch so rebinding cannot race admission.
    pub fn bind_run(
        &self,
        id: &str,
        workspace: &Path,
        policy: ToolPolicyMode,
    ) -> Result<(), String> {
        self.bind_run_with_scope(id, workspace, policy, false)
    }

    pub fn bind_run_with_scope(
        &self,
        id: &str,
        workspace: &Path,
        policy: ToolPolicyMode,
        child_read_only: bool,
    ) -> Result<(), String> {
        self.ensure_journal()?;
        if id.trim().is_empty() {
            return Err("parent Run ID is required".into());
        }
        let workspace = std::fs::canonicalize(workspace)
            .map(|p| crate::infrastructure::fs_path::normalize_path(&p))
            .map_err(|e| format!("parent workspace unavailable: {e}"))?;
        if !workspace.is_dir() {
            return Err("parent workspace is not a directory".into());
        }
        let mut binding = self.parent_run.write().unwrap_or_else(|e| e.into_inner());
        if self
            .list(None)
            .iter()
            .any(|s| !s.state.is_terminal() || !s.process_reaped)
        {
            return Err("cannot rebind parent with unsettled subagents".into());
        }
        let origin =
            crate::infrastructure::fs_path::resolve_origin_project_from_baseline(&workspace, &[]);
        let policy = match policy {
            ToolPolicyMode::ReadOnly => WorkMode::Plan,
            ToolPolicyMode::Approval => WorkMode::Build,
            ToolPolicyMode::AutoEdit => WorkMode::Edit,
            ToolPolicyMode::Audit => WorkMode::Yolo,
            ToolPolicyMode::Guarded => WorkMode::Guarded,
        };
        if self.emit_events {
            println!(
                "{}",
                serde_json::json!({"type":"subagent_runtime","schemaVersion":1,"event":"bound","parent":{
                    "parentRunId":id,"parentSessionId":self.owner_session_id,"workspacePath":workspace,
                "originProject":origin,"toolPolicy":policy,"model":self.model,"childReadOnly":child_read_only
                }})
            );
        }
        *binding = Some(ParentRun {
            id: id.into(),
            workspace,
            origin,
            policy,
            child_read_only,
        });
        Ok(())
    }

    pub async fn close_run(&self) -> Result<(), String> {
        // Close admission before inspecting records. Terminal state is published
        // only after process reaping, so the parent cannot race a late child write.
        let run = self
            .parent_run
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let Some(run) = run else {
            return Ok(());
        };
        let children = self
            .list(None)
            .into_iter()
            .filter(|s| s.handle.parent_run_id == run.id)
            .collect::<Vec<_>>();
        let unfinished = children.iter().any(|s| !s.state.is_terminal());
        for child in &children {
            if !child.state.is_terminal() {
                let _ = self.cancel(&child.handle.subagent_id);
            }
        }
        let settled = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self
                    .list(None)
                    .iter()
                    .filter(|s| s.handle.parent_run_id == run.id)
                    .all(|s| s.process_reaped)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok();
        if !settled {
            return Err("subagent process cleanup deadline exceeded".into());
        }
        if self
            .lifecycle_failed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err("child lifecycle persistence failed; completion rejected".into());
        }
        if unfinished {
            return Err("parent ended with unfinished subagents; owned processes cancelled".into());
        }
        Ok(())
    }

    pub fn launch(
        self: &Arc<Self>,
        mut request: SubagentLaunchRequest,
    ) -> Result<SubagentHandle, String> {
        request.parent_depth = request.parent_depth.max(self.owner_depth);
        validate_request(&request)?;
        let scheduler = self
            .scheduler
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
            .ok_or("subagent async runtime is unavailable")?;
        if request.parent_session_id != self.owner_session_id {
            return Err("subagent parent does not match runtime owner".to_string());
        }
        let binding = self.parent_run.read().unwrap_or_else(|e| e.into_inner());
        let parent = binding
            .as_ref()
            .ok_or("subagent requires a bound active parent Run")?;
        if request
            .expected_parent_run
            .as_ref()
            .is_some_and(|id| id != &parent.id)
        {
            return Err("control does not match the active parent Run".into());
        }
        if let Some(path) = request.workspace_path.as_deref() {
            let supplied = std::fs::canonicalize(path)
                .map_err(|e| format!("child workspace unavailable: {e}"))?;
            if crate::infrastructure::fs_path::normalize_path(&supplied) != parent.workspace {
                return Err("child workspace must match bound parent workspace".into());
            }
        }
        request.workspace_path = Some(parent.workspace.to_string_lossy().into());
        let policy = if parent.child_read_only {
            WorkMode::Plan
        } else {
            child_policy(&request.role, parent.policy)
        };
        let request_identity = format!(
            "{:?}",
            (
                &request.role,
                &request.prompt,
                &request.system_prompt,
                &request.workspace_mode,
                request.timeout
            )
        );
        let request_identity =
            crate::shared_kernel::crypto::sha256_hex(request_identity.as_bytes());
        let depth = request.parent_depth.saturating_add(1);
        let mut records = self
            .records
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(correlation_id) = request.correlation_id.as_deref() {
            if let Some(record) = records.values().find(|record| {
                let record = record
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                record.handle.parent_session_id == request.parent_session_id
                    && record.handle.parent_run_id == parent.id
                    && record.handle.correlation_id.as_deref() == Some(correlation_id)
            }) {
                let record = record.lock().unwrap_or_else(|e| e.into_inner());
                if record.request_identity != request_identity || record.handle.replaces_subagent_id != request.replaces_subagent_id {
                    return Err("correlation ID already used with different task input".into());
                }
                return Ok(record.handle.clone());
            }
        }
        if let Some(old_id)=request.replaces_subagent_id.as_deref() {
            let old=records.get(old_id).ok_or("replacement predecessor unavailable")?.lock().unwrap_or_else(|e|e.into_inner());
            let readonly_purpose=old.recipe.as_ref().is_some_and(|recipe| {
                super::cognitive_runtime::CognitiveRuntime::begin(&recipe.prompt).task_progress().lock().unwrap_or_else(|e|e.into_inner()).is_read_only_scope()
            });
            if readonly_purpose {return Err("read-only review/investigation purpose cannot be replaced by this implementation retry".into());}
            if old.handle.parent_run_id!=parent.id || old.handle.parent_session_id!=self.owner_session_id || !old.state.is_terminal() || !old.process_reaped || old.state==SubagentState::Succeeded || old.handle.replaces_subagent_id.is_some() || old.handle.tool_policy==WorkMode::Plan || old.workspace.is_none() || old.request_identity!=request_identity || policy!=old.handle.tool_policy {
                return Err("replacement requires the same frozen owned implementation task, terminal failure and reaped isolated process; review/replacement chains are forbidden".into());
            }
            drop(old);
            if records.values().any(|r|r.lock().unwrap_or_else(|e|e.into_inner()).handle.replaces_subagent_id.as_deref()==Some(old_id)) {return Err("predecessor already has a replacement".into());}
        }
        let running_global = records
            .values()
            .filter(|record| {
                !record
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .process_reaped
            })
            .count();
        if running_global >= MAX_GLOBAL_RUNNING {
            return Err(format!(
                "subagent global concurrency limit reached ({MAX_GLOBAL_RUNNING})"
            ));
        }
        let running_for_parent = records
            .values()
            .filter(|record| {
                let record = record
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                record.handle.parent_session_id == request.parent_session_id
                    && !record.process_reaped
            })
            .count();
        if running_for_parent >= MAX_PARENT_RUNNING {
            return Err(format!(
                "subagent parent concurrency limit reached ({MAX_PARENT_RUNNING})"
            ));
        }

        let now = Utc::now().timestamp_millis();
        let handle = SubagentHandle {
            replaces_subagent_id:request.replaces_subagent_id.clone(),
            subagent_id: Uuid::new_v4().to_string(),
            parent_session_id: request.parent_session_id.clone(),
            correlation_id: request.correlation_id.clone(),
            role: request.role.clone(),
            depth,
            created_at: now,
            parent_run_id: parent.id.clone(),
            workspace_path: parent.workspace.clone(),
            origin_project: parent.origin.clone(),
            tool_policy: policy,
        };
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let record = Arc::new(Mutex::new(Record {
                    recipe:Some(request.clone()),
            process_birth: None,
            workspace: None,
            handle: handle.clone(),
            state: SubagentState::Starting,
            updated_at: now,
            result: None,
            error: None,
            events: vec![SubagentEvent {
                at: now,
                kind: "accepted".to_string(),
                content: format!("role={} depth={depth}", request.role),
            }],
            stdin_tx: None,
            cancel_tx,
            request_identity,
            process_id: None,
            execution_workspace: None,
            last_heartbeat_at: None,
            process_reaped: false,
        }));
        records.insert(handle.subagent_id.clone(), record.clone());
        drop(records);
        if let Err(error) = self.publish(&record) {
            self.records
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&handle.subagent_id);
            return Err(error);
        }

        let runtime = self.clone();
        scheduler.spawn(async move {
            runtime.run_child(record, request, cancel_rx).await;
        });
        Ok(handle)
    }

    pub fn bind_delivery_progress(self: &Arc<Self>, progress: &super::task_progress::SharedTaskProgress) {
        progress.lock().unwrap_or_else(|e| e.into_inner()).bind_subagents(Arc::downgrade(self));
        *self
            .delivery_progress
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(progress));
    }

    /// Projection only: child/process/journal facts remain owned by this runtime.
    pub fn delivery_projection(&self, parent_run: &str) -> serde_json::Value {
        let children: Vec<_> = self.list(None).into_iter().filter(|c| c.handle.parent_run_id == parent_run).collect();
        let mut gap = None;
        let mut needs_verification = false;
        for child in &children {
            if !child.process_reaped || !child.state.is_terminal() {
                gap.get_or_insert_with(|| format!("child {} has not settled", child.handle.subagent_id));
            } else if child.state != SubagentState::Succeeded && !children.iter().any(|next|next.handle.replaces_subagent_id.as_deref()==Some(&child.handle.subagent_id)&&next.state==SubagentState::Succeeded&&next.process_reaped) {
                gap.get_or_insert_with(|| format!("child {} ended {:?}; delivery rejected; preserve failure. A reaped isolated implementation can use retry_subagent once; reviews or unavailable recipes require a new bounded Run", child.handle.subagent_id, child.state));
            }
            needs_verification |= child.handle.tool_policy != WorkMode::Plan;
            if child.workspace_identity.is_some() && child.state == SubagentState::Succeeded && child.process_reaped {
                if let Ok(record) = self.record(&child.handle.subagent_id) {
                    let workspace = record.lock().unwrap_or_else(|e|e.into_inner()).workspace.clone();
                    match workspace.ok_or_else(|| "owned artifact workspace unavailable".to_string()).and_then(|w| w.inspect(&child.handle)) {
                        Ok(inspection) => {
                            let changed = inspection["changes"]["files"].as_array().is_some_and(|f| !f.is_empty());
                            needs_verification |= changed;
                            if changed && child.collection_receipt.is_none() {
                                gap.get_or_insert_with(|| format!("child {} has uncollected artifacts; inspect/apply before parent validation", child.handle.subagent_id));
                            }
                        }
                        Err(error) => {gap.get_or_insert_with(|| format!("child artifact inspection failed: {error}"));}
                    }
                }
            }
        }
        let mut epoch_children=children.iter().map(|c|serde_json::json!({"id":c.handle.subagent_id,"replaces":c.handle.replaces_subagent_id,"state":c.state,"reaped":c.process_reaped,"collection":c.collection_receipt})).collect::<Vec<_>>();
        epoch_children.sort_by_key(|c|c["id"].as_str().unwrap_or("").to_string());
        let epoch=crate::shared_kernel::crypto::sha256_hex(serde_json::to_string(&epoch_children).unwrap().as_bytes());
        serde_json::json!({"evidenceEpoch":epoch,"parentRunId":parent_run,"childCount":children.len(),"requiresParentVerification":needs_verification,
            "gap":gap,"state":if gap.is_some(){"blocked"}else if needs_verification{"awaiting_parent_verification"}else{"settled"},
            "children":children.iter().map(|c|serde_json::json!({"subagentId":c.handle.subagent_id,"replacesSubagentId":c.handle.replaces_subagent_id,"replacementSubagentId":children.iter().find(|n|n.handle.replaces_subagent_id.as_deref()==Some(c.handle.subagent_id.as_str())).map(|n|n.handle.subagent_id.clone()),"state":c.state,"processReaped":c.process_reaped,"collected":c.collection_receipt.is_some()})).collect::<Vec<_>>(),"businessVerified":false})
    }

    pub fn completion_gap(&self) -> Option<String> {
        let run = self.parent_run.read().unwrap_or_else(|e|e.into_inner()).as_ref()?.id.clone();
        if self.delivery_projection(&run)["childCount"] == 0 {return None;}
        let progress = self.delivery_progress.read().unwrap_or_else(|e|e.into_inner()).as_ref().and_then(|p|p.upgrade());
        match progress { Some(p) => p.lock().unwrap_or_else(|e|e.into_inner()).delivery_gate_gap(), None => Some("parent delivery evidence is unavailable for delegated work".into()) }
    }

    pub fn retry(self:&Arc<Self>,parent_run:&str,id:&str)->Result<SubagentHandle,String> {
        let allowed=self.parent_run.read().unwrap_or_else(|e|e.into_inner()).as_ref().is_some_and(|p|
            p.id==parent_run && super::tool_policy::ToolPolicy::new(p.policy.policy_mode()).evaluate("retry_subagent",&serde_json::json!({})).1==super::tool_policy::ToolPolicyDecision::Allow);
        if !allowed {return Err("active parent policy does not permit retry without approval".into());}
        let old=self.record(id)?;
        let mut request={let r=old.lock().unwrap_or_else(|e|e.into_inner());
            if r.handle.parent_run_id!=parent_run || r.handle.parent_session_id!=self.owner_session_id {return Err("predecessor is not owned by parent Run".into());}
            r.recipe.clone().ok_or("original frozen recipe unavailable after recovery; start a new bounded Run")?};
        request.replaces_subagent_id=Some(id.into());request.correlation_id=Some(format!("retry:{id}"));request.expected_parent_run=Some(parent_run.into());
        self.launch(request)
    }

    pub fn collect_changes(
        &self,
        parent_run: &str,
        id: &str,
        action: &str,
        digest: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let binding = self.parent_run.read().unwrap_or_else(|e| e.into_inner());
        let parent = binding
            .as_ref()
            .filter(|p| p.id == parent_run)
            .ok_or("collection requires the active owning parent Run")?;
        let record = self.record(id)?;
        let (handle, workspace, state, reaped) = {
            let r = record.lock().unwrap_or_else(|e| e.into_inner());
            (
                r.handle.clone(),
                r.workspace.clone(),
                r.state,
                r.process_reaped,
            )
        };
        if handle.parent_run_id != parent_run || handle.parent_session_id != self.owner_session_id {
            return Err("artifact belongs to another parent Run".into());
        }
        if !state.is_terminal() || !reaped {
            return Err("collect only after the owned child process is settled".into());
        }
        let workspace = workspace.ok_or("child has no isolated artifact workspace")?;
        if action == "inspect" {
            return workspace.inspect(&handle);
        }
        if action != "apply" {
            return Err("collection action must be inspect or apply".into());
        }
        if state != SubagentState::Succeeded {
            return Err("failed/cancelled/rejected child cannot be collected".into());
        }
        if parent.child_read_only
            || parent.policy == WorkMode::Plan
            || super::tool_policy::ToolPolicy::new(parent.policy.policy_mode())
                .evaluate("write_to_file", &serde_json::json!({}))
                .1
                != super::tool_policy::ToolPolicyDecision::Allow
        {
            return Err(
                "parent permission does not allow artifact application without approval".into(),
            );
        }
        let progress = self
            .delivery_progress
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|p| p.upgrade())
            .ok_or("parent delivery guard is unavailable")?;
        let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
        let result = workspace.apply(
            &handle,
            digest.ok_or("ExpectedDigest from inspect is required")?,
            |file| {
                let args = serde_json::json!({"TargetFile":file});
                if let Some(reason) = progress
                    .receipt_edit_guard("write_to_file", &args)
                    .or_else(|| progress.internal_trace_guard("write_to_file", &args))
                {
                    return Err(reason);
                }
                Ok(())
            },
        )?;
        if result["alreadyApplied"] != true && ["appliedFiles","deletedFiles"].iter().any(|key|
            result["receipt"]["apply"][*key].as_array().is_some_and(|files|!files.is_empty())) {
            // Management API collection bypasses the model tool gateway. It
            // must invalidate the same existing workflow/physical receipts.
            progress.note_requirement_update();
        }
        drop(progress);
        drop(binding);
        self.publish(&record)?;
        Ok(result)
    }

    pub fn status(&self, subagent_id: &str) -> Option<SubagentSnapshot> {
        let record = self
            .records
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(subagent_id)
            .cloned()?;
        let record = record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(snapshot(&record))
    }

    pub fn list(&self, parent_session_id: Option<&str>) -> Vec<SubagentSnapshot> {
        let records = self
            .records
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut snapshots = records
            .values()
            .filter_map(|record| {
                let record = record
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if parent_session_id.is_some_and(|parent| parent != record.handle.parent_session_id)
                {
                    None
                } else {
                    Some(snapshot(&record))
                }
            })
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| snapshot.handle.created_at);
        snapshots
    }

    pub fn intervene(&self, subagent_id: &str, instruction: &str) -> Result<(), String> {
        if instruction.trim().is_empty() {
            return Err("intervention must not be empty".to_string());
        }
        let record = self.record(subagent_id)?;
        let mut record = record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if record.state != SubagentState::Running {
            return Err(format!("subagent is not running: {:?}", record.state));
        }
        let tx = record
            .stdin_tx
            .clone()
            .ok_or_else(|| "subagent input is unavailable".to_string())?;
        let instruction = bounded(instruction, MAX_EVENT_CHARS);
        tx.send(IpcMessage::Intervene(instruction.clone()))
            .map_err(|error| format!("failed to send intervention: {error}"))?;
        push_event(&mut record, "intervention", instruction);
        Ok(())
    }

    pub fn cancel(&self, subagent_id: &str) -> Result<bool, String> {
        let record = self.record(subagent_id)?;
        let mut record = record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if record.state.is_terminal() {
            return Ok(false);
        }
        record.state = SubagentState::CancelRequested;
        record.updated_at = Utc::now().timestamp_millis();
        push_event(
            &mut record,
            "cancel_requested",
            "parent requested cancellation".to_string(),
        );
        let _ = record.cancel_tx.send(true);
        drop(record);
        self.publish(&self.record(subagent_id)?)?;
        Ok(true)
    }

    fn record(&self, id: &str) -> Result<Arc<Mutex<Record>>, String> {
        self.records
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown subagent: {id}"))
    }

    async fn run_child(
        self: Arc<Self>,
        record: Arc<Mutex<Record>>,
        request: SubagentLaunchRequest,
        mut cancel_rx: watch::Receiver<bool>,
    ) {
        let handle = {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .handle
                .clone()
        };
        let prepared = tokio::select! {
            result = prepare_workspace(&handle, &request, &self.workspace_root) => result,
            _ = cancel_rx.wait_for(|v| *v) => Err("subagent cancelled before process launch".into()),
        };
        let (workspace, owned_workspace) = match prepared {
            Ok(value) => value,
            Err(error) => {
                record
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .process_reaped = true;
                let state = if *cancel_rx.borrow() {
                    SubagentState::Cancelled
                } else {
                    SubagentState::Failed
                };
                finish_record(&record, state, None, Some(error));
                self.deliver_parent_result(&handle, &record);
                return;
            }
        };
        {
            let mut guard = record.lock().unwrap_or_else(|e| e.into_inner());
            guard.execution_workspace = Some(workspace.clone());
            guard.workspace = owned_workspace;
        }
        let worker = worker_command(&handle, &request, &workspace, self.model.as_deref());
        #[cfg(test)]
        let worker = if let Some(script) = &self.worker_fixture {
            let mut command = Command::new("node");
            command.arg(script);
            Ok((command, true))
        } else {
            worker
        };
        let (mut command, legacy_ipc) = match worker {
            Ok(value) => value,
            Err(error) => {
                record
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .process_reaped = true;
                finish_record(&record, SubagentState::Failed, None, Some(error));
                self.deliver_parent_result(&handle, &record);
                return;
            }
        };
        command
            .current_dir(&workspace)
            .env("HUB_AGENT_SESSION_ID", &handle.subagent_id)
            .env("HUB_PARENT_SESSION_ID", &handle.parent_session_id)
            .env("HUB_PARENT_RUN_ID", &handle.parent_run_id)
            .env("HUB_DELEGATION_DEPTH", handle.depth.to_string())
            .env("HUB_TOOL_POLICY_MODE", handle.tool_policy.as_str())
            .env("HUB_PROJECT_ROOT", &workspace)
            .env("HUB_WORKSPACE_DIR", &workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        if *cancel_rx.borrow() {
            record
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .process_reaped = true;
            finish_record(
                &record,
                SubagentState::Cancelled,
                None,
                Some("subagent cancelled before process launch".into()),
            );
            self.deliver_parent_result(&handle, &record);
            return;
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                record
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .process_reaped = true;
                finish_record(
                    &record,
                    SubagentState::Failed,
                    None,
                    Some(format!("failed to spawn child: {error}")),
                );
                self.deliver_parent_result(&handle, &record);
                return;
            }
        };
        let process_tree = match crate::services::agent::proc::RunProcessTree::attach(&child) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill().await;
                let reaped = child.wait().await.is_ok();
                record
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .process_reaped = reaped;
                finish_record(
                    &record,
                    SubagentState::Failed,
                    None,
                    Some(format!("cannot contain subagent process tree: {error}")),
                );
                self.deliver_parent_result(&handle, &record);
                return;
            }
        };
        let mut stdin = child.stdin.take().expect("piped child stdin");
        let stdout = child.stdout.take().expect("piped child stdout");
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel();
        {
            let mut guard = record.lock().unwrap_or_else(|e| e.into_inner());
            guard.stdin_tx = Some(stdin_tx.clone());
            guard.process_id = child.id();
            guard.process_birth = child.id().and_then(super::subagent_journal::process_birth);
            if guard.state != SubagentState::CancelRequested {
                guard.state = SubagentState::Running;
            }
            guard.updated_at = Utc::now().timestamp_millis();
            push_event(&mut guard, "running", "owned child process started".into());
        }
        let running_persist = self.publish(&record);
        let writer = tokio::spawn(async move {
            while let Some(message) = stdin_rx.recv().await {
                let Ok(serialized) = serde_json::to_string(&message) else {
                    continue;
                };
                if stdin
                    .write_all(format!("{serialized}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });
        if legacy_ipc {
            let _ = stdin_tx.send(IpcMessage::Spawn {
                role: request.role,
                system_prompt: request.system_prompt,
                prompt: request.prompt,
            });
        }
        let journal = self
            .journal
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let outcome = if let Err(error) = running_persist {
            (SubagentState::Failed, None, Some(error))
        } else {
            tokio::select! {
                output = tokio::time::timeout(request.timeout, read_child_output(stdout, record.clone(), self.emit_events, journal)) => match output {
                    Ok(Some(Ok(result))) => (SubagentState::Succeeded, Some(result), None),
                    Ok(Some(Err(error))) => (SubagentState::Failed, None, Some(error)),
                    Ok(None) => (SubagentState::Failed, None, Some("child exited without a terminal result".into())),
                    Err(_) => (SubagentState::TimedOut, None, Some("subagent timed out".into())),
                },
                _ = cancel_rx.wait_for(|v| *v) => (SubagentState::Cancelled, None, Some("subagent cancelled by parent".into())),
            }
        };
        process_tree.terminate();
        let _ = child.kill().await;
        let reaped = child.wait().await.is_ok();
        writer.abort();
        let _ = writer.await;
        let outcome = {
            let mut guard = record.lock().unwrap_or_else(|e| e.into_inner());
            guard.process_reaped = reaped;
            if guard.state == SubagentState::CancelRequested {
                (
                    SubagentState::Cancelled,
                    None,
                    Some("subagent cancelled by parent".into()),
                )
            } else if !reaped {
                (
                    SubagentState::Failed,
                    None,
                    Some("failed to reap subagent process".into()),
                )
            } else {
                outcome
            }
        };
        finish_record(&record, outcome.0, outcome.1, outcome.2);
        // Keep owned worktrees and untracked artifacts for verified collection.

        self.deliver_parent_result(&handle, &record);
    }

    fn deliver_parent_result(&self, handle: &SubagentHandle, record: &Arc<Mutex<Record>>) {
        if self.publish(record).is_err() {
            return;
        }
        let snapshot = {
            let record = record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            snapshot(&record)
        };
        let content = serde_json::to_string(&snapshot).unwrap_or_else(|_| {
            format!(
                "subagent {} ended with {:?}",
                handle.subagent_id, snapshot.state
            )
        });
        if let Err(error) = self.owner_queue.enqueue(
            SteeringSource::Subagent,
            &content,
            Some(&format!("subagent:{}", handle.subagent_id)),
        ) {
            let mut record = record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            push_event(&mut record, "delivery_error", error);
        }
    }
}

async fn read_child_output(
    stdout: tokio::process::ChildStdout,
    record: Arc<Mutex<Record>>,
    emit_events: bool,
    journal: Option<Arc<super::subagent_journal::ChildJournal>>,
) -> Option<Result<String, String>> {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(message) = serde_json::from_str::<IpcMessage>(&line) {
            match message {
                IpcMessage::Result(result) => return Some(Ok(bounded(&result, MAX_RESULT_CHARS))),
                IpcMessage::Error(error) => return Some(Err(bounded(&error, MAX_RESULT_CHARS))),
                IpcMessage::Event {
                    event_type,
                    content,
                } => {
                    let mut record = record
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    push_event(&mut record, &event_type, bounded(&content, MAX_EVENT_CHARS));
                }
                IpcMessage::Heartbeat => {
                    let mut guard = record
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.updated_at = Utc::now().timestamp_millis();
                    guard.last_heartbeat_at = Some(guard.updated_at);
                    drop(guard);
                    // State still comes from the owned runtime, never from the child payload.
                    if let Err(error) = persist_snapshot(&record, emit_events, journal.as_deref()) {
                        return Some(Err(format!("heartbeat persistence failed: {error}")));
                    }
                }
                _ => {}
            }
        } else {
            // The bundled Hub worker uses its existing JSONL event protocol.
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if value["type"] == "status" && value["status"] == "completed" {
                    return Some(
                        value["result"]
                            .as_str()
                            .map(|s| bounded(s, MAX_RESULT_CHARS))
                            .ok_or_else(|| "worker completed without a result".to_string()),
                    );
                }
                if value["type"] == "error" || value["type"] == "failed" {
                    return Some(Err(bounded(
                        value["content"].as_str().unwrap_or("worker failed"),
                        MAX_RESULT_CHARS,
                    )));
                }
            }
            let mut record = record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            push_event(&mut record, "log", bounded(&line, MAX_EVENT_CHARS));
        }
    }
    None
}

fn emit_snapshot(record: &Arc<Mutex<Record>>, enabled: bool) {
    if !enabled {
        return;
    }
    let record = record.lock().unwrap_or_else(|e| e.into_inner());
    println!(
        "{}",
        serde_json::json!({"type":"subagent_runtime","schemaVersion":1,"event":"snapshot","snapshot":snapshot(&record)})
    );
}

fn persist_snapshot(
    record: &Arc<Mutex<Record>>,
    enabled: bool,
    journal: Option<&super::subagent_journal::ChildJournal>,
) -> Result<(), String> {
    let guard = record.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(journal) = journal {
        journal.append("snapshot", &guard.request_identity, &snapshot(&guard))?;
    }
    drop(guard);
    emit_snapshot(record, enabled);
    Ok(())
}

fn finish_record(
    record: &Arc<Mutex<Record>>,
    state: SubagentState,
    result: Option<String>,
    error: Option<String>,
) {
    let mut record = record
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if record.state.is_terminal() {
        return;
    }
    let (state, result, error) = if record.state == SubagentState::CancelRequested {
        (
            SubagentState::Cancelled,
            None,
            Some("subagent cancelled by parent".into()),
        )
    } else {
        (state, result, error)
    };
    record.state = state;
    record.updated_at = Utc::now().timestamp_millis();
    record.result = result.map(|value| bounded(&value, MAX_RESULT_CHARS));
    record.error = error.map(|value| bounded(&value, MAX_RESULT_CHARS));
    record.stdin_tx = None;
    push_event(&mut record, "terminal", format!("{state:?}"));
}

fn push_event(record: &mut Record, kind: &str, content: String) {
    record.events.push(SubagentEvent {
        at: Utc::now().timestamp_millis(),
        kind: bounded(kind, 80),
        content: bounded(&content, MAX_EVENT_CHARS),
    });
    if record.events.len() > MAX_EVENTS_PER_CHILD {
        record
            .events
            .drain(..record.events.len() - MAX_EVENTS_PER_CHILD);
    }
}

fn snapshot(record: &Record) -> SubagentSnapshot {
    SubagentSnapshot {
        collection_receipt: record.workspace.as_ref().and_then(|w| w.applied_receipt()),
        process_birth: record.process_birth.clone(),
        handle: record.handle.clone(),
        state: record.state,
        updated_at: record.updated_at,
        result: record.result.clone(),
        error: record.error.clone(),
        recent_events: record
            .events
            .iter()
            .rev()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        process_id: record.process_id,
        execution_workspace: record.execution_workspace.clone(),
        last_heartbeat_at: record.last_heartbeat_at,
        process_reaped: record.process_reaped,
        business_verified: false,
        workspace_identity: record.workspace.as_ref().map(|w| w.identity.clone()),
    }
}

fn validate_request(request: &SubagentLaunchRequest) -> Result<(), String> {
    if !matches!(
        request.workspace_mode.as_deref(),
        None | Some("inherit" | "worktree")
    ) {
        return Err("workspace mode must be inherit or worktree".into());
    }
    if request.parent_session_id.trim().is_empty() {
        return Err("parent_session_id is required".to_string());
    }
    if request.prompt.trim().is_empty() || request.prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!(
            "prompt must contain 1..={MAX_PROMPT_CHARS} characters"
        ));
    }
    if request.role.trim().is_empty() || request.role.chars().count() > MAX_ROLE_CHARS {
        return Err(format!("role must contain 1..={MAX_ROLE_CHARS} characters"));
    }
    if request
        .system_prompt
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_SYSTEM_PROMPT_CHARS)
    {
        return Err(format!(
            "system_prompt exceeds {MAX_SYSTEM_PROMPT_CHARS} characters"
        ));
    }
    if request.parent_depth >= MAX_DELEGATION_DEPTH {
        return Err(format!(
            "maximum delegation depth reached ({MAX_DELEGATION_DEPTH})"
        ));
    }
    if request.timeout.is_zero() || request.timeout > Duration::from_secs(MAX_TIMEOUT_SECONDS) {
        return Err(format!(
            "timeout must be between 1 and {MAX_TIMEOUT_SECONDS} seconds"
        ));
    }
    Ok(())
}

fn worker_command(
    handle: &SubagentHandle,
    request: &SubagentLaunchRequest,
    workspace: &Path,
    model: Option<&str>,
) -> Result<(Command, bool), String> {
    if let Ok(path) = std::env::var("HUB_SUPERAGENT_BIN") {
        if !path.trim().is_empty() {
            return Ok((Command::new(path), true));
        }
    }
    if let Ok(current) = std::env::current_exe() {
        if current
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("superagent"))
        {
            return Ok((Command::new(current), true));
        }
        let mut command = Command::new(current);
        command
            .arg("superagent-worker")
            .arg("--prompt")
            .arg(format!(
                "Role: {}\n{}\n{}",
                request.role,
                request.system_prompt.as_deref().unwrap_or(""),
                request.prompt
            ))
            .arg("--project")
            .arg(workspace)
            .arg("--resume-session-id")
            .arg(&handle.subagent_id)
            .arg("--tool-policy")
            .arg(handle.tool_policy.as_str());
        if let Some(model) = model {
            command.arg("--model").arg(model);
        }
        return Ok((command, false));
    }
    Err("cannot resolve bundled subagent worker executable".into())
}

fn child_policy(role: &str, parent: WorkMode) -> WorkMode {
    let role = role.to_lowercase();
    if [
        "review",
        "research",
        "investigat",
        "audit",
        "inspect",
        "analy",
        "审查",
        "审阅",
        "调查",
        "调研",
        "审计",
        "分析",
        "只读",
    ]
    .iter()
    .any(|s| role.contains(s))
    {
        WorkMode::Plan
    } else {
        parent
    }
}

fn delegation_depth() -> u8 {
    std::env::var("HUB_DELEGATION_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

async fn prepare_workspace(
    handle: &SubagentHandle,
    request: &SubagentLaunchRequest,
    managed_root: &Path,
) -> Result<
    (
        PathBuf,
        Option<Arc<super::subagent_workspace::OwnedWorkspace>>,
    ),
    String,
> {
    if request.workspace_mode.as_deref() != Some("worktree") {
        return Ok((handle.workspace_path.clone(), None));
    }
    let owned = super::subagent_workspace::OwnedWorkspace::prepare(handle, managed_root)
        .await
        .map_err(|e| {
            format!("owned worktree creation failed: {e}; shared workspace fallback forbidden")
        })?;
    Ok((owned.path.clone(), Some(Arc::new(owned))))
}

/// Both worker entry points use the SessionLedger identity and steering queue.
pub fn register_tools(
    agent: &mut crate::domains::agent_cognitive::executor::AgentLoop,
    model: Option<&str>,
) {
    use super::tools::subagent::*;
    let parent = agent.session_id().to_string();
    let model = Some(
        model
            .map(str::to_owned)
            .unwrap_or_else(|| crate::config::llm_settings::read_llm_settings().model),
    );
    let runtime = Arc::new(
        SubagentRuntime::new(&parent, agent.steering_queue.clone())
            .with_model(model)
            .with_events(agent.emit_jsonl),
    );
    agent.ctx.provide("subagent_runtime", runtime.clone());
    if runtime.owner_depth < MAX_DELEGATION_DEPTH {
        agent.register_tool(Box::new(InvokeSubagentTool::new(runtime.clone(), &parent)));
        agent.register_tool(Box::new(CheckWorkerStatusTool::new(
            runtime.clone(),
            &parent,
        )));
        agent.register_tool(Box::new(InterveneWorkerTool::new(runtime.clone())));
        agent.register_tool(Box::new(CollectSubagentChangesTool::new(
            runtime.clone(),
            &parent,
        )));
        agent.register_tool(Box::new(RetrySubagentTool::new(runtime.clone())));
        agent.register_tool(Box::new(CancelSubagentTool::new(runtime)));
    }
}

fn bounded(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        result.push_str("…[truncated]");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_depth_timeout_and_bounded_prompts() {
        let mut request = SubagentLaunchRequest::new("parent", "researcher", "inspect code");
        assert!(validate_request(&request).is_ok());
        request.parent_depth = MAX_DELEGATION_DEPTH;
        assert!(validate_request(&request).unwrap_err().contains("depth"));
        request.parent_depth = 0;
        request.timeout = Duration::ZERO;
        assert!(validate_request(&request).unwrap_err().contains("timeout"));
        request.timeout = Duration::from_secs(1);
        request.prompt = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert!(validate_request(&request).unwrap_err().contains("prompt"));
    }

    #[test]
    fn terminal_state_cannot_be_overwritten() {
        let handle = SubagentHandle {
            replaces_subagent_id:None,
            subagent_id: "child".to_string(),
            parent_session_id: "parent".to_string(),
            correlation_id: None,
            role: "leaf".to_string(),
            depth: 1,
            created_at: 0,
            parent_run_id: "run".into(),
            workspace_path: std::env::temp_dir(),
            origin_project: std::env::temp_dir(),
            tool_policy: WorkMode::Plan,
        };
        let (cancel_tx, _) = watch::channel(false);
        let record = Arc::new(Mutex::new(Record {
                    recipe:None,
            process_birth: None,
            workspace: None,
            handle,
            state: SubagentState::Running,
            updated_at: 0,
            result: None,
            error: None,
            events: Vec::new(),
            stdin_tx: None,
            cancel_tx,
            request_identity: String::new(),
            process_id: None,
            execution_workspace: None,
            last_heartbeat_at: None,
            process_reaped: true,
        }));
        finish_record(&record, SubagentState::Cancelled, None, None);
        finish_record(
            &record,
            SubagentState::Succeeded,
            Some("late".to_string()),
            None,
        );
        let record = record.lock().unwrap();
        assert_eq!(record.state, SubagentState::Cancelled);
        assert!(record.result.is_none());
    }
}

#[cfg(test)]
#[path = "subagent_runtime_tests.rs"]
mod ownership_tests;
