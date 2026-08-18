//! IPC 跨进程通信模块
//! 大脑 A (hub) 与 大脑 B (superagent) 的 JSON-RPC 级双向通道

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use std::process::Child;

/// 指令载体
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IpcMessage {
    Prompt(String),
    ReloadRequest,
    AskUser { question: String },
    UserResponse(String),
    Result(String),
    Error(String),
}

/// 专属于大脑 A (Hub) 的 IPC 客户端
pub struct IpcClient {
    pub stdout_reader: Option<tokio::process::ChildStdout>,
    pub stdin_writer: Option<tokio::process::ChildStdin>,
}

impl IpcClient {
    pub fn new(child: &mut tokio::process::Child) -> Self {
        Self {
            stdout_reader: child.stdout.take(),
            stdin_writer: child.stdin.take(),
        }
    }

    /// 向大脑 B 发送一条指令
    pub async fn send_message(&mut self, msg: IpcMessage) -> Result<(), String> {
        if let Some(writer) = &mut self.stdin_writer {
            let json_str = serde_json::to_string(&msg).unwrap() + "\n";
            writer.write_all(json_str.as_bytes()).await.map_err(|e| e.to_string())?;
            writer.flush().await.map_err(|e| e.to_string())?;
            Ok(())
        } else {
            Err("IPC Stdin channel is not available".into())
        }
    }

    /// 接收来自大脑 B 的回调
    pub async fn read_message(&mut self) -> Result<IpcMessage, String> {
        if let Some(reader) = &mut self.stdout_reader {
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();
            let bytes_read = buf_reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
            
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
pub async fn start_worker_loop(mut engine: super::executor::AgentLoop) {
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut buffer = String::new();

    loop {
        buffer.clear();
        match handle.read_line(&mut buffer) {
            Ok(0) => break, // EOF, pipe closed
            Ok(_) => {
                if let Ok(msg) = serde_json::from_str::<IpcMessage>(&buffer) {
                    match msg {
                        IpcMessage::Prompt(prompt) => {
                            // 执行引擎处理
                            match engine.run(&prompt).await {
                                Ok(res) => send_to_hub(IpcMessage::Result(res)),
                                Err(e) => send_to_hub(IpcMessage::Error(e)),
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => break,
        }
    }
}

/// 大脑 B 向大本营发送消息的统一通道
pub fn send_to_hub(msg: IpcMessage) {
    let json_str = serde_json::to_string(&msg).unwrap();
    println!("{}", json_str);
    io::stdout().flush().unwrap();
}
