use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use crate::services::brain::context::Context;
use crate::services::brain::events::SessionEvent;
use crate::services::llm_provider::LlmProvider;
use crate::services::brain::dynamic_tool::scan_dynamic_tools;

/// 工具定义与执行的回调接口
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    
    // 执行工具并返回 JSON 结果
    fn execute(&self, args: Value) -> Result<Value, String>;

    // 可选：该工具挂载时动态投影到系统潜意识的提示词
    fn prompt_projection(&self) -> Option<String> {
        None
    }
}

/// DSH 范式的运行引擎：一切基于 Context 容器与 SessionEvent 事件流
pub struct AgentLoop {
    pub ctx: Context,
    tools: Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>>,
    pub events: Vec<SessionEvent>, // 会话历史（强类型事件溯源）
    system_prompt: String,
    llm_provider: LlmProvider,
    pub interventions: Arc<Mutex<Vec<String>>>,
    pub role: String,
}

impl AgentLoop {
    pub fn new(base_prompt: &str) -> Self {
        let client = reqwest::Client::new();
        
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

        let mut events = vec![];
        if let Ok(data) = std::fs::read_to_string(".brain_events.json") {
            if let Ok(loaded) = serde_json::from_str::<Vec<SessionEvent>>(&data) {
                eprintln!("[Brain B] Resuming from event sourcing log (Loaded {} events)", loaded.len());
                events = loaded;
            }
        }

        Self {
            ctx: Context::new(),
            tools: Arc::new(Mutex::new(HashMap::new())),
            events,
            system_prompt,
            llm_provider: LlmProvider::new(client),
            interventions: Arc::new(Mutex::new(Vec::new())),
            role: "SuperAgent".to_string(),
        }
    }

    pub fn save_session(&self) {
        if let Ok(data) = serde_json::to_string(&self.events) {
            let _ = std::fs::write(".brain_events.json", data);
        }
    }

    pub fn register_tool(&mut self, tool: Box<dyn ToolExecutor>) {
        let mut lock = self.tools.lock().unwrap();
        lock.insert(tool.name().to_string(), tool);
    }

    pub fn get_tools_ref(&self) -> Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>> {
        self.tools.clone()
    }

    /// 根据 Event Sourcing 动态构建当前上下文给大模型 (Projection)
    fn project_llm_messages(&self) -> Vec<Value> {
        let mut msgs = Vec::new();
        for event in &self.events {
            msgs.push(event.to_llm_message());
        }
        msgs
    }

    /// 核心思维循环：完全由事件驱动
    pub async fn run(&mut self, prompt: &str) -> Result<String, String> {
        // 1. 发射用户输入事件
        self.events.push(SessionEvent::UserInput(prompt.to_string()));
        self.save_session();
        
        let mut consecutive_same_tool_calls = 0;
        let mut last_tool_call_signature = String::new();

        loop {
            // 1.5 监工中断注入
            let pending_interventions = {
                let mut guard = self.interventions.lock().unwrap();
                let list = guard.clone();
                guard.clear();
                list
            };
            for inst in pending_interventions {
                eprintln!("[Brain B] 收到监工最高优先级介入指令: {}", inst);
                self.events.push(SessionEvent::SupervisorIntervention(inst.clone()));
            }

            // 2. 将事件源投影为 LLM 可读消息
            let projected_messages = self.project_llm_messages();

            // 3. 向 LLM 发起请求
            let llm_message = self.call_llm_api(projected_messages).await?;

            // 4. 解析 LLM 回复，发射模型思考事件 / 工具调用事件
            if let Some(tool_calls) = llm_message.get("tool_calls").and_then(|v| v.as_array()) {
                let mut guard_triggered = false;
                for call in tool_calls {
                    let func_name = call["function"]["name"].as_str().unwrap_or("").to_string();
                    let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    
                    let call_id = call["id"].as_str().unwrap_or("").to_string();

                    let current_sig = format!("{}({})", func_name, args_str);
                    if current_sig == last_tool_call_signature {
                        consecutive_same_tool_calls += 1;
                    } else {
                        last_tool_call_signature = current_sig;
                        consecutive_same_tool_calls = 0;
                    }

                    // 发射工具调用事件
                    self.events.push(SessionEvent::ToolCall {
                        id: call_id.clone(),
                        name: func_name.clone(),
                        arguments: args.clone(),
                    });

                    if consecutive_same_tool_calls >= 3 {
                        eprintln!("\x1b[31m[System Guard] Dead loop detected. Forcibly intervening.\x1b[0m");
                        self.events.push(SessionEvent::SystemWarning(
                            "You have repeated the exact same tool call 3 times and are stuck in a failure loop. STOP this action immediately. Analyze the errors and try a completely different approach, or use AskUserTool.".to_string()
                        ));
                        consecutive_same_tool_calls = 0;
                        guard_triggered = true;
                        break;
                    }

                    // 执行工具 (此时工具可能会利用 Context 里面的 Capability)
                    let tool_result = {
                        let tools_guard = self.tools.lock().unwrap();
                        if let Some(tool) = tools_guard.get(&func_name) {
                            eprintln!("[Brain B] 执行工具: {} with args: {}", func_name, args_str);
                            let panic_guard = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                tool.execute(args)
                            }));

                            match panic_guard {
                                Ok(Ok(res)) => res,
                                Ok(Err(e)) => json!({ "error": e }),
                                Err(panic_err) => {
                                    json!({ "error": "Tool panicked", "panicked": true })
                                }
                            }
                        } else {
                            json!({ "error": format!("Tool {} not found", func_name) })
                        }
                    };

                    // 发射工具执行结果事件
                    self.events.push(SessionEvent::ToolResult {
                        id: call_id,
                        result: tool_result,
                    });
                }
                self.save_session();
                
                if guard_triggered { continue; }
                continue;
            }

            // 7. 最终文本回复
            if let Some(content) = llm_message.get("content").and_then(|v| v.as_str()) {
                self.events.push(SessionEvent::ModelThought(content.to_string()));
                self.save_session();
                return Ok(content.to_string());
            }

            return Err("Unexpected empty response from LLM".to_string());
        }
    }

    async fn call_llm_api(&self, messages: Vec<Value>) -> Result<Value, String> {
        let mut dynamic_prompt = self.system_prompt.clone();
        
        let tools_schema: Vec<Value> = {
            let tools_guard = self.tools.lock().unwrap();
            let mut projections = Vec::new();
            
            let schema = tools_guard.values().map(|t| {
                if let Some(proj) = t.prompt_projection() {
                    projections.push(proj);
                }
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters_schema(),
                    }
                })
            }).collect();
            
            if !projections.is_empty() {
                dynamic_prompt.push_str("\n\n# Dynamic Tool Projections (Active Skills)\n");
                dynamic_prompt.push_str(&projections.join("\n\n"));
            }
            
            schema
        };
        
        let tools_param = if tools_schema.is_empty() { None } else { Some(json!(tools_schema)) };
        
        match self.llm_provider.chat_with_tools(messages, Some(&dynamic_prompt), tools_param).await {
            Ok((msg, usage)) => {
                eprintln!("\n\x1b[45;37m[Token 消耗账单] Prompt: {}, Completion: {}, Cost: ${:.4}, Latency: {}ms\x1b[0m\n", 
                    usage.prompt_tokens, usage.completion_tokens, usage.cost, usage.duration_ms);
                Ok(msg)
            },
            Err(e) => Err(e),
        }
    }
}
