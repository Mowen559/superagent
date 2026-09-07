use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::config::app_paths::AppPaths;
use crate::models::task::{Task, TaskStatus};
use crate::services::agent::AgentMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskControl {
    UserResponse {
        request_id: String,
        response: String,
    },
    Intervene {
        instruction: String,
    },
}

/// Capacity of each per-task broadcast channel. Late subscribers first replay
/// the persisted `events.jsonl`, so the live buffer only needs to cover the
/// window between subscribe and the next backlog read.
const EVENT_CHANNEL_CAPACITY: usize = 256;

pub struct TaskManager {
    tasks_dir: PathBuf,
    /// Per-task live event bus. Created lazily on first publish/subscribe and
    /// kept for the process lifetime; the persisted `.events.jsonl` is the
    /// durable record, this is only the realtime fan-out.
    event_bus: Mutex<HashMap<String, broadcast::Sender<(usize, AgentMessage)>>>,
    /// Serializes append with replay+subscribe so their boundary is exact.
    event_lock: Mutex<()>,
    /// Per-task input channel owned by a managed interactive Adapter.
    control_bus: Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<TaskControl>>>,
    /// Per-task cancellation signals. Dropped when the task reaches a terminal state.
    cancel_bus: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    /// Serializes persisted task state transitions. Process/event locks stay
    /// separate so cancellation never waits on SSE consumers.
    status_lock: Mutex<()>,
}

impl TaskManager {
    pub fn new(app_paths: &AppPaths) -> Self {
        let tasks_dir = app_paths.ensure_data_dir(&["tasks"]);
        Self {
            tasks_dir,
            event_bus: Mutex::new(HashMap::new()),
            event_lock: Mutex::new(()),
            control_bus: Mutex::new(HashMap::new()),
            cancel_bus: Mutex::new(HashMap::new()),
            status_lock: Mutex::new(()),
        }
    }

    pub fn create_task(
        &self,
        task_type: &str,
        project_path: &str,
        payload: Option<serde_json::Value>,
    ) -> (Task, tokio::sync::watch::Receiver<bool>) {
        let task_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let task = Task {
            id: task_id.clone(),
            task_type: task_type.to_string(),
            project_path: project_path.to_string(),
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
            duration_ms: None,
            error: None,
            payload,
            workspace_baseline: None,
            plan: None,
            change_set: None,
            verification: None,
            approval: None,
            receipt: None,
        };
        let _ = self.save_task(&task);
        self.append_log(&task_id, &format!("[System] Task {} created.", task_id));

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        self.cancel_bus
            .lock()
            .unwrap()
            .insert(task_id.clone(), cancel_tx);

        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        self.event_bus
            .lock()
            .unwrap()
            .insert(task_id.clone(), event_tx);

        (task, cancel_rx)
    }

    pub fn get_task(&self, task_id: &str) -> Option<Task> {
        let path = self.task_file_path(task_id);
        let backup_path = self.tasks_dir.join(format!("{}.json.bak", task_id));
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .or_else(|| {
                std::fs::read_to_string(backup_path)
                    .ok()
                    .and_then(|content| serde_json::from_str(&content).ok())
            })
    }

    pub fn update_status(&self, task_id: &str, status: TaskStatus, error: Option<String>) -> bool {
        self.try_update_status(task_id, status, error)
    }

    pub fn update_development_run(
        &self,
        task_id: &str,
        change_set: Option<serde_json::Value>,
        approval: Option<String>,
    ) {
        let _transition_guard = self.status_lock.lock().unwrap();
        if let Some(mut task) = self.get_task(task_id) {
            if change_set.is_some() {
                task.change_set = change_set;
            }
            if approval.is_some() {
                task.approval = approval;
            }
            task.updated_at = chrono::Utc::now().timestamp_millis();
            let _ = self.save_task(&task);
        }
    }

    pub fn update_workspace_baseline(&self, task_id: &str, baseline: String) -> bool {
        let _transition_guard = self.status_lock.lock().unwrap();
        let Some(mut task) = self.get_task(task_id) else {
            return false;
        };
        if task.status.is_terminal() {
            return false;
        }
        task.workspace_baseline = Some(baseline);
        task.updated_at = chrono::Utc::now().timestamp_millis();
        self.save_task(&task).is_ok()
    }

    pub fn update_verification(&self, task_id: &str, verification: serde_json::Value) -> bool {
        let _transition_guard = self.status_lock.lock().unwrap();
        let Some(mut task) = self.get_task(task_id) else {
            return false;
        };
        task.verification = Some(verification);
        task.updated_at = chrono::Utc::now().timestamp_millis();
        self.save_task(&task).is_ok()
    }

    /// Attach a structured result before committing the terminal status.
    /// Terminal tasks are immutable so a late worker cannot rewrite the
    /// evidence associated with a cancelled or otherwise finished task.
    pub fn update_receipt(&self, task_id: &str, receipt: serde_json::Value) -> bool {
        let _transition_guard = self.status_lock.lock().unwrap();
        let Some(mut task) = self.get_task(task_id) else {
            return false;
        };
        if task.status.is_terminal() {
            return false;
        }
        task.receipt = Some(receipt);
        task.updated_at = chrono::Utc::now().timestamp_millis();
        self.save_task(&task).is_ok()
    }

    /// Atomically persist an evidence receipt and commit `completed`. This
    /// closes the final race with cancellation: either cancellation wins and
    /// no successful receipt is attached, or the verified completion wins.
    pub fn complete_with_receipt(&self, task_id: &str, receipt: serde_json::Value) -> bool {
        let _transition_guard = self.status_lock.lock().unwrap();
        let Some(mut task) = self.get_task(task_id) else {
            return false;
        };
        if task.status.is_terminal() {
            return false;
        }
        let now = chrono::Utc::now().timestamp_millis();
        task.receipt = Some(receipt);
        task.status = TaskStatus::Completed;
        task.updated_at = now;
        task.duration_ms = Some(now - task.created_at);
        if self.save_task(&task).is_err() {
            return false;
        }
        self.append_event(
            task_id,
            &AgentMessage::Status {
                status: TaskStatus::Completed.as_str().to_string(),
                session_id: String::new(),
            },
        );
        self.event_bus.lock().unwrap().remove(task_id);
        self.cancel_bus.lock().unwrap().remove(task_id);
        self.remove_control(task_id);
        true
    }

    fn try_update_status(&self, task_id: &str, status: TaskStatus, error: Option<String>) -> bool {
        let _transition_guard = self.status_lock.lock().unwrap();
        if let Some(mut task) = self.get_task(task_id) {
            let now = chrono::Utc::now().timestamp_millis();
            let terminal_error = error.clone();

            // Prevent terminal states from being overwritten
            let is_currently_terminal = task.status.is_terminal();
            if is_currently_terminal {
                return false; // Do not overwrite a terminal state
            }
            task.status = status.clone();
            task.updated_at = now;

            let is_terminal = status.is_terminal();
            if is_terminal {
                task.duration_ms = Some(now - task.created_at);
            }
            if let Some(e) = error {
                task.error = Some(e);
            }
            if self.save_task(&task).is_err() {
                return false;
            }

            if is_terminal {
                if let Some(content) = terminal_error.filter(|content| !content.is_empty()) {
                    self.append_event(task_id, &AgentMessage::Error { content });
                }
                self.append_event(
                    task_id,
                    &AgentMessage::Status {
                        status: status.as_str().to_string(),
                        session_id: String::new(),
                    },
                );
                // Drop the event bus sender to cleanly terminate any SSE subscribers
                self.event_bus.lock().unwrap().remove(task_id);
                // Drop the cancel token as it's no longer needed
                self.cancel_bus.lock().unwrap().remove(task_id);
                self.remove_control(task_id);
            }
            return true;
        }
        false
    }

    pub fn append_log(&self, task_id: &str, log: &str) {
        let log_path = self.log_file_path(task_id);
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(file, "{}", log);
        }
    }

    pub fn get_logs(&self, task_id: &str) -> String {
        let log_path = self.log_file_path(task_id);
        std::fs::read_to_string(&log_path).unwrap_or_default()
    }

    /// Append a structured agent event: persist one JSON line to
    /// `{task_id}.events.jsonl` and fan it out to any live SSE subscribers.
    /// Also mirrors a human-readable line into the legacy `.log` for backward
    /// compatibility with the old text-only frontend.
    pub fn append_event(&self, task_id: &str, event: &AgentMessage) {
        let _event_guard = self
            .event_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sequence = self.persisted_event_count(task_id).saturating_add(1);
        if let Ok(line) = serde_json::to_string(event) {
            let path = self.events_file_path(task_id);
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(file, "{}", line);
            }
        }
        self.append_log(task_id, &human_readable(event));
        // Fan out to live subscribers without holding the lock across the send.
        let sender = {
            let bus = self.event_bus.lock().unwrap();
            bus.get(task_id).cloned()
        };
        if let Some(tx) = sender {
            let _ = tx.send((sequence, event.clone()));
        }
    }

    /// Read back all persisted structured events for a task, in order.
    pub fn get_events(&self, task_id: &str) -> Vec<(usize, AgentMessage)> {
        let path = self.events_file_path(task_id);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .filter_map(|(i, l)| {
                serde_json::from_str::<AgentMessage>(l)
                    .ok()
                    .map(|msg| (i + 1, msg))
            })
            .collect()
    }

    /// Atomically capture persisted events and subscribe to future events.
    /// Events appended before this boundary are only in `backlog`; events
    /// appended after it are only delivered through `receiver`.
    pub fn snapshot_and_subscribe(
        &self,
        task_id: &str,
        after_sequence: usize,
    ) -> (
        Vec<(usize, AgentMessage)>,
        Option<broadcast::Receiver<(usize, AgentMessage)>>,
    ) {
        let _event_guard = self
            .event_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let receiver = self
            .event_bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(task_id)
            .map(|tx| tx.subscribe());
        let backlog = self
            .get_events(task_id)
            .into_iter()
            .filter(|(sequence, _)| *sequence > after_sequence)
            .collect();
        (backlog, receiver)
    }

    pub fn events_after(&self, task_id: &str, after_sequence: usize) -> Vec<(usize, AgentMessage)> {
        self.get_events(task_id)
            .into_iter()
            .filter(|(sequence, _)| *sequence > after_sequence)
            .collect()
    }

    pub fn register_control(
        &self,
        task_id: &str,
        sender: tokio::sync::mpsc::UnboundedSender<TaskControl>,
    ) -> bool {
        let Some(task) = self.get_task(task_id) else {
            return false;
        };
        if task.status.is_terminal() {
            return false;
        }
        self.control_bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(task_id.to_string(), sender);
        true
    }

    pub fn send_control(&self, task_id: &str, control: TaskControl) -> bool {
        self.control_bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(task_id)
            .is_some_and(|sender| sender.send(control).is_ok())
    }

    pub fn remove_control(&self, task_id: &str) {
        self.control_bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(task_id);
    }

    pub fn cancel_task(&self, task_id: &str) -> bool {
        let cancel_tx = self.cancel_bus.lock().unwrap().get(task_id).cloned();
        if !self.try_update_status(task_id, TaskStatus::Cancelled, None) {
            return false;
        }
        if let Some(tx) = cancel_tx {
            let _ = tx.send(true);
        }
        self.append_log(
            task_id,
            &format!("[System] Task {} cancelled by user.", task_id),
        );
        true
    }

    pub fn list_tasks(&self) -> Vec<Task> {
        let mut tasks = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.tasks_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(task) = serde_json::from_str::<Task>(&content) {
                            tasks.push(task);
                        }
                    }
                }
            }
        }
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        tasks
    }

    pub fn latest_project_analysis(&self, project_path: &str) -> Option<Task> {
        self.latest_project_task(project_path, "analysis")
    }

    pub fn latest_project_task(&self, project_path: &str, task_type: &str) -> Option<Task> {
        self.list_tasks().into_iter().find(|task| {
            task.task_type == task_type && same_project_path(&task.project_path, project_path)
        })
    }

    fn save_task(&self, task: &Task) -> std::io::Result<()> {
        let path = self.task_file_path(&task.id);
        let temp_path = self.tasks_dir.join(format!("{}.json.tmp", task.id));
        let backup_path = self.tasks_dir.join(format!("{}.json.bak", task.id));
        let json = serde_json::to_string_pretty(task)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(&temp_path, json)?;
        if path.exists() {
            let _ = std::fs::remove_file(&backup_path);
            std::fs::rename(&path, &backup_path)?;
            if let Err(error) = std::fs::rename(&temp_path, &path) {
                let _ = std::fs::rename(&backup_path, &path);
                return Err(error);
            }
            let _ = std::fs::remove_file(backup_path);
        } else {
            std::fs::rename(&temp_path, &path)?;
        }
        Ok(())
    }

    fn task_file_path(&self, task_id: &str) -> PathBuf {
        self.tasks_dir.join(format!("{}.json", task_id))
    }

    fn log_file_path(&self, task_id: &str) -> PathBuf {
        self.tasks_dir.join(format!("{}.log", task_id))
    }

    fn events_file_path(&self, task_id: &str) -> PathBuf {
        self.tasks_dir.join(format!("{}.events.jsonl", task_id))
    }

    fn persisted_event_count(&self, task_id: &str) -> usize {
        std::fs::read_to_string(self.events_file_path(task_id))
            .map(|content| {
                content
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count()
            })
            .unwrap_or(0)
    }
}

fn same_project_path(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn manager() -> (tempfile::TempDir, TaskManager) {
        let directory = tempdir().unwrap();
        let manager = TaskManager {
            tasks_dir: directory.path().to_path_buf(),
            event_bus: Mutex::new(HashMap::new()),
            event_lock: Mutex::new(()),
            control_bus: Mutex::new(HashMap::new()),
            cancel_bus: Mutex::new(HashMap::new()),
            status_lock: Mutex::new(()),
        };
        (directory, manager)
    }

    #[test]
    fn cancellation_signals_adapter_and_terminal_state_cannot_be_overwritten() {
        let (_directory, manager) = manager();
        let (task, mut cancel_rx) = manager.create_task("agent", "", None);
        manager.update_status(&task.id, TaskStatus::Running, None);

        assert!(manager.cancel_task(&task.id));
        assert!(*cancel_rx.borrow_and_update());
        manager.update_status(&task.id, TaskStatus::Completed, None);
        assert_eq!(
            manager.get_task(&task.id).unwrap().status,
            TaskStatus::Cancelled
        );
        assert!(matches!(
            manager.get_events(&task.id).last(),
            Some((_, AgentMessage::Status { status, .. })) if status == "cancelled"
        ));
    }

    #[test]
    fn cancelling_an_already_terminal_task_reports_no_transition() {
        let (_directory, manager) = manager();
        let (task, _cancel_rx) = manager.create_task("agent", "", None);
        manager.update_status(&task.id, TaskStatus::Completed, None);
        assert!(!manager.cancel_task(&task.id));
        assert_eq!(
            manager.get_task(&task.id).unwrap().status,
            TaskStatus::Completed
        );
    }

    #[test]
    fn task_read_recovers_from_atomic_write_backup() {
        let (_directory, manager) = manager();
        let (task, _cancel_rx) = manager.create_task("agent", "", None);
        let primary = manager.task_file_path(&task.id);
        let backup = manager.tasks_dir.join(format!("{}.json.bak", task.id));
        std::fs::rename(primary, backup).unwrap();

        assert_eq!(manager.get_task(&task.id).unwrap().id, task.id);
    }

    #[test]
    fn receipt_is_persisted_before_completion_and_is_immutable_afterwards() {
        let (_directory, manager) = manager();
        let (task, _cancel_rx) = manager.create_task("analysis", "", None);
        manager.update_status(&task.id, TaskStatus::Running, None);
        assert!(manager.update_receipt(&task.id, serde_json::json!({ "indexed": true })));
        manager.update_status(&task.id, TaskStatus::Completed, None);
        assert_eq!(
            manager
                .get_task(&task.id)
                .unwrap()
                .receipt
                .unwrap()
                .get("indexed")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(!manager.update_receipt(&task.id, serde_json::json!({ "indexed": false })));
    }

    #[test]
    fn receipt_and_success_terminal_state_commit_together() {
        let (_directory, manager) = manager();
        let (task, _cancel_rx) = manager.create_task("analysis", "", None);
        manager.update_status(&task.id, TaskStatus::Running, None);
        assert!(manager
            .complete_with_receipt(&task.id, serde_json::json!({ "kind": "project-analysis" })));
        let completed = manager.get_task(&task.id).unwrap();
        assert_eq!(completed.status, TaskStatus::Completed);
        assert_eq!(
            completed
                .receipt
                .as_ref()
                .and_then(|receipt| receipt.get("kind"))
                .and_then(serde_json::Value::as_str),
            Some("project-analysis")
        );
        assert!(!manager.cancel_task(&task.id));
    }

    #[test]
    fn replay_and_live_subscription_have_one_monotonic_boundary() {
        let (_directory, manager) = manager();
        let (task, _cancel_rx) = manager.create_task("agent", "", None);
        manager.append_event(
            &task.id,
            &AgentMessage::Text {
                content: "before".into(),
            },
        );

        let (backlog, mut receiver) = manager.snapshot_and_subscribe(&task.id, 0);
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog[0].0, 1);
        assert!(receiver.as_mut().unwrap().try_recv().is_err());

        manager.append_event(
            &task.id,
            &AgentMessage::Text {
                content: "after".into(),
            },
        );
        let (sequence, event) = receiver.as_mut().unwrap().try_recv().unwrap();
        assert_eq!(sequence, 2);
        assert!(matches!(event, AgentMessage::Text { content } if content == "after"));
    }

    #[test]
    fn managed_task_control_is_correlated_and_removed_at_terminal_state() {
        let (_directory, manager) = manager();
        let (task, _cancel_rx) = manager.create_task("agent", "", None);
        manager.update_status(&task.id, TaskStatus::Running, None);
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        assert!(manager.register_control(&task.id, sender));
        assert!(manager.send_control(
            &task.id,
            TaskControl::UserResponse {
                request_id: "request-1".into(),
                response: "yes".into(),
            }
        ));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            TaskControl::UserResponse { request_id, response }
                if request_id == "request-1" && response == "yes"
        ));
        manager.update_status(&task.id, TaskStatus::Completed, None);
        assert!(!manager.send_control(
            &task.id,
            TaskControl::Intervene {
                instruction: "late".into()
            }
        ));
    }
}

/// Render an [`AgentMessage`] as a single human-readable log line, mirroring
/// the shape the legacy text log used so existing viewers keep working.
fn human_readable(event: &AgentMessage) -> String {
    match event {
        AgentMessage::Text { content } => content.clone(),
        AgentMessage::Thinking { content } => format!("[Thinking] {content}"),
        AgentMessage::ToolUse { tool, .. } => format!("[Tool] {tool}"),
        AgentMessage::ToolResult { tool, output, .. } => {
            if tool.is_empty() {
                format!("[ToolResult] {output}")
            } else {
                format!("[ToolResult:{tool}] {output}")
            }
        }
        AgentMessage::Status { status, .. } => format!("[Status] {status}"),
        AgentMessage::Error { content } => format!("[Error] {content}"),
        AgentMessage::Log { level, content } => format!("[{level}] {content}"),
        AgentMessage::InteractionRequest {
            request_id,
            question,
            ..
        } => format!("[NeedsInput:{request_id}] {question}"),
    }
}
