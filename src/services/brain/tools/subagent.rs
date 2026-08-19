//! 子智能体委派与强监督工具集 (Subagent & Supervision Tools)
//! 包含:
//! 1. InvokeSubagentTool: 派发后台子智能体 Worker，非阻塞立即返回 Worker_ID
//! 2. CheckWorkerStatusTool: 监工查询 Worker 的当前状态与事件流/日志
//! 3. InterveneWorkerTool: 向指定 Worker 管道注入最高优先级控制指令

use crate::services::brain::executor::ToolExecutor;
use crate::services::brain::ipc::IpcMessage;
use chrono::Local;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEvent {
    pub timestamp: String,
    pub event_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkerState {
    Running,
    Completed,
    Failed,
}

pub struct WorkerInfo {
    pub id: String,
    pub role: String,
    pub prompt: String,
    pub state: Arc<Mutex<WorkerState>>,
    pub result: Arc<Mutex<Option<String>>>,
    pub events: Arc<Mutex<Vec<WorkerEvent>>>,
    pub stdin_tx: tokio::sync::mpsc::UnboundedSender<IpcMessage>,
}

lazy_static! {
    pub static ref WORKER_MANAGER: Arc<RwLock<HashMap<String, WorkerInfo>>> =
        Arc::new(RwLock::new(HashMap::new()));
    static ref WORKER_COUNTER: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(1);
}

pub fn spawn_worker(
    role: String,
    system_prompt: Option<String>,
    prompt: String,
) -> Result<String, String> {
    let worker_num = WORKER_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let worker_id = format!("worker_{}", worker_num);

    let state = Arc::new(Mutex::new(WorkerState::Running));
    let result_store = Arc::new(Mutex::new(None));
    let events = Arc::new(Mutex::new(Vec::new()));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<IpcMessage>();

    let now = Local::now().format("%H:%M:%S").to_string();
    events.lock().unwrap().push(WorkerEvent {
        timestamp: now,
        event_type: "spawn".to_string(),
        content: format!("Worker spawned with role: '{}'", role),
    });

    let state_clone = state.clone();
    let result_store_clone = result_store.clone();
    let events_clone = events.clone();

    let role_clone = role.clone();
    let prompt_clone = prompt.clone();
    let system_prompt_clone = system_prompt;

    let tx_for_spawn = tx.clone();
    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        let mut child = match Command::new("cargo")
            .arg("run")
            .arg("--bin")
            .arg("superagent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("Failed to spawn child subagent process: {}", e);
                *state_clone.lock().unwrap() = WorkerState::Failed;
                *result_store_clone.lock().unwrap() = Some(err_msg.clone());
                events_clone.lock().unwrap().push(WorkerEvent {
                    timestamp: Local::now().format("%H:%M:%S").to_string(),
                    event_type: "error".to_string(),
                    content: err_msg,
                });
                return;
            }
        };

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        // Stdin 写入后台任务
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Ok(json_str) = serde_json::to_string(&msg) {
                    let data = format!("{}\n", json_str);
                    if stdin.write_all(data.as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = stdin.flush().await;
                }
            }
        });

        // 初始 Spawn 设定
        let init_msg = IpcMessage::Spawn {
            role: role_clone,
            system_prompt: system_prompt_clone,
            prompt: prompt_clone,
        };
        let _ = tx_for_spawn.send(init_msg);

        // Stdout 读取后台任务
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let timestamp = Local::now().format("%H:%M:%S").to_string();

            if let Ok(ipc_msg) = serde_json::from_str::<IpcMessage>(&line) {
                match ipc_msg {
                    IpcMessage::Event { event_type, content } => {
                        events_clone.lock().unwrap().push(WorkerEvent {
                            timestamp,
                            event_type,
                            content,
                        });
                    }
                    IpcMessage::Result(res) => {
                        *state_clone.lock().unwrap() = WorkerState::Completed;
                        *result_store_clone.lock().unwrap() = Some(res.clone());
                        events_clone.lock().unwrap().push(WorkerEvent {
                            timestamp,
                            event_type: "result".to_string(),
                            content: res,
                        });
                        break;
                    }
                    IpcMessage::Error(err) => {
                        *state_clone.lock().unwrap() = WorkerState::Failed;
                        *result_store_clone.lock().unwrap() = Some(err.clone());
                        events_clone.lock().unwrap().push(WorkerEvent {
                            timestamp,
                            event_type: "error".to_string(),
                            content: err,
                        });
                        break;
                    }
                    _ => {}
                }
            } else {
                events_clone.lock().unwrap().push(WorkerEvent {
                    timestamp,
                    event_type: "raw_log".to_string(),
                    content: line,
                });
            }
        }

        let _ = child.kill().await;
        let _ = child.wait().await;

        let mut st = state_clone.lock().unwrap();
        if *st == WorkerState::Running {
            *st = WorkerState::Completed;
        }
    });

    let info = WorkerInfo {
        id: worker_id.clone(),
        role,
        prompt,
        state,
        result: result_store,
        events,
        stdin_tx: tx,
    };

    WORKER_MANAGER
        .write()
        .unwrap()
        .insert(worker_id.clone(), info);
    Ok(worker_id)
}

pub fn query_worker_status(worker_id: Option<&str>) -> Value {
    let mgr = WORKER_MANAGER.read().unwrap();
    if let Some(id) = worker_id {
        if let Some(info) = mgr.get(id) {
            let state = info.state.lock().unwrap().clone();
            let result = info.result.lock().unwrap().clone();
            let events = info.events.lock().unwrap().clone();
            json!({
                "worker_id": info.id,
                "role": info.role,
                "prompt": info.prompt,
                "state": state,
                "result": result,
                "events_count": events.len(),
                "recent_events": events.iter().rev().take(20).rev().collect::<Vec<_>>()
            })
        } else {
            json!({ "error": format!("Worker '{}' not found", id) })
        }
    } else {
        let list: Vec<Value> = mgr
            .values()
            .map(|info| {
                let state = info.state.lock().unwrap().clone();
                json!({
                    "worker_id": info.id,
                    "role": info.role,
                    "state": state,
                    "events_count": info.events.lock().unwrap().len()
                })
            })
            .collect();
        json!({ "workers": list })
    }
}

pub fn inject_worker_intervention(worker_id: &str, instruction: &str) -> Result<Value, String> {
    let mgr = WORKER_MANAGER.read().unwrap();
    if let Some(info) = mgr.get(worker_id) {
        let state = info.state.lock().unwrap().clone();
        if state != WorkerState::Running {
            return Err(format!(
                "Worker '{}' is not running (state: {:?})",
                worker_id, state
            ));
        }

        let timestamp = Local::now().format("%H:%M:%S").to_string();
        info.events.lock().unwrap().push(WorkerEvent {
            timestamp,
            event_type: "intervene_sent".to_string(),
            content: instruction.to_string(),
        });

        info.stdin_tx
            .send(IpcMessage::Intervene(instruction.to_string()))
            .map_err(|e| format!("Failed to send intervention via IPC channel: {}", e))?;

        Ok(json!({
            "status": "success",
            "worker_id": worker_id,
            "message": format!("High-priority intervention sent to '{}'", worker_id)
        }))
    } else {
        Err(format!("Worker '{}' not found", worker_id))
    }
}

// ------------------- TOOL IMPLEMENTATIONS -------------------

pub struct InvokeSubagentTool;

impl ToolExecutor for InvokeSubagentTool {
    fn name(&self) -> &'static str {
        "invoke_subagent"
    }

    fn description(&self) -> &'static str {
        "Spawn a background subagent worker asynchronously. Immediately returns a Worker_ID. System settings and specialized prompts can be passed to isolate identity."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Prompt": {
                    "type": "string",
                    "description": "A clear, actionable task description for the subagent."
                },
                "Role": {
                    "type": "string",
                    "description": "A short role description for the subagent, e.g., 'Researcher' or 'Coder'."
                },
                "SystemPrompt": {
                    "type": "string",
                    "description": "Optional specialized system instructions for identity/prompt isolation."
                }
            },
            "required": ["Prompt", "Role"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let prompt = args.get("Prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let role = args.get("Role").and_then(|v| v.as_str()).unwrap_or("Subagent").to_string();
        let system_prompt = args.get("SystemPrompt").and_then(|v| v.as_str()).map(|s| s.to_string());

        let worker_id = spawn_worker(role.clone(), system_prompt, prompt)?;

        Ok(json!({
            "status": "spawned",
            "worker_id": worker_id,
            "role": role,
            "message": format!("Subagent '{}' spawned successfully. Use 'check_worker_status' with worker_id '{}' to monitor progress or 'intervene_worker' to send intervention instructions.", worker_id, worker_id)
        }))
    }
}

pub struct CheckWorkerStatusTool;

impl ToolExecutor for CheckWorkerStatusTool {
    fn name(&self) -> &'static str {
        "check_worker_status"
    }

    fn description(&self) -> &'static str {
        "Check the current state, progress, and recent event log of a subagent worker by Worker_ID, or list all workers if Worker_ID is omitted."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Worker_ID": {
                    "type": "string",
                    "description": "The target Worker_ID to check. If omitted, returns a summary list of all workers."
                }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let worker_id = args.get("Worker_ID").and_then(|v| v.as_str());
        Ok(query_worker_status(worker_id))
    }
}

pub struct InterveneWorkerTool;

impl ToolExecutor for InterveneWorkerTool {
    fn name(&self) -> &'static str {
        "intervene_worker"
    }

    fn description(&self) -> &'static str {
        "Inject a high-priority intervention instruction into a running subagent worker's IPC stream to correct its path or redirect its focus."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Worker_ID": {
                    "type": "string",
                    "description": "The target Worker_ID to intervene."
                },
                "Instruction": {
                    "type": "string",
                    "description": "The high-priority instruction or redirection for the worker."
                }
            },
            "required": ["Worker_ID", "Instruction"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let worker_id = args
            .get("Worker_ID")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Worker_ID parameter is required".to_string())?;
        let instruction = args
            .get("Instruction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Instruction parameter is required".to_string())?;

        inject_worker_intervention(worker_id, instruction)
    }
}
