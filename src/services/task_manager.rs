use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::config::app_paths::AppPaths;
use crate::models::task::{Task, TaskStatus};
use crate::services::agent::AgentMessage;

/// Capacity of each per-task broadcast channel. Late subscribers first replay
/// the persisted `events.jsonl`, so the live buffer only needs to cover the
/// window between subscribe and the next backlog read.
const EVENT_CHANNEL_CAPACITY: usize = 256;

pub struct TaskManager {
    tasks_dir: PathBuf,
    /// Per-task live event bus. Created lazily on first publish/subscribe and
    /// kept for the process lifetime; the persisted `.events.jsonl` is the
    /// durable record, this is only the realtime fan-out.
    event_bus: Mutex<HashMap<String, broadcast::Sender<AgentMessage>>>,
    /// Per-task cancellation signals. Dropped when the task reaches a terminal state.
    cancel_bus: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
}

impl TaskManager {
    pub fn new(app_paths: &AppPaths) -> Self {
        let tasks_dir = app_paths.ensure_data_dir(&["tasks"]);
        Self {
            tasks_dir,
            event_bus: Mutex::new(HashMap::new()),
            cancel_bus: Mutex::new(HashMap::new()),
        }
    }

    pub fn create_task(&self, task_type: &str, project_path: &str, payload: Option<serde_json::Value>) -> (Task, tokio::sync::watch::Receiver<bool>) {
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
        };
        self.save_task(&task);
        self.append_log(&task_id, &format!("[System] Task {} created.", task_id));

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        self.cancel_bus.lock().unwrap().insert(task_id.clone(), cancel_tx);

        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        self.event_bus.lock().unwrap().insert(task_id.clone(), event_tx);

        (task, cancel_rx)
    }

    pub fn get_task(&self, task_id: &str) -> Option<Task> {
        let path = self.task_file_path(task_id);
        if !path.exists() {
            return None;
        }
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
    }

    pub fn update_status(&self, task_id: &str, status: TaskStatus, error: Option<String>) {
        if let Some(mut task) = self.get_task(task_id) {
            let now = chrono::Utc::now().timestamp_millis();
            task.status = status.clone();
            task.updated_at = now;
            let is_terminal = matches!(status, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled);
            if is_terminal {
                task.duration_ms = Some(now - task.created_at);
            }
            if let Some(e) = error {
                task.error = Some(e);
            }
            self.save_task(&task);

            if is_terminal {
                // Drop the event bus sender to cleanly terminate any SSE subscribers
                self.event_bus.lock().unwrap().remove(task_id);
                // Drop the cancel token as it's no longer needed
                self.cancel_bus.lock().unwrap().remove(task_id);
            }
        }
    }

    pub fn append_log(&self, task_id: &str, log: &str) {
        let log_path = self.log_file_path(task_id);
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
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
        if let Ok(line) = serde_json::to_string(event) {
            let path = self.events_file_path(task_id);
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
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
            let _ = tx.send(event.clone());
        }
    }

    /// Read back all persisted structured events for a task, in order.
    pub fn get_events(&self, task_id: &str) -> Vec<AgentMessage> {
        let path = self.events_file_path(task_id);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<AgentMessage>(l).ok())
            .collect()
    }

    /// Subscribe to the live event stream for a task, creating the channel on
    /// first use. Callers should replay [`Self::get_events`] first to catch up
    /// on backlog, then drain this receiver for realtime updates.
    pub fn subscribe_events(&self, task_id: &str) -> Option<broadcast::Receiver<AgentMessage>> {
        let bus = self.event_bus.lock().unwrap();
        bus.get(task_id).map(|tx| tx.subscribe())
    }

    pub fn cancel_task(&self, task_id: &str) {
        if let Some(tx) = self.cancel_bus.lock().unwrap().get(task_id) {
            let _ = tx.send(true);
        }
        self.update_status(task_id, TaskStatus::Cancelled, None);
        self.append_log(task_id, &format!("[System] Task {} cancelled by user.", task_id));
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

    fn save_task(&self, task: &Task) {
        let path = self.task_file_path(&task.id);
        if let Ok(json) = serde_json::to_string_pretty(task) {
            std::fs::write(&path, json).ok();
        }
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
    }
}
