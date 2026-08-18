//! 动态执行大脑 (Executor) - 吸收了 Harness 核心能力的纯 Rust 引擎
//! 负责运行大模型的 Agent Loop，管理上下文，以及解析和执行 Tool Call

use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::services::llm_provider::LlmProvider;

/// 工具定义与执行的回调接口
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    
    // 执行工具并返回 JSON 结果
    fn execute(&self, args: Value) -> Result<Value, String>;
}

/// Brain B 的核心运行引擎
pub struct AgentLoop {
    tools: HashMap<String, Box<dyn ToolExecutor>>,
    messages: Vec<Value>, // 会话历史
    system_prompt: String,
    llm_provider: LlmProvider,
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
        
        let system_prompt = format!(
            "{}\n\n[Environment Context]\n- OS: {} ({})\n- Current Directory: {}\n- Current Time: {}",
            base_prompt, os, arch, cwd, now
        );

        let mut messages = vec![];
        if let Ok(data) = std::fs::read_to_string(".brain_session.json") {
            if let Ok(loaded) = serde_json::from_str::<Vec<Value>>(&data) {
                eprintln!("[Brain B] Resuming from previous session (Loaded {} messages)", loaded.len());
                messages = loaded;
                let _ = std::fs::remove_file(".brain_session.json"); // 阅后即焚
            }
        }

        Self {
            tools: HashMap::new(),
            messages,
            system_prompt,
            llm_provider: LlmProvider::new(client),
        }
    }

    /// 持久化当前会话记忆
    pub fn save_session(&self) {
        if let Ok(data) = serde_json::to_string(&self.messages) {
            let _ = std::fs::write(".brain_session.json", data);
        }
    }

    /// 挂载原生工具 (吸收自 Harness)
    pub fn register_tool(&mut self, tool: Box<dyn ToolExecutor>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// 核心思维环：不断请求 LLM，如果有工具调用则执行，直到输出最终文本
    pub async fn run(&mut self, prompt: &str) -> Result<String, String> {
        // 1. 追加用户消息
        self.messages.push(json!({ "role": "user", "content": prompt }));
        self.save_session();

        loop {
            // 2. 检查是否需要历史摘要与截断 (Compaction)
            self.compact_history_if_needed().await?;

            // 3. 向 LLM API 发起请求
            let llm_message = self.call_llm_api().await?;

            // 4. 将 LLM 的回复加入历史
            self.messages.push(llm_message.clone());
            self.save_session();

            // 5. 检查是否触发了工具调用 (Tool Call)
            if let Some(tool_calls) = llm_message.get("tool_calls").and_then(|v| v.as_array()) {
                for call in tool_calls {
                    let func_name = call["function"]["name"].as_str().unwrap_or("");
                    let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    
                    let tool_result = if let Some(tool) = self.tools.get(func_name) {
                        eprintln!("[Brain B] 执行工具: {} with args: {}", func_name, args_str);
                        match tool.execute(args) {
                            Ok(res) => res,
                            Err(e) => json!({ "error": e }),
                        }
                    } else {
                        json!({ "error": format!("Tool {} not found", func_name) })
                    };

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
                return Ok(content.to_string());
            }

            return Err("Unexpected empty response from LLM".to_string());
        }
    }

    /// 真实的 LLM API 调用
    async fn call_llm_api(&self) -> Result<Value, String> {
        let tools_schema: Vec<Value> = self.tools.values().map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters_schema(),
                }
            })
        }).collect();
        
        let tools_param = if tools_schema.is_empty() { None } else { Some(json!(tools_schema)) };
        
        match self.llm_provider.chat_with_tools(self.messages.clone(), Some(&self.system_prompt), tools_param).await {
            Ok((msg, _usage)) => Ok(msg),
            Err(e) => Err(e),
        }
    }

    /// 辅助方法：高级对话摘要与挤压 (LLM Compaction)
    async fn compact_history_if_needed(&mut self) -> Result<(), String> {
        // 如果消息条数超过阈值（例如 15 条），触发挤压
        if self.messages.len() <= 15 {
            return Ok(());
        }

        eprintln!("\n\x1b[33m[Brain B] Triggering LLM Compaction (History length: {})...\x1b[0m", self.messages.len());

        // 提取需要被总结的旧记录（保留最后 5 条作为最近的上下文）
        let keep_latest = 5;
        let to_summarize_count = self.messages.len() - keep_latest;
        
        // 构造给 LLM 的总结 Prompt
        let mut history_text = String::new();
        for (i, msg) in self.messages.iter().take(to_summarize_count).enumerate() {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("");
            history_text.push_str(&format!("[{}] {}({}): {}\n", i, role, name, content));
        }

        let summary_prompt = format!(
            "Please summarize the following conversation history briefly. Focus on core conclusions, completed goals, and critical paths. Discard verbose errors and irrelevant details.\n\nHistory:\n{}",
            history_text
        );

        let temp_messages = vec![
            json!({ "role": "system", "content": "You are a helpful summarizer." }),
            json!({ "role": "user", "content": summary_prompt }),
        ];

        // 临时调用一次 LLM
        match self.llm_provider.chat_with_tools(temp_messages, None, None).await {
            Ok((res, _)) => {
                let summary = res.get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Failed to generate summary.")
                    .to_string();
                
                eprintln!("\x1b[32m[Brain B] Compaction successful. Replacing {} old messages.\x1b[0m", to_summarize_count);
                
                // 替换消息
                let mut new_messages = vec![
                    json!({ "role": "system", "content": format!("[System Compaction] Summary of past context:\n{}", summary) })
                ];
                
                // 将最新保留的追加回去
                new_messages.extend(self.messages.drain(to_summarize_count..));
                self.messages = new_messages;
                self.save_session();
                
                Ok(())
            }
            Err(e) => {
                eprintln!("\x1b[31m[Brain B] Compaction failed: {}. Falling back to hard truncation.\x1b[0m", e);
                // 降级：硬截断（丢弃最旧的 N 条）
                let mut new_messages = vec![
                    json!({ "role": "system", "content": "[System Compaction] Hard truncation occurred due to summarization failure." })
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
