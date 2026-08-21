use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use crate::services::brain::context::Context;
use crate::services::brain::events::{SessionEvent, EventBus, BeforePromptHook, BoxFuture};
use crate::services::llm_provider::LlmProvider;
use crate::services::brain::dynamic_tool::scan_dynamic_tools;

/// 工具定义与执行的回调接口
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    
    // 执行工具并返回 JSON 结果
    fn execute(&self, args: Value) -> Result<Value, String>;

    fn prompt_projection(&self) -> Option<String> {
        None
    }
}

/// RTK Hook for Circuit Breaker
struct CircuitBreakerHook {
    threshold: usize,
}

impl BeforePromptHook for CircuitBreakerHook {
    fn on_before_prompt<'a>(&'a self, events: &'a mut Vec<SessionEvent>, llm: &'a LlmProvider) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let total_chars: usize = events.iter()
                .map(|e| serde_json::to_string(&e.to_llm_message()).unwrap_or_default().len())
                .sum();
            
            if total_chars > self.threshold && events.len() > 10 {
                eprintln!("\x1b[31m[Circuit Breaker] Context limit reached ({} chars > {}), compacting memory...\x1b[0m", total_chars, self.threshold);
                
                let preserve_count = 3;
                let compact_count = events.len() - preserve_count;
                
                let events_to_compact: Vec<_> = events.drain(0..compact_count).collect();
                
                let recent_events: Vec<serde_json::Value> = events_to_compact.iter().map(|e| e.to_llm_message()).collect();
                
                let prompt = "You are an AI memory compaction agent. Summarize the following past events into a highly dense and factual summary. Retain all key facts, constraints, decisions, and tool outputs.";
                
                let messages = vec![
                    serde_json::json!({ "role": "system", "content": prompt }),
                    serde_json::json!({ "role": "user", "content": format!("{:?}", recent_events) })
                ];
                
                match llm.chat_with_tools(messages, None, None).await {
                    Ok((msg, _)) => {
                        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                            let compacted_event = SessionEvent::ContextCompacted {
                                summary: content.to_string(),
                                original_event_count: compact_count,
                            };
                            events.insert(0, compacted_event);
                            eprintln!("\x1b[32m[Circuit Breaker] Compaction successful. Context significantly reduced.\x1b[0m");
                        }
                    },
                    Err(e) => {
                        eprintln!("\x1b[31m[Circuit Breaker] Compaction failed: {}. Restoring events.\x1b[0m", e);
                        for (i, e) in events_to_compact.into_iter().enumerate() {
                            events.insert(i, e);
                        }
                    }
                }
            }
        })
    }
}

/// DSH 范式的运行引擎：一切基于 Context 容器与 SessionEvent 事件流
pub struct AgentLoop {
    pub ctx: Context,
    tools: Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>>,
    pub events: Vec<SessionEvent>, // 会话历史（强类型事件溯源）
    pub event_bus: EventBus,
    system_prompt: String,
    llm_provider: LlmProvider,
    pub interventions: Arc<Mutex<Vec<String>>>,
    pub role: String,
    pub runtime_config: crate::config::runtime::RuntimeConfig,
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
        
        let app_paths = crate::config::app_paths::AppPaths::new();
        let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
        
        let mut super_memory_context = String::new();
        {
            let store = crate::services::supermemory_store::SupermemoryStore::new(&config, None);
            let records = store.search("", 5);
            let mut content_summary = String::new();
            for r in records.into_iter() {
                content_summary.push_str(&format!("- {}: {}\n", r.name, r.summary));
            }
            if !content_summary.is_empty() {
                super_memory_context = format!("\n\n[SuperMemory Subconscious Context]\n{}", content_summary);
                eprintln!("\x1b[35m[Subconscious Awakening] Loaded SuperMemory context into System Prompt.\x1b[0m");
            }
        }

        let mut graph_context = String::new();
        {
            if let Ok(graph) = crate::services::graph_service::get_project_graph(&cwd, 20, None) {
                if !graph.nodes.is_empty() {
                    let mut node_summary = String::new();
                    for n in graph.nodes.into_iter().take(20) {
                        node_summary.push_str(&format!("- {} ({}): {}\n", n.id, n.label, n.file_path.unwrap_or_default()));
                    }
                    graph_context = format!("\n\n[Codebase Graph Subconscious]\nTop Architecture Nodes:\n{}", node_summary);
                    eprintln!("\x1b[35m[Subconscious Awakening] Loaded Knowledge Graph architecture into System Prompt.\x1b[0m");
                }
            }
        }

        let system_prompt = format!(
            "{}\n\n[Environment Context]\n- OS: {} ({})\n- Current Directory: {}\n- Current Time: {}{}{}",
            base_prompt, os, arch, cwd, now, super_memory_context, graph_context
        );

        let mut events = vec![];
        if let Ok(data) = std::fs::read_to_string(".brain_events.json") {
            if let Ok(loaded) = serde_json::from_str::<Vec<SessionEvent>>(&data) {
                eprintln!("[Brain B] Resuming from event sourcing log (Loaded {} events)", loaded.len());
                events = loaded;
            }
            }
        }

        let mut event_bus = EventBus::new();
        event_bus.register_before_prompt(Box::new(CircuitBreakerHook { threshold: 12000 }));

        Self {
            ctx: Context::new(),
            tools: Arc::new(Mutex::new(HashMap::new())),
            events,
            event_bus,
            system_prompt,
            llm_provider: LlmProvider::new(client),
            interventions: Arc::new(Mutex::new(Vec::new())),
            role: "SuperAgent".to_string(),
            runtime_config: config,
        }
    }

    /// 多层记忆合成 (Multi-layered Synthesis)
    /// 定期压缩最近的事件历史，生成记忆快照存入 SuperMemory，防止长线任务中断片。
    pub async fn synthesize_memory(&mut self) {
        if self.events.len() < 10 { return; }
        eprintln!("\x1b[35m[Memory Checkpoint] Triggers Layer-1 Synthesis for recent actions...\x1b[0m");
        
        let recent_events: Vec<Value> = self.events.iter().rev().take(15).map(|e| e.to_llm_message()).collect();
        let prompt = "You are a memory synthesizer. Summarize the agent's recent actions and intermediate findings into a concise 'checkpoint' memory block. Focus ONLY on what was achieved, the current state, and the immediate next goal. Do not generate markdown wrapping, just plain text.";
        
        let messages = vec![
            json!({ "role": "system", "content": prompt }),
            json!({ "role": "user", "content": format!("{:?}", recent_events) })
        ];

        if let Ok((msg, _)) = self.llm_provider.chat_with_tools(messages, None, None).await {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                let store = crate::services::supermemory_store::SupermemoryStore::new(&self.runtime_config, None);
                use crate::models::memory::SupermemoryRecord;
                
                store.write_memory(SupermemoryRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: format!("Layer Synthesis - {}", chrono::Utc::now().format("%H:%M:%S")),
                    summary: content.to_string(),
                    tags: vec!["synthesis".to_string(), "checkpoint".to_string()],
                    complexity: "layer1".to_string(),
                    node_type: "synthesis".to_string(),
                    source_graph_path: "agent_loop".to_string(),
                    source_label: "checkpoint".to_string(),
                    file_path: "".to_string(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    embedding: None,
                });
                eprintln!("\x1b[32m[Memory Checkpoint] Successfully saved Layer-1 memory to SuperMemoryStore.\x1b[0m");
            }
        }
    }

    pub fn save_session(&self) {
        if let Ok(data) = serde_json::to_string(&self.events) {
            let _ = std::fs::write(".brain_events.json", data);
        }
    }

    pub fn set_system_prompt(&mut self, base_prompt: &str) {
        self.system_prompt = base_prompt.to_string();
    }

    pub fn set_role(&mut self, role: &str) {
        self.role = role.to_string();
    }

    pub fn load_dynamic_plugins(&mut self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let dirs = vec![cwd.join("dynamic_tools"), cwd.join("skills").join("plugins")];
        for dir in dirs {
            if dir.exists() && dir.is_dir() {
                let dynamic_tools = scan_dynamic_tools(&dir);
                for tool in dynamic_tools {
                    let tool_name = tool.name().to_string();
                    let mut lock = self.tools.lock().unwrap();
                    lock.insert(tool_name.clone(), Box::new(tool));
                    eprintln!("[Brain B] Hot-plugged dynamic tool: {}", tool_name);
                }
            }
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
        let mut loop_count = 0;

        loop {
            loop_count += 1;
            
            // Checkpoint Logic (Multi-layered Synthesis)
            // Trigger every 15 loops if history has enough events
            if loop_count % 15 == 0 && self.events.len() > 20 {
                self.synthesize_memory().await;
            }

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

            // 1.8 触发 RTK 钩子 (Circuit Breaker 等)
            self.event_bus.emit_before_prompt(&mut self.events, &self.llm_provider).await;

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
