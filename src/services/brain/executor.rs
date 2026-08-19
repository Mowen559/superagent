//! 动态执行大脑 (Executor) - 吸收来自 Harness 核心能力的纯 Rust 引擎
//! 负责运行大模型的 Agent Loop，管理上下文，以及解析和执行 Tool Call
//! 吸收 DeepSeek Harness 防御性模式 (Defensive Patterns):
//! - 在分发器/回调中隔离 Panic 崩溃 (Isolate callback panic in tool execution dispatch loop)

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::services::brain::tools::dynamic_tool::scan_dynamic_tools;
use crate::services::llm_provider::LlmProvider;

/// 工具定义与执行的回调接口
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    
    // 执行工具并返回 JSON 结果
    fn execute(&self, args: Value) -> Result<Value, String>;
}

/// 估算消息列表的总字符数
fn calculate_total_chars(messages: &[Value]) -> usize {
    let mut total = 0;
    for msg in messages {
        if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
            total += content.len();
        }
        if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            for call in tool_calls {
                if let Some(args) = call["function"]["arguments"].as_str() {
                    total += args.len();
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    total += name.len();
                }
            }
        }
    }
    total
}

fn find_safe_compaction_boundary(messages: &[Value], keep_latest: usize) -> Option<usize> {
    if messages.len() <= keep_latest {
        return None;
    }
    let target = messages.len().saturating_sub(keep_latest);
    
    // 必须寻找最近的一个 role == "user" 的消息作为保留起点。
    // 因为对于 Gemini，包含 tool_calls 的 assistant 消息不能紧跟在 system 摘要后面，必须紧跟在 user 后面。
    let mut safe_idx = target;
    while safe_idx > 0 {
        if messages[safe_idx].get("role").and_then(|v| v.as_str()) == Some("user") {
            return Some(safe_idx);
        }
        safe_idx -= 1;
    }

    None
}


#[derive(Debug)]
pub enum AgentEvent {
    UserInput(String),
    SystemEvent(String),
    LlmOutput(Result<serde_json::Value, String>),
}

/// Brain B 的核心运行引擎
pub struct AgentLoop {
    pub plugin_registry: crate::services::brain::tools::registry::PluginRegistry,
    messages: Vec<Value>, // 会话历史
    system_prompt: String,
    llm_provider: LlmProvider,
    pub interventions: Arc<Mutex<Vec<String>>>,
    pub role: String,
    pub event_tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    pub event_rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
}

impl AgentLoop {
    pub fn new(base_prompt: &str) -> Self {
        let client = reqwest::Client::new();
        
        // 自动注入环境上下文 (Context Plugin 逻辑)
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        
        use chrono::Local;
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        
        let mut super_memory_context = String::new();
        if let Ok(mem_data) = std::fs::read_to_string(".super_memory.json") {
            super_memory_context = format!("\n\n[SuperMemory Subconscious Context]\n{}", mem_data);
            eprintln!("\x1b[35m[Subconscious Awakening] Loaded SuperMemory context into System Prompt.\x1b[0m");
        }

        let system_prompt = format!(
            "{}\n\n[Environment Context]\n- OS: {} ({})\n- Current Directory: {}\n- Current Time: {}{}",
            base_prompt, os, arch, cwd, now, super_memory_context
        );

        let mut messages = vec![];
        if let Ok(data) = std::fs::read_to_string(".brain_session.json") {
            if let Ok(loaded) = serde_json::from_str::<Vec<Value>>(&data) {
                eprintln!("[Brain B] Resuming from previous session (Loaded {} messages)", loaded.len());
                messages = loaded;
                let _ = std::fs::remove_file(".brain_session.json"); // 阅后即焚
            }
        }

        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            plugin_registry: crate::services::brain::tools::registry::PluginRegistry::new(),
            messages,
            system_prompt,
            llm_provider: LlmProvider::new(client),
            interventions: Arc::new(Mutex::new(Vec::new())),
            role: "SuperAgent".to_string(),
            event_tx,
            event_rx,
        }
    }

    pub fn get_tools_ref(&self) -> Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>> {
        self.plugin_registry.get_tools_ref()
    }

    /// 自动扫描动态插件/技能目录并挂载


    pub fn set_system_prompt(&mut self, base_prompt: &str) {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        
        use chrono::Local;
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        
        self.system_prompt = format!(
            "{}\n\n[Environment Context]\n- OS: {} ({})\n- Current Directory: {}\n- Current Time: {}",
            base_prompt, os, arch, cwd, now
        );
    }

    pub fn set_role(&mut self, role: &str) {
        self.role = role.to_string();
    }

    /// 持久化当前会话记录
    pub fn save_session(&self) {
        if let Ok(data) = serde_json::to_string(&self.messages) {
            let _ = std::fs::write(".brain_session.json", data);
        }
    }

    /// 挂载原生工具 (吸收自 Harness)
    pub fn register_tool(&mut self, tool: Box<dyn ToolExecutor>) {
        self.plugin_registry.register(tool);
    }

    /// 核心思维循环：不断请求 LLM，如果有工具调用则执行，直到输出最终文本
    pub async fn run(&mut self, prompt: &str) -> Result<String, String> {
        // 1. 追加用户消息
        self.messages.push(json!({ "role": "user", "content": prompt }));
        self.save_session();

        loop {
            // 1.5 检查是否有监工 (Supervisor) 强行注入的最高优先级指令
            let pending_interventions = {
                let mut guard = self.interventions.lock().unwrap();
                let list = guard.clone();
                guard.clear();
                list
            };

            for inst in pending_interventions {
                eprintln!("[Brain B] 收到监工 (Supervisor) 最高优先级介入指令: {}", inst);
                self.messages.push(json!({
                    "role": "user",
                    "content": format!("[SUPERVISOR HIGH-PRIORITY INTERVENTION]: {}", inst)
                }));
                crate::services::brain::ipc::send_to_hub(crate::services::brain::ipc::IpcMessage::Event {
                    event_type: "intervened".to_string(),
                    content: format!("Intervention injected: {}", inst),
                });
            }

            // 2. 检查是否需要历史摘要与截断 (Compaction)
            self.compact_history_if_needed().await?;

            // 3. 异步事件驱动：同时监听 LLM 返回与外部事件
            let llm_future = Self::call_llm_api(&self.plugin_registry, &self.llm_provider, self.messages.clone(), &self.system_prompt);
            
            let llm_message = tokio::select! {
                res = llm_future => {
                    res?
                }
                Some(event) = self.event_rx.recv() => {
                    match event {
                        AgentEvent::UserInput(input) => {
                            eprintln!("[Brain B] 收到异步用户输入: {}", input);
                            self.messages.push(json!({ "role": "user", "content": input }));
                            continue;
                        }
                        AgentEvent::SystemEvent(sys_evt) => {
                            eprintln!("[Brain B] 收到系统事件: {}", sys_evt);
                            self.messages.push(json!({ "role": "system", "content": sys_evt }));
                            continue;
                        }
                        AgentEvent::LlmOutput(res) => {
                            match res {
                                Ok(val) => val,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }
            };

            // 4. 将 LLM 的回复加入历史
            self.messages.push(llm_message.clone());
            self.save_session();

            // 5. 检查是否触发了工具调用 (Tool Call)
            if let Some(tool_calls) = llm_message.get("tool_calls").and_then(|v| v.as_array()) {
                for call in tool_calls {
                    let func_name = call["function"]["name"].as_str().unwrap_or("");
                    let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    
                    crate::services::brain::ipc::send_to_hub(crate::services::brain::ipc::IpcMessage::Event {
                        event_type: "tool_call".to_string(),
                        content: format!("{}({})", func_name, args_str),
                    });

                    let tool_result = {
                        let tools_ref = self.plugin_registry.get_tools_ref();
                        let tools_guard = tools_ref.lock().unwrap();
                        if let Some(tool) = tools_guard.get(func_name) {
                            eprintln!("[Brain B] 执行工具: {} with args: {}", func_name, args_str);
                            
                            // 防御性模式：在分发器中隔离回调工具执行中的 panic 异常
                            let panic_guard = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                tool.execute(args)
                            }));

                            match panic_guard {
                                Ok(Ok(res)) => res,
                                Ok(Err(e)) => json!({ "error": e }),
                                Err(panic_err) => {
                                    let err_msg = if let Some(s) = panic_err.downcast_ref::<&str>() {
                                        s.to_string()
                                    } else if let Some(s) = panic_err.downcast_ref::<String>() {
                                        s.clone()
                                    } else {
                                        "Unknown panic occurred".to_string()
                                    };
                                    eprintln!("[Brain B Defensive Error] Tool '{}' panicked! Caught: {}", func_name, err_msg);
                                    json!({
                                        "error": format!("Tool '{}' panicked during execution: {}", func_name, err_msg),
                                        "panicked": true
                                    })
                                }
                            }
                        } else {
                            json!({ "error": format!("Tool {} not found", func_name) })
                        }
                    };

                    let truncated_res = if tool_result.to_string().len() > 1000 {
                        format!("{}... [truncated]", &tool_result.to_string()[..1000])
                    } else {
                        tool_result.to_string()
                    };
                    crate::services::brain::ipc::send_to_hub(crate::services::brain::ipc::IpcMessage::Event {
                        event_type: "tool_result".to_string(),
                        content: format!("{}: {}", func_name, truncated_res),
                    });

                    // 6. 将工具执行结果喂给 LLM
                    self.messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call["id"],
                        "content": tool_result.to_string(),
                    }));
                }
                self.save_session();
                // 继续下一次循环，让 LLM 根据工具结果继续思考
                continue;
            }

            // 7. 如果没有工具调用，说明大模型给出了最终的普通文本回复
            if let Some(content) = llm_message.get("content").and_then(|v| v.as_str()) {
                crate::services::brain::ipc::send_to_hub(crate::services::brain::ipc::IpcMessage::Event {
                    event_type: "thought".to_string(),
                    content: content.to_string(),
                });
                return Ok(content.to_string());
            }

            return Err("Unexpected empty response from LLM".to_string());
        }
    }

    /// 真实向 LLM API 调用
    async fn call_llm_api(
        plugin_registry: &crate::services::brain::tools::registry::PluginRegistry,
        llm_provider: &LlmProvider,
        messages: Vec<Value>,
        system_prompt: &str,
    ) -> Result<Value, String> {
        let tools_schema: Vec<Value> = {
            let tools_guard = plugin_registry.get_tools_ref();
            let tools_guard = tools_guard.lock().unwrap();
            tools_guard.values().map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters_schema(),
                    }
                })
            }).collect()
        };
        
        let tools_param = if tools_schema.is_empty() { None } else { Some(json!(tools_schema)) };
        
        match llm_provider.chat_with_tools(messages, Some(system_prompt), tools_param).await {
            Ok((msg, usage)) => {
                eprintln!("\n\x1b[45;37m[Token 消耗账单] Prompt: {}, Completion: {}, Cost: ${:.4}, Latency: {}ms\x1b[0m\n", 
                    usage.prompt_tokens, usage.completion_tokens, usage.cost, usage.duration_ms);
                Ok(msg)
            },
            Err(e) => Err(e),
        }
    }

    /// 辅助方法：智能对话摘要与滚动挤压 (Rolling LLM Compaction)
    async fn compact_history_if_needed(&mut self) -> Result<(), String> {
        const MAX_MESSAGES_THRESHOLD: usize = 12;
        const MAX_CHARS_THRESHOLD: usize = 20_000; // 约 5000 Tokens 估算
        const KEEP_LATEST: usize = 5;

        let total_chars = calculate_total_chars(&self.messages);
        let msg_count = self.messages.len();

        // 仅在超出消息数或字符数上限时触发
        if msg_count <= MAX_MESSAGES_THRESHOLD && total_chars <= MAX_CHARS_THRESHOLD {
            return Ok(());
        }

        eprintln!(
            "\n\x1b[33m[Brain B] Triggering LLM Compaction (Messages: {}, Chars: {})...\x1b[0m",
            msg_count, total_chars
        );

        let to_summarize_count = match find_safe_compaction_boundary(&self.messages, KEEP_LATEST) {
            Some(idx) => idx,
            None => {
                eprintln!("\x1b[33m[Brain B] Cannot find safe compaction boundary. Skipping compaction.\x1b[0m");
                return Ok(());
            }
        };

        // 分离先前的滚动总结 (Prior Summary) 和新需要总结的历史 (New History)
        let mut prior_summary = String::new();
        let mut history_text = String::new();

        for (i, msg) in self.messages.iter().take(to_summarize_count).enumerate() {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("");

            // 检查是否有之前压缩留下来的 Context Summary
            if role == "system" && content.contains("[Context Summary") {
                prior_summary.push_str(content);
                prior_summary.push('\n');
            } else {
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                    let calls_summary: Vec<String> = tool_calls.iter().map(|c| {
                        let f_name = c["function"]["name"].as_str().unwrap_or("fn");
                        let args = c["function"]["arguments"].as_str().unwrap_or("{}");
                        format!("tool_call:{}({})", f_name, args)
                    }).collect();
                    history_text.push_str(&format!("[{}] {}: {}\n", i, role, calls_summary.join("; ")));
                }
                if !content.is_empty() {
                    // 对极其冗长的 tool 回复进行预切割，避免溢出总结 Prompt
                    let truncated_content = if content.len() > 4000 {
                        let mut idx = 4000;
                        while !content.is_char_boundary(idx) && idx > 0 {
                            idx -= 1;
                        }
                        format!("{}... [truncated {} bytes]", &content[..idx], content.len() - idx)
                    } else {
                        content.to_string()
                    };
                    history_text.push_str(&format!("[{}] {}({}): {}\n", i, role, name, truncated_content));
                }
            }
        }

        // 构建结构化与滚动总结提示词
        let summary_prompt = if !prior_summary.is_empty() {
            format!(
                "You are an expert agent context compactor. Update the existing summary with new conversation events into a cohesive Rolling Context Summary.\n\n\
                --- PRIOR ROLLING SUMMARY ---\n{}\n\n\
                --- NEW HISTORY TO MERGE ---\n{}\n\n\
                Instructions:\n\
                1. Preserve core user goals, key findings, modified files, completed steps, and critical tool execution results.\n\
                2. Omit raw logs, repeated errors, redundant data, and outdated temporary context.\n\
                3. Structure the result into:\n\
                   - **Goal & Requirements**\n\
                   - **Actions Taken & Tool Execution Outcomes** (e.g. files changed, build/test results)\n\
                   - **Key Decisions & Discoveries**\n\
                   - **Active Context / Pending Tasks**",
                prior_summary.trim(), history_text.trim()
            )
        } else {
            format!(
                "You are an expert agent context compactor. Summarize the conversation history into a structured Context Summary.\n\n\
                --- CONVERSATION HISTORY ---\n{}\n\n\
                Instructions:\n\
                1. Preserve core user goals, key findings, modified files, completed steps, and critical tool execution results.\n\
                2. Omit raw logs, repeated errors, redundant data, and outdated temporary context.\n\
                3. Structure the result into:\n\
                   - **Goal & Requirements**\n\
                   - **Actions Taken & Tool Execution Outcomes** (e.g. files changed, build/test results)\n\
                   - **Key Decisions & Discoveries**\n\
                   - **Active Context / Pending Tasks**",
                history_text.trim()
            )
        };

        let temp_messages = vec![
            json!({ "role": "system", "content": "You are a precise technical memory compactor. Output concise, structured bullet points retaining all crucial tool execution states and file paths." }),
            json!({ "role": "user", "content": summary_prompt }),
        ];

        // 临时调用一次 LLM 生成 Rolling Summary
        match self.llm_provider.chat_with_tools(temp_messages, None, None).await {
            Ok((res, _)) => {
                let summary = res.get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Failed to generate summary.")
                    .to_string();

                eprintln!("\x1b[32m[Brain B] Compaction successful. Replacing {} old messages.\x1b[0m", to_summarize_count);

                let mut new_messages = vec![
                    json!({ "role": "system", "content": format!("[Context Summary & Rolling State]\n{}", summary) })
                ];

                new_messages.extend(self.messages.drain(to_summarize_count..));
                self.messages = new_messages;
                self.save_session();

                Ok(())
            }
            Err(e) => {
                eprintln!("\x1b[31m[Brain B] Compaction failed: {}. Falling back to hard truncation.\x1b[0m", e);
                let mut new_messages = vec![
                    json!({ "role": "system", "content": "[Context Summary] Hard truncation occurred due to summarization failure." })
                ];
                new_messages.extend(self.messages.drain(to_summarize_count..));
                self.messages = new_messages;
                self.save_session();
                Ok(())
            }
        }
    }
}

impl Drop for AgentLoop {
    fn drop(&mut self) {
        eprintln!("\x1b[31m[Brain B] AgentLoop is shutting down. Cleaning up orphan processes...\x1b[0m");
        crate::services::brain::tools::process_manager::cleanup_all_processes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_total_chars() {
        let msgs = vec![
            json!({ "role": "user", "content": "hello" }),
            json!({ "role": "assistant", "content": "world" }),
        ];
        assert_eq!(calculate_total_chars(&msgs), 10);
    }

    #[test]
    fn test_find_safe_compaction_boundary() {
        let msgs = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "u1" }),
            json!({ "role": "assistant", "content": "a1" }),
            json!({ "role": "user", "content": "u2" }),
            json!({ "role": "assistant", "content": "a2" }),
        ];
        let idx = find_safe_compaction_boundary(&msgs, 2);
        assert_eq!(idx, Some(3));
    }
}
