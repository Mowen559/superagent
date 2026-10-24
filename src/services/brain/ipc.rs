//! IPC 跨进程通信模块
//! 大脑 A (hub) 与 大脑 B (superagent) 的 JSON-RPC 级双向通道

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Write};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 指令载体
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum IpcMessage {
    Prompt(String),
    Spawn {
        role: String,
        system_prompt: Option<String>,
        prompt: String,
    },
    Intervene(String),
    ReloadRequest,
    AskUser {
        request_id: String,
        question: String,
        choices: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choice_details: Vec<crate::services::brain::interaction_runtime::InteractionChoice>,
    },
    UserResponse {
        request_id: String,
        response: String,
    },
    Result(String),
    Error(String),
    Event {
        event_type: String,
        content: String,
    },
    Heartbeat,
    CrashReport(String),
}

/// 专属于大脑 A (Hub) 的 IPC 客户端
pub struct IpcClient {
    pub stdout_reader: Option<BufReader<tokio::process::ChildStdout>>,
    pub stdin_writer: Option<tokio::process::ChildStdin>,
}

impl IpcClient {
    pub fn new(child: &mut tokio::process::Child) -> Self {
        Self {
            stdout_reader: child.stdout.take().map(BufReader::new),
            stdin_writer: child.stdin.take(),
        }
    }

    /// 向大脑 B 发送一条指令
    pub async fn send_message(&mut self, msg: IpcMessage) -> Result<(), String> {
        if let Some(writer) = &mut self.stdin_writer {
            let json_str = serde_json::to_string(&msg).unwrap() + "\n";
            writer
                .write_all(json_str.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            writer.flush().await.map_err(|e| e.to_string())?;
            Ok(())
        } else {
            Err("IPC Stdin channel is not available".into())
        }
    }

    /// 接收来自大脑 B 的回调
    pub async fn read_message(&mut self) -> Result<IpcMessage, String> {
        if let Some(reader) = &mut self.stdout_reader {
            let mut line = String::new();
            let bytes_read = reader
                .read_line(&mut line)
                .await
                .map_err(|e| e.to_string())?;

            if bytes_read == 0 {
                return Err("EOF from child process".into());
            }

            match serde_json::from_str(&line) {
                Ok(msg) => Ok(msg),
                Err(e) => Err(format!("{} (raw: {:?})", e.to_string(), line)),
            }
        } else {
            Err("IPC Stdout channel is not available".into())
        }
    }
}

/// 大脑 B 内部启动的独立进程事件循环
pub async fn start_worker_loop(mut engine: crate::domains::agent_cognitive::executor::AgentLoop) {
    let steering_queue = engine.steering_queue.clone();
    let interaction_runtime = engine.interaction_runtime.clone();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<IpcMessage>();

    // 启动后台 Stdin 异步读取 Task
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();
        while let Ok(n) = reader.read_line(&mut line).await {
            if n == 0 {
                break;
            }
            if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                match &msg {
                    IpcMessage::Intervene(instruction) => {
                        let _ = steering_queue.enqueue(
                            super::steering_queue::SteeringSource::Supervisor,
                            instruction,
                            None,
                        );
                    }
                    IpcMessage::CrashReport(report) => {
                        let _ = steering_queue.enqueue(
                            super::steering_queue::SteeringSource::Recovery,
                            &format!("[SYSTEM FAIL-SAFE] Your previous evolution caused a crash or compilation failure. We have rolled back your code to the Last Known Good (LKG) state. CRASH LOG:\n{}", report),
                            None,
                        );
                    }
                    IpcMessage::UserResponse {
                        request_id,
                        response,
                    } => {
                        let _ = interaction_runtime.deliver_response(request_id, response);
                    }
                    _ => {
                        let _ = tx.send(msg);
                    }
                }
            }
            line.clear();
        }
    });

    // 启动心跳 Task
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            send_to_hub(IpcMessage::Heartbeat);
        }
    });

    while let Some(msg) = rx.recv().await {
        match msg {
            IpcMessage::Spawn {
                role,
                system_prompt,
                prompt,
            } => {
                if let Some(sys) = system_prompt {
                    engine.set_system_prompt(&sys);
                }
                engine.set_role(&role);
                match engine.run(&prompt).await {
                    Ok(res) => send_to_hub(IpcMessage::Result(res)),
                    Err(e) => send_to_hub(IpcMessage::Error(e)),
                }
            }
            IpcMessage::Prompt(prompt) => match engine.run(&prompt).await {
                Ok(res) => send_to_hub(IpcMessage::Result(res)),
                Err(e) => send_to_hub(IpcMessage::Error(e)),
            },
            _ => {}
        }
    }
}

/// 大脑 B 向大本营发送消息的统一通道
pub fn send_to_hub(msg: IpcMessage) {
    if let Ok(json_str) = serde_json::to_string(&msg) {
        println!("{}", json_str);
        let _ = io::stdout().flush();
    }
}
