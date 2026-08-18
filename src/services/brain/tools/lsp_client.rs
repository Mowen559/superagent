//! 极简 LSP 客户端 (Lightweight LSP Client)
//! 用于与 rust-analyzer / tsserver 等基于 stdin/stdout 的 Language Server 通信

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use lazy_static::lazy_static;

lazy_static! {
    /// 全局 LSP 客户端单例，防止大模型每次调用都重启 Server
    pub static ref GLOBAL_LSP_CLIENT: Arc<Mutex<Option<LspClient>>> = Arc::new(Mutex::new(None));
}

pub struct LspClient {
    stdin: ChildStdin,
    request_counter: u64,
    responses: Arc<Mutex<HashMap<u64, Value>>>,
    // 保持 Child 的所有权防止进程退出
    _child: Child,
}

impl LspClient {
    /// 启动 LSP 服务器
    pub fn start(command: &str, args: &[&str], root_uri: &str) -> Result<Self, String> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // 暂时忽略 stderr
            .spawn()
            .map_err(|e| format!("Failed to spawn LSP server: {}", e))?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        
        let responses = Arc::new(Mutex::new(HashMap::new()));
        let responses_clone = responses.clone();

        // 启动后台监听线程读取 stdout
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                // 读取 Header (Content-Length: xxx)
                let mut content_length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return; // EOF
                    }
                    let line = line.trim();
                    if line.is_empty() {
                        break; // 头部结束
                    }
                    if line.to_lowercase().starts_with("content-length:") {
                        let parts: Vec<&str> = line.split(':').collect();
                        if parts.len() == 2 {
                            content_length = parts[1].trim().parse().unwrap_or(0);
                        }
                    }
                }

                if content_length > 0 {
                    let mut body = vec![0; content_length];
                    if reader.read_exact(&mut body).is_ok() {
                        if let Ok(json_val) = serde_json::from_slice::<Value>(&body) {
                            // 检查是否有 id，如果是 response 则存入 HashMap
                            if let Some(id_val) = json_val.get("id") {
                                if let Some(id) = id_val.as_u64() {
                                    if let Ok(mut map) = responses_clone.lock() {
                                        map.insert(id, json_val);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let mut client = Self {
            stdin,
            request_counter: 1,
            responses,
            _child: child,
        };

        // 发送 initialize
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "hover": { "dynamicRegistration": true },
                    "definition": { "dynamicRegistration": true },
                    "references": { "dynamicRegistration": true }
                }
            }
        });
        
        // 阻塞等待 initialize 响应
        let _ = client.send_request_and_wait("initialize", init_params)?;
        
        // 发送 initialized 通知 (无响应)
        client.send_notification("initialized", json!({}))?;

        Ok(client)
    }

    /// 发送一个不需要响应的通知
    pub fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.write_payload(payload)
    }

    /// 发送请求并阻塞等待响应 (超时 5 秒)
    pub fn send_request_and_wait(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.request_counter;
        self.request_counter += 1;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.write_payload(payload)?;

        // 轮询等待响应
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(100));
            if let Ok(mut map) = self.responses.lock() {
                if let Some(resp) = map.remove(&id) {
                    return Ok(resp);
                }
            }
        }

        Err("LSP request timed out".to_string())
    }

    fn write_payload(&mut self, payload: Value) -> Result<(), String> {
        let body = serde_json::to_string(&payload).unwrap();
        let msg = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        
        self.stdin.write_all(msg.as_bytes()).map_err(|e| format!("Write error: {}", e))?;
        self.stdin.flush().map_err(|e| format!("Flush error: {}", e))?;
        
        Ok(())
    }
}
