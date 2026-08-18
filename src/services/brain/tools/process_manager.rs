//! 进程管理器 (Process Manager)
//! 用于托管后台长时间运行的 Terminal 进程，收集日志，并在需要时终止它们。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::process::Child;
use std::io::Read;
use lazy_static::lazy_static;

pub struct ProcessState {
    pub command: String,
    pub child: Child,
    pub logs: Arc<Mutex<Vec<String>>>,
}

lazy_static! {
    pub static ref PROCESS_REGISTRY: Arc<RwLock<HashMap<String, ProcessState>>> = 
        Arc::new(RwLock::new(HashMap::new()));
}

/// 注册一个新后台进程，自动拉起监听线程把 stdout/stderr 灌入 logs
pub fn register_process(id: String, command: String, mut child: Child) {
    let logs = Arc::new(Mutex::new(Vec::new()));
    
    // 提取 stdout 和 stderr
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    
    let logs_clone = logs.clone();
    
    // 启动后台读取线程 (Stdout)
    std::thread::spawn(move || {
        if let Some(ref mut out) = stdout {
            let mut buf = [0; 1024];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 { break; }
                if let Ok(s) = String::from_utf8(buf[..n].to_vec()) {
                    if let Ok(mut l) = logs_clone.lock() {
                        l.push(s);
                        if l.len() > 1000 {
                            l.remove(0);
                        }
                    }
                }
            }
        }
    });

    let logs_clone2 = logs.clone();
    // 启动后台读取线程 (Stderr)
    std::thread::spawn(move || {
        if let Some(ref mut err) = stderr {
            let mut buf = [0; 1024];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 { break; }
                if let Ok(s) = String::from_utf8(buf[..n].to_vec()) {
                    if let Ok(mut l) = logs_clone2.lock() {
                        l.push(format!("[STDERR] {}", s));
                        if l.len() > 1000 {
                            l.remove(0);
                        }
                    }
                }
            }
        }
    });

    let state = ProcessState { command, child, logs };
    if let Ok(mut registry) = PROCESS_REGISTRY.write() {
        registry.insert(id, state);
    }
}

/// 获取某个进程的当前日志
pub fn get_process_logs(id: &str) -> Option<String> {
    if let Ok(registry) = PROCESS_REGISTRY.read() {
        if let Some(state) = registry.get(id) {
            if let Ok(l) = state.logs.lock() {
                return Some(l.join(""));
            }
        }
    }
    None
}

/// 杀死指定进程
pub fn kill_process(id: &str) -> bool {
    if let Ok(mut registry) = PROCESS_REGISTRY.write() {
        if let Some(mut state) = registry.remove(id) {
            let _ = state.child.kill();
            return true;
        }
    }
    false
}

/// 列出所有后台进程
pub fn list_processes() -> Vec<serde_json::Value> {
    let mut list = Vec::new();
    if let Ok(registry) = PROCESS_REGISTRY.read() {
        for (id, state) in registry.iter() {
            list.push(serde_json::json!({
                "process_id": id,
                "command": state.command,
            }));
        }
    }
    list
}

/// 清理所有孤儿进程
pub fn cleanup_all_processes() {
    if let Ok(mut registry) = PROCESS_REGISTRY.write() {
        for (id, state) in registry.iter_mut() {
            eprintln!("[Brain B] Cleaning up orphan process: {} ({})", id, state.command);
            let _ = state.child.kill();
        }
        registry.clear();
    }
}
