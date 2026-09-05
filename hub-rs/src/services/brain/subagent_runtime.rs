use std::collections::HashMap;
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

const MAX_PROMPT_CHARS: usize = 16_000;
const MAX_SYSTEM_PROMPT_CHARS: usize = 32_000;
const MAX_ROLE_CHARS: usize = 120;
const MAX_RESULT_CHARS: usize = 32_000;
const MAX_EVENT_CHARS: usize = 2_000;
const MAX_EVENTS_PER_CHILD: usize = 200;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30 * 60;
const MAX_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
const MAX_GLOBAL_RUNNING: usize = 8;
const MAX_PARENT_RUNNING: usize = 4;
const MAX_DELEGATION_DEPTH: u8 = 3;

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
    pub parent_session_id: String,
    pub correlation_id: Option<String>,
    pub role: String,
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub parent_depth: u8,
    pub timeout: Duration,
}

impl SubagentLaunchRequest {
    pub fn new(
        parent_session_id: impl Into<String>,
        role: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            parent_session_id: parent_session_id.into(),
            correlation_id: None,
            role: role.into(),
            prompt: prompt.into(),
            system_prompt: None,
            parent_depth: 0,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentHandle {
    pub subagent_id: String,
    pub parent_session_id: String,
    pub correlation_id: Option<String>,
    pub role: String,
    pub depth: u8,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    pub at: i64,
    pub kind: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSnapshot {
    pub handle: SubagentHandle,
    pub state: SubagentState,
    pub updated_at: i64,
    pub result: Option<String>,
    pub error: Option<String>,
    pub recent_events: Vec<SubagentEvent>,
}

struct Record {
    handle: SubagentHandle,
    state: SubagentState,
    updated_at: i64,
    result: Option<String>,
    error: Option<String>,
    events: Vec<SubagentEvent>,
    stdin_tx: Option<mpsc::UnboundedSender<IpcMessage>>,
    cancel_tx: watch::Sender<bool>,
}

/// Owns child admission and lifecycle. Process execution is an internal
/// Adapter; callers only use handles and lifecycle operations.
pub struct SubagentRuntime {
    records: RwLock<HashMap<String, Arc<Mutex<Record>>>>,
    owner_session_id: String,
    owner_queue: Arc<SteeringQueue>,
}

impl Default for SubagentRuntime {
    fn default() -> Self {
        let owner_session_id = std::env::var("HUB_AGENT_SESSION_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "default".to_string());
        Self {
            records: RwLock::new(HashMap::new()),
            owner_queue: Arc::new(SteeringQueue::for_session(&owner_session_id)),
            owner_session_id,
        }
    }
}

impl SubagentRuntime {
    pub fn new(owner_session_id: impl Into<String>, owner_queue: Arc<SteeringQueue>) -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
            owner_session_id: owner_session_id.into(),
            owner_queue,
        }
    }

    pub fn launch(
        self: &Arc<Self>,
        request: SubagentLaunchRequest,
    ) -> Result<SubagentHandle, String> {
        validate_request(&request)?;
        if request.parent_session_id != self.owner_session_id {
            return Err("subagent parent does not match runtime owner".to_string());
        }
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
                    && record.handle.correlation_id.as_deref() == Some(correlation_id)
            }) {
                return Ok(record
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .handle
                    .clone());
            }
        }
        let running_global = records
            .values()
            .filter(|record| {
                !record
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .state
                    .is_terminal()
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
                    && !record.state.is_terminal()
            })
            .count();
        if running_for_parent >= MAX_PARENT_RUNNING {
            return Err(format!(
                "subagent parent concurrency limit reached ({MAX_PARENT_RUNNING})"
            ));
        }

        let now = Utc::now().timestamp_millis();
        let handle = SubagentHandle {
            subagent_id: Uuid::new_v4().to_string(),
            parent_session_id: request.parent_session_id.clone(),
            correlation_id: request.correlation_id.clone(),
            role: request.role.clone(),
            depth,
            created_at: now,
        };
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let record = Arc::new(Mutex::new(Record {
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
        }));
        records.insert(handle.subagent_id.clone(), record.clone());
        drop(records);

        let runtime = self.clone();
        tokio::spawn(async move {
            runtime.run_child(record, request, cancel_rx).await;
        });
        Ok(handle)
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
        let mut command = worker_command();
        command
            .env("HUB_AGENT_SESSION_ID", &handle.subagent_id)
            .env("HUB_PARENT_SESSION_ID", &handle.parent_session_id)
            .env("HUB_DELEGATION_DEPTH", handle.depth.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
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
        let Some(mut stdin) = child.stdin.take() else {
            finish_record(
                &record,
                SubagentState::Failed,
                None,
                Some("child stdin unavailable".to_string()),
            );
            let _ = child.kill().await;
            self.deliver_parent_result(&handle, &record);
            return;
        };
        let Some(stdout) = child.stdout.take() else {
            finish_record(
                &record,
                SubagentState::Failed,
                None,
                Some("child stdout unavailable".to_string()),
            );
            let _ = child.kill().await;
            self.deliver_parent_result(&handle, &record);
            return;
        };
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel();
        {
            let mut guard = record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.stdin_tx = Some(stdin_tx.clone());
            guard.state = SubagentState::Running;
            guard.updated_at = Utc::now().timestamp_millis();
            push_event(&mut guard, "running", "child process started".to_string());
        }
        tokio::spawn(async move {
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
        let _ = stdin_tx.send(IpcMessage::Spawn {
            role: request.role,
            system_prompt: request.system_prompt,
            prompt: request.prompt,
        });

        let read_result = read_child_output(stdout, record.clone());
        tokio::select! {
            output = tokio::time::timeout(request.timeout, read_result) => {
                match output {
                    Ok(Some(Ok(result))) => finish_record(&record, SubagentState::Succeeded, Some(result), None),
                    Ok(Some(Err(error))) => finish_record(&record, SubagentState::Failed, None, Some(error)),
                    Ok(None) => finish_record(&record, SubagentState::Failed, None, Some("child exited without a terminal result".to_string())),
                    Err(_) => finish_record(&record, SubagentState::TimedOut, None, Some("subagent timed out".to_string())),
                }
            }
            _ = cancel_rx.wait_for(|cancelled| *cancelled) => {
                finish_record(&record, SubagentState::Cancelled, None, Some("subagent cancelled by parent".to_string()));
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
        self.deliver_parent_result(&handle, &record);
    }

    fn deliver_parent_result(&self, handle: &SubagentHandle, record: &Arc<Mutex<Record>>) {
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
                    let mut record = record
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    record.updated_at = Utc::now().timestamp_millis();
                }
                _ => {}
            }
        } else {
            let mut record = record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            push_event(&mut record, "log", bounded(&line, MAX_EVENT_CHARS));
        }
    }
    None
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
    }
}

fn validate_request(request: &SubagentLaunchRequest) -> Result<(), String> {
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

fn worker_command() -> Command {
    if let Ok(path) = std::env::var("HUB_SUPERAGENT_BIN") {
        if !path.trim().is_empty() {
            return Command::new(path);
        }
    }
    if let Ok(current) = std::env::current_exe() {
        if current
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("superagent"))
        {
            return Command::new(current);
        }
    }
    let mut command = Command::new("cargo");
    command.arg("run").arg("--bin").arg("superagent");
    command
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
            subagent_id: "child".to_string(),
            parent_session_id: "parent".to_string(),
            correlation_id: None,
            role: "leaf".to_string(),
            depth: 1,
            created_at: 0,
        };
        let (cancel_tx, _) = watch::channel(false);
        let record = Arc::new(Mutex::new(Record {
            handle,
            state: SubagentState::Running,
            updated_at: 0,
            result: None,
            error: None,
            events: Vec::new(),
            stdin_tx: None,
            cancel_tx,
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
