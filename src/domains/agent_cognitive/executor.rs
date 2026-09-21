use serde_json::{json, Value};
use std::collections::HashMap;


// --- Fix 3: Tool Execution Timeout Guard ---
fn execute_with_timeout(gateway: std::sync::Arc<crate::services::brain::tool_execution_gateway::ToolExecutionGateway>, req: crate::services::brain::tool_execution_gateway::ToolExecutionRequest, timeout_secs: u64) -> crate::services::brain::tool_execution_gateway::ToolExecutionReceipt {
    let call_id = req.call_id.clone();
    let name = req.name.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    
    std::thread::spawn(move || {
        let _ = tx.send(gateway.execute(req));
    });
    
    match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        Ok(res) => res,
        Err(_) => crate::services::brain::tool_execution_gateway::ToolExecutionReceipt {
            call_id,
            name,
            status: crate::services::brain::tool_execution_gateway::ToolExecutionStatus::Failed,
            duration_ms: timeout_secs * 1000,
            result: serde_json::json!(format!("Tool execution strictly timed out after {} seconds. Aborted.", timeout_secs)),
            original_result_chars: 0,
            truncated: false,
            stored_result_reference: None,
            risk: crate::services::brain::tool_policy::ToolRisk::ProcessControl,
            verification: false,
        }
    }
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::services::brain::cognitive_runtime::{CognitiveRuntime, CompletionDecision};
use crate::services::brain::compaction_runtime::{CompactionOutcome, CompactionRuntime};
use crate::services::brain::context::Context;
use crate::services::brain::events::{EventBus, RunOutcome, SessionEvent, SessionStatus};
use crate::services::brain::evolution_memory::EvolutionMemory;
use crate::services::brain::interaction_runtime::InteractionRuntime;
use crate::domains::model_gateway::model_attempt_router::ModelAttemptRouter;
use crate::services::brain::run_prelude::RunPreludeRuntime;
use crate::services::brain::session_ledger::SessionLedger;
use crate::services::brain::session_lifecycle::SessionLifecycle;
use crate::services::brain::steering_queue::{SteeringQueue, SteeringSource};
use crate::services::brain::terminal_receipt::{finish_run, repair_incomplete_runs, start_run};
use crate::services::brain::tool_execution_gateway::{ToolExecutionGateway, ToolExecutionRequest};
use crate::services::brain::tool_pairing::repair_tool_pairs;
use crate::services::brain::tools::dynamic_tool::scan_dynamic_tools;
use crate::domains::model_gateway::providers::LlmProvider;

/// 宸ュ叿瀹氫箟涓庢墽琛岀殑鍥炶皟鎺ュ彛
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;

    // 鎵ц宸ュ叿骞惰繑鍥?JSON 缁撴灉
    fn execute(&self, args: Value) -> Result<Value, String>;

    fn prompt_projection(&self) -> Option<String> {
        None
    }
}

/// Helper function to classify tools as read-only (speculatively parallelizable)
pub fn is_read_only_tool(name: &str) -> bool {
    matches!(
        name,
        "view_file"
            | "read_file"
            | "view_outline"
            | "list_dir"
            | "list_directory"
            | "find_by_name"
            | "grep_search"
            | "search_graph"
            | "query_graph"
            | "get_architecture"
            | "trace_path"
            | "get_code_snippet"
            | "super_memory"
            | "read_tool_result"
            | "web_fetch"
            | "lsp_client"
            | "check_worker_status"
            | "knowledge_search"
            | "corpus_search"
            | "impact_radar"
            | "run_benchmark"
    )
}

/// DSH 范式的运行引擎：一切基于 Context 容器与 SessionEvent 事件流
pub struct AgentLoop {
    pub ctx: Context,
    pub plugin_registry: crate::services::brain::tools::registry::PluginRegistry,
    pub events: Vec<SessionEvent>, // 会话历史（强类型事件溯源）
    pub event_bus: EventBus,
    system_prompt: String,
    llm_provider: LlmProvider,
    fast_llm_provider: LlmProvider,
    pub steering_queue: Arc<SteeringQueue>,
    pub interaction_runtime: Arc<InteractionRuntime>,
    pub role: String,
    pub runtime_config: crate::config::runtime::RuntimeConfig,
    session_ledger: SessionLedger,
    session_lifecycle: SessionLifecycle,
    tool_gateway: std::sync::Arc<crate::services::brain::tool_execution_gateway::ToolExecutionGateway>,
    evolution_memory: EvolutionMemory,
    compaction_runtime: CompactionRuntime,
    model_attempt_router: ModelAttemptRouter,
    run_prelude_runtime: RunPreludeRuntime,
    pub emit_jsonl: bool,
    pub last_token_usage: Option<crate::domains::model_gateway::providers::types::TokenUsage>,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
}

impl AgentLoop {
    pub fn new(base_prompt: &str) -> Self {
        Self::with_model(base_prompt, None)
    }

    pub fn with_model(base_prompt: &str, model_override: Option<&str>) -> Self {
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

        let system_prompt = format!(
            "{}\n\n[Proactive High-Agency Protocol]\n\
             - Autonomous Exploration: Actively use `list_dir`, `find_by_name`, `grep_search`, and `search_graph` to map code structure and trace symbols.\n\
             - Surgical Edits: Always prefer `replace_in_file` and `multi_replace_file_content` for localized code modifications. Never rewrite whole files when changing a few lines.\n\
             - Self-Healing & Resilience: When a tool returns errors or compiler diagnostics, analyze the cause, adapt the hypothesis, and repair in-stride without giving up.\n\n\
             [Deterministic Engineering & TDD Protocol]\n\
             - Test-Driven Verification: When resolving bugs or developing features, write or identify targeted test cases first (Red -> Green -> Refactor), then verify against these tests.\n\
             - Compiler-in-the-Loop: When file editing tools return non-empty `diagnostics`, prioritize correcting these syntax/compiler issues immediately before proceeding to subsequent steps.\n\n\
             [Environment Context]\n- OS: {} ({})\n- Current Directory: {}\n- Current Time: {}",
            base_prompt, os, arch, cwd, now
        );
        let run_prelude_runtime = RunPreludeRuntime::new(config.clone(), &cwd);

        let session_ledger = SessionLedger::default_location();
        let mut events = session_ledger.load().unwrap_or_else(|error| {
            eprintln!("[Brain B] Failed to load session ledger: {}", error);
            Vec::new()
        });
        let pairing_report = repair_tool_pairs(&mut events);
        if pairing_report.changed() {
            eprintln!(
                "[Brain B] Recovered {} interrupted tool call(s), removed {} orphan result(s)",
                pairing_report.repaired_missing_results, pairing_report.removed_orphan_results
            );
            if let Err(error) = session_ledger.sync(&events) {
                eprintln!("[Brain B] Failed to persist repaired tool pairs: {}", error);
            }
        }
        let repaired_runs = repair_incomplete_runs(&mut events);
        if repaired_runs > 0 {
            eprintln!("[Brain B] Recovered {} interrupted run(s)", repaired_runs);
            if let Err(error) = session_ledger.sync(&events) {
                eprintln!("[Brain B] Failed to persist repaired runs: {}", error);
            }
        }
        let session_lifecycle = SessionLifecycle::from_events(&events);
        if !events.is_empty() {
            eprintln!(
                "[Brain B] Resuming session '{}' with {} events",
                session_ledger.session_id(),
                events.len()
            );
        }

        let event_bus = EventBus::new();

        let plugin_registry = crate::services::brain::tools::registry::PluginRegistry::new();
        let tool_gateway = Arc::new(ToolExecutionGateway::new(plugin_registry.get_tools_ref()));
        let evolution_memory = EvolutionMemory::default_location();
        let steering_queue = Arc::new(SteeringQueue::default_location());
        let interaction_runtime = Arc::new(InteractionRuntime::default());
        let injected_steering_ids = events
            .iter()
            .filter_map(|event| match event {
                SessionEvent::SteeringInjected { item_ids, .. } => Some(item_ids),
                _ => None,
            })
            .flatten()
            .cloned()
            .collect();
        if let Err(error) = steering_queue.recover(&injected_steering_ids) {
            eprintln!("[Brain B] Failed to recover steering queue: {error}");
        }

        let llm_settings = crate::config::llm_settings::read_llm_settings();
        let target_model = model_override
            .filter(|m| !m.trim().is_empty())
            .map(|m| m.to_string())
            .unwrap_or_else(|| llm_settings.model.clone());

        let fast_model = if model_override.is_some() {
            let mut temp_settings = llm_settings.clone();
            temp_settings.model = target_model.clone();
            temp_settings.resolved_fast_model()
        } else {
            llm_settings.resolved_fast_model()
        };

        let fast_llm_provider = LlmProvider::with_model_override(client.clone(), Some(fast_model))
            .with_purpose("cognition:fast");
        let llm_provider = LlmProvider::with_model_override(client.clone(), Some(target_model))
            .with_purpose("cognition");

        Self {
            ctx: Context::new(),
            plugin_registry,
            events,
            event_bus,
            system_prompt,
            llm_provider,
            fast_llm_provider,
            steering_queue,
            interaction_runtime,
            role: "SuperAgent".to_string(),
            runtime_config: config,
            session_ledger,
            session_lifecycle,
            tool_gateway,
            evolution_memory,
            compaction_runtime: CompactionRuntime::default(),
            model_attempt_router: ModelAttemptRouter::default(),
            run_prelude_runtime,
            emit_jsonl: false,
            last_token_usage: None,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
        }
    }

    /// 澶氬眰璁板繂鍚堟垚 (Multi-layered Synthesis)
    /// 瀹氭湡鍘嬬缉鏈€杩戠殑浜嬩欢鍘嗗彶锛岀敓鎴愯蹇嗗揩鐓у瓨鍏?SuperMemory锛岄槻姝㈤暱绾夸换鍔′腑鏂墖銆?
    pub async fn synthesize_memory(&mut self) {
        if self.events.len() < 10 {
            return;
        }
        eprintln!(
            "\x1b[35m[Memory Checkpoint] Triggers Layer-1 Synthesis for recent actions...\x1b[0m"
        );

        let recent_events: Vec<Value> = self
            .events
            .iter()
            .rev()
            .take(15)
            .map(|e| e.to_llm_message())
            .collect();
        let prompt = "You are a memory synthesizer. Summarize the agent's recent actions and intermediate findings into a concise 'checkpoint' memory block. Focus ONLY on what was achieved, the current state, and the immediate next goal. Do not generate markdown wrapping, just plain text.";

        let messages = vec![
            json!({ "role": "system", "content": prompt }),
            json!({ "role": "user", "content": format!("{:?}", recent_events) }),
        ];

        if let Ok((msg, _)) = self
            .llm_provider
            .chat_with_tools(messages, None, None)
            .await
        {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                let store = crate::services::supermemory_store::SupermemoryStore::new(
                    &self.runtime_config,
                    None,
                );
                use crate::models::memory::SupermemoryRecord;

                store.write_memory(SupermemoryRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: format!(
                        "Layer Synthesis - {}",
                        chrono::Utc::now().format("%H:%M:%S")
                    ),
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

    pub fn push_event(&mut self, evt: SessionEvent) {
        if self.emit_jsonl {
            match &evt {
                SessionEvent::ModelThought(t) => {
                    println!("{}", serde_json::json!({ "type": "text", "content": t }));
                }
                SessionEvent::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    println!(
                        "{}",
                        serde_json::json!({ "type": "tool_call", "tool": name, "arguments": arguments })
                    );
                }
                SessionEvent::ToolResult { id, result } => {
                    let tool_name = self
                        .events
                        .iter()
                        .rev()
                        .find_map(|e| match e {
                            SessionEvent::ToolCall {
                                id: cid, name, ..
                            } if cid == id => Some(name.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| id.clone());
                    println!(
                        "{}",
                        serde_json::json!({ "type": "tool_result", "tool": tool_name, "call_id": id, "result": result })
                    );
                }
                SessionEvent::SystemWarning(w) => {
                    println!(
                        "{}",
                        serde_json::json!({ "type": "text", "content": format!("[System Warning] {}", w) })
                    );
                }
                _ => {}
            }
        }
        self.events.push(evt);
    }

    pub fn save_session(&self) {
        if let Err(error) = self.session_ledger.sync(&self.events) {
            eprintln!("[Brain B] Failed to persist session ledger: {}", error);
        }
    }

    pub fn session_id(&self) -> &str {
        self.session_ledger.session_id()
    }

    pub fn enqueue_steering(
        &self,
        source: SteeringSource,
        content: &str,
        correlation_id: Option<&str>,
    ) -> Result<String, String> {
        self.steering_queue.enqueue(source, content, correlation_id)
    }

    fn inject_pending_steering(&mut self) -> Result<(), String> {
        let Some(lease) = self.steering_queue.lease(None)? else {
            return Ok(());
        };
        self.push_event(SessionEvent::SteeringInjected {
            lease_id: lease.lease_id.clone(),
            item_ids: lease.item_ids.clone(),
            content: lease.prompt.clone(),
        });
        if let Err(error) = self.session_ledger.sync(&self.events) {
            self.events.pop();
            let _ = self.steering_queue.release(&lease);
            return Err(format!(
                "failed to persist steering injection before delivery: {error}"
            ));
        }
        self.steering_queue.ack(&lease)?;
        Ok(())
    }

    fn transition_session(
        &mut self,
        status: SessionStatus,
        reason: Option<String>,
    ) -> Result<(), String> {
        if let Some(event) = self.session_lifecycle.transition(status, reason)? {
            self.push_event(event);
            self.save_session();
        }
        Ok(())
    }

    pub fn set_system_prompt(&mut self, base_prompt: &str) {
        self.system_prompt = base_prompt.to_string();
    }

    pub fn set_role(&mut self, role: &str) {
        self.role = role.to_string();
    }

    pub fn load_dynamic_plugins(&mut self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let dirs = vec![
            cwd.join("dynamic_tools"),
            cwd.join("skills").join("plugins"),
        ];
        for dir in dirs {
            if dir.exists() && dir.is_dir() {
                let dynamic_tools = scan_dynamic_tools(&dir);
                for tool in dynamic_tools {
                    let tool_name = tool.name().to_string();
                    self.plugin_registry.register(Box::new(tool));

                    eprintln!("[Brain B] Hot-plugged dynamic tool: {}", tool_name);
                }
            }
        }
    }

    pub fn register_tool(&mut self, tool: Box<dyn ToolExecutor>) {
        self.plugin_registry.register(tool);
    }

    pub fn get_tools_ref(&self) -> Arc<Mutex<HashMap<String, Arc<dyn ToolExecutor>>>> {
        self.plugin_registry.get_tools_ref()
    }

    /// 根据 Event Sourcing 动态构建当前上下文给大模型 (Projection)
    /// 融合时间衰减指纹化微剪枝 (Micro-Compaction) 与思考链修剪 (CoT Pruning)
    fn project_llm_messages(&self) -> Vec<Value> {
        project_events_with_micro_compaction(&self.events, 3)
    }

    /// 核心思维循环：完全由事件驱动
    pub async fn run(&mut self, prompt: &str) -> Result<String, String> {
        self.tool_gateway.begin_run();
        self.transition_session(SessionStatus::Running, None)?;
        let run_id = uuid::Uuid::new_v4().to_string();
        start_run(&mut self.events, run_id.clone())?;
        self.save_session();
        let result = self.run_active(prompt).await;
        match &result {
            Ok(_) => finish_run(&mut self.events, &run_id, RunOutcome::Completed, None)?,
            Err(error) => finish_run(
                &mut self.events,
                &run_id,
                RunOutcome::Failed,
                Some(error.clone()),
            )?,
        }
        self.save_session();
        let terminal_transition = match &result {
            Ok(_) => self.transition_session(SessionStatus::Completed, None),
            Err(error) => self.transition_session(SessionStatus::Failed, Some(error.clone())),
        };
        terminal_transition?;
        result
    }

    async fn run_active(&mut self, prompt: &str) -> Result<String, String> {
        // 1. 鍙戝皠鐢ㄦ埛杈撳叆浜嬩欢
        self.events
            .push(SessionEvent::UserInput(prompt.to_string()));
        self.save_session();
        let run_prelude = self.run_prelude_runtime.prepare(prompt);
        eprintln!("[Run Prelude] capability route: {:?}", run_prelude.routes);
        let mut cognition = CognitiveRuntime::begin(prompt);
        match self
            .evolution_memory
            .promoted_guidance(cognition.task_kind())
        {
            Ok(guidance) => cognition.apply_promoted_guidance(guidance),
            Err(error) => eprintln!("[Brain B] Failed to load promoted strategies: {error}"),
        }
        match self.evolution_memory.retrieve_relevant_lessons(prompt) {
            Ok(lessons) => {
                if !lessons.is_empty() {
                    eprintln!(
                        "[Hermes Evolution] Recalled {} distilled lessons for current task",
                        lessons.len()
                    );
                    cognition.apply_distilled_lessons(lessons);
                }
            }
            Err(error) => eprintln!("[Hermes Evolution] Failed to retrieve lessons: {error}"),
        }
        let mut loop_count = 0;

        loop {
            loop_count += 1;


            // 1.5 Lease runtime steering, persist its delivery fact, then ack.
            self.inject_pending_steering()?;

            // 1.8 瑙﹀彂 RTK 閽╁瓙 (Circuit Breaker 绛?
            self.event_bus
                .emit_before_prompt(&mut self.events, &self.llm_provider)
                .await;
            match self
                .compaction_runtime
                .compact_if_needed(&mut self.events, self.llm_provider.max_context_tokens())
            {
                CompactionOutcome::Compacted {
                    compacted_events,
                    before_tokens,
                    after_tokens,
                } => {
                    eprintln!(
                        "[Compaction] checkpointed {} events ({} -> {} estimated tokens)",
                        compacted_events, before_tokens, after_tokens
                    );
                    self.save_session();
                    self.synthesize_memory().await;
                }
                CompactionOutcome::Deferred {
                    estimated_tokens,
                    reason,
                } => {
                    eprintln!(
                        "[Compaction] deferred at {} estimated tokens: {}",
                        estimated_tokens, reason
                    );
                }
                CompactionOutcome::NotNeeded { .. } => {}
            }
            self.save_session();

            let cognitive_guidance = match cognition.next_guidance() {
                Ok(guidance) => guidance,
                Err(error) => {
                    self.record_evolution(&mut cognition, prompt, false);
                    return Err(error);
                }
            };

            let brain_tier = cognition.select_brain_tier();

            // 2. 将事件源投影为 LLM 可读消息
            let projected_messages = self.project_llm_messages();

            // 3. 向 LLM 发起请求 (快慢脑双模型协同编排)
            let (llm_message, usage) = match self
                .call_llm_api(
                    projected_messages,
                    &cognitive_guidance,
                    run_prelude.prompt_projection(),
                    brain_tier,
                )
                .await
            {
                Ok(res) => res,
                Err(error) => {
                    self.record_evolution(&mut cognition, prompt, false);
                    return Err(error);
                }
            };

            self.last_token_usage = Some(usage.clone());
            self.total_prompt_tokens += usage.prompt_tokens as u64;
            self.total_completion_tokens += usage.completion_tokens as u64;

            if self.emit_jsonl {
                let max_ctx = self.llm_provider.max_context_tokens();
                let pct = if max_ctx > 0 {
                    ((usage.prompt_tokens as f32 / max_ctx as f32) * 100.0).min(100.0)
                } else {
                    0.0
                };
                println!(
                    "{}",
                    serde_json::json!({
                        "type": "context_usage",
                        "prompt_tokens": usage.prompt_tokens,
                        "completion_tokens": usage.completion_tokens,
                        "total_tokens": usage.prompt_tokens + usage.completion_tokens,
                        "max_tokens": max_ctx,
                        "percentage": pct
                    })
                );
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }

            // 4. 解析 LLM 回复，发射模型思考事件 / 工具调用事件
            if let Some(tool_calls) = llm_message.get("tool_calls").and_then(|v| v.as_array()) {
                struct ParsedCall {
                    call_id: String,
                    func_name: String,
                    args: Value,
                    is_read_only: bool,
                }

                let mut parsed_calls: Vec<ParsedCall> = Vec::with_capacity(tool_calls.len());
                for call in tool_calls {
                    let func_name = call["function"]["name"].as_str().unwrap_or("").to_string();
                    let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    let call_id = call["id"].as_str().unwrap_or("").to_string();
                    let is_read_only = is_read_only_tool(&func_name);

                    parsed_calls.push(ParsedCall {
                        call_id,
                        func_name,
                        args,
                        is_read_only,
                    });
                }

                // Partition parsed_calls into contiguous chunks:
                // Contiguous read-only calls form a chunk (can execute concurrently),
                // Mutating calls form single-element chunks (executed sequentially with barriers).
                let mut chunks: Vec<Vec<ParsedCall>> = Vec::new();
                for call in parsed_calls {
                    if call.is_read_only {
                        if let Some(last_chunk) = chunks.last_mut() {
                            if last_chunk.first().map(|c| c.is_read_only).unwrap_or(false) {
                                last_chunk.push(call);
                                continue;
                            }
                        }
                        chunks.push(vec![call]);
                    } else {
                        chunks.push(vec![call]);
                    }
                }

                for chunk in chunks {
                    let is_read_only_chunk = chunk.first().map(|c| c.is_read_only).unwrap_or(false);

                    if is_read_only_chunk && chunk.len() > 1 {
                        // Parallel branch for speculative read-only tool calls
                        for item in &chunk {
                            self.push_event(SessionEvent::ToolCall {
                                id: item.call_id.clone(),
                                name: item.func_name.clone(),
                                arguments: item.args.clone(),
                            });
                        }
                        self.save_session();

                        eprintln!(
                            "[Brain B] Speculative parallel execution of {} read-only tools: {:?}",
                            chunk.len(),
                            chunk.iter().map(|c| c.func_name.as_str()).collect::<Vec<_>>()
                        );

                        let mut handles = Vec::with_capacity(chunk.len());
                        for item in chunk {
                            let gateway = self.tool_gateway.clone();
                            let req = ToolExecutionRequest {
                                call_id: item.call_id,
                                name: item.func_name,
                                arguments: item.args,
                            };
                            handles.push(tokio::task::spawn_blocking(move || execute_with_timeout(gateway, req, 60)));
                        }

                        for handle in handles {
                            if let Ok(receipt) = handle.await {
                                eprintln!(
                                    "[Brain B] Tool {} (parallel) finished with {:?} in {}ms",
                                    receipt.name, receipt.status, receipt.duration_ms
                                );
                                cognition.observe_tool(receipt.status, receipt.risk);
                                self.push_event(SessionEvent::ToolResult {
                                    id: receipt.call_id,
                                    result: receipt.result,
                                });
                            }
                        }
                        self.save_session();
                    } else {
                        // Sequential branch (mutating tools or single tool call)
                        for item in chunk {
                            self.push_event(SessionEvent::ToolCall {
                                id: item.call_id.clone(),
                                name: item.func_name.clone(),
                                arguments: item.args.clone(),
                            });
                            self.save_session();

                            let awaits_user = item.func_name == "ask_user";
                            if awaits_user {
                                self.transition_session(
                                    SessionStatus::NeedsInput,
                                    Some("waiting for correlated Interaction Request".to_string()),
                                )?;
                            }

                            let req = ToolExecutionRequest {
                                call_id: item.call_id.clone(),
                                name: item.func_name.clone(),
                                arguments: item.args,
                            };
                            let receipt = execute_with_timeout(self.tool_gateway.clone(), req, 60);

                            if awaits_user {
                                self.transition_session(
                                    SessionStatus::Running,
                                    Some("Interaction Request settled".to_string()),
                                )?;
                            }

                            eprintln!(
                                "[Brain B] Tool {} finished with {:?} in {}ms",
                                receipt.name, receipt.status, receipt.duration_ms
                            );
                            cognition.observe_tool(receipt.status, receipt.risk);
                            let tool_result = receipt.result;

                            self.push_event(SessionEvent::ToolResult {
                                id: item.call_id,
                                result: tool_result,
                            });

                            if item.func_name == "revert_to_checkpoint"
                                && receipt.status == crate::services::brain::tool_execution_gateway::ToolExecutionStatus::Succeeded
                            {
                                let user_input_idx = self
                                    .events
                                    .iter()
                                    .position(|e| matches!(e, SessionEvent::UserInput(_)))
                                    .unwrap_or(0);
                                if self.events.len() > user_input_idx + 2 {
                                    eprintln!(
                                        "\x1b[35m[Mental State Pruning] Time travel: Pruned {} failure events from session\x1b[0m",
                                        self.events.len() - (user_input_idx + 2)
                                    );
                                    self.events.truncate(user_input_idx + 2);
                                }
                                self.push_event(SessionEvent::SystemWarning(
                                    "[TIME TRAVEL CLEAN STATE] Workspace and mental state reverted to clean checkpoint. Former failure context discarded. Formulate an entirely alternative strategy and proceed with fresh evidence.".to_string(),
                                ));
                            }

                            self.save_session();
                        }
                    }
                }
                continue;
            }

            // 7. 鏈€缁堟枃鏈洖澶?
            if let Some(content) = llm_message.get("content").and_then(|v| v.as_str()) {
                self.events
                    .push(SessionEvent::ModelThought(content.to_string()));
                if let CompletionDecision::Revise { reason } = cognition.review_completion(content)
                {
                    self.push_event(SessionEvent::SystemWarning(format!(
                        "Completion review required: {reason} Treat the preceding assistant message as a draft. Gather missing evidence if needed, then produce the corrected final answer."
                    )));
                    self.save_session();
                    continue;
                }
                if self.emit_jsonl {
                    println!("{}", serde_json::json!({ "type": "text", "content": content }));
                }
                self.save_session();
                self.record_evolution(&mut cognition, prompt, true);
                let _ = crate::domains::agent_cognitive::plan_reflection::ReflectionEngine::distill_and_persist(
                    &self.events,
                    &self.runtime_config,
                    Some(self.run_prelude_runtime.project_path()),
                );
                return Ok(content.to_string());
            }

            self.record_evolution(&mut cognition, prompt, false);
            return Err("Unexpected empty response from LLM".to_string());
        }
    }

    fn record_evolution(&self, cognition: &mut CognitiveRuntime, prompt: &str, completed: bool) {
        let report = cognition.evolution_report(completed);
        if let Err(error) = self.evolution_memory.observe(&report) {
            eprintln!("[Brain B] Failed to record evolution evidence: {error}");
        }
        if completed {
            if let Some(new_lesson) = self.evolution_memory.distill_trajectory(prompt, &self.events, true) {
                eprintln!(
                    "\x1b[32m[Hermes Distillation] Successfully distilled new lesson: [{}] {} (Trap: {})\x1b[0m",
                    new_lesson.domain, new_lesson.golden_solution, new_lesson.trap_pattern
                );
            }
        }
    }

    async fn call_llm_api(
        &self,
        messages: Vec<Value>,
        cognitive_guidance: &str,
        run_prelude: &str,
        brain_tier: crate::domains::model_gateway::model_attempt_router::BrainTier,
    ) -> Result<(Value, crate::domains::model_gateway::providers::types::TokenUsage), String> {
        let (provider, tier_label) = match brain_tier {
            crate::domains::model_gateway::model_attempt_router::BrainTier::Fast => {
                (&self.fast_llm_provider, "Fast Brain")
            }
            crate::domains::model_gateway::model_attempt_router::BrainTier::Slow => {
                (&self.llm_provider, "Slow Brain")
            }
        };
        eprintln!("\x1b[34m[Dual-Brain] Orchestrating turn with {}...\x1b[0m", tier_label);

        let mut dynamic_prompt = self.system_prompt.clone();
        dynamic_prompt.push_str("\n\n");
        dynamic_prompt.push_str(cognitive_guidance);
        dynamic_prompt.push_str("\n\n");
        dynamic_prompt.push_str(run_prelude);

        let tools_schema: Vec<Value> = {
            let tools_arc = self.plugin_registry.get_tools_ref();
            let tools_guard = tools_arc.lock().unwrap();
            let mut projections = Vec::new();

            let schema = tools_guard
                .values()
                .map(|t| {
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
                })
                .collect();

            if !projections.is_empty() {
                dynamic_prompt.push_str("\n\n# Dynamic Tool Projections (Active Skills)\n");
                dynamic_prompt.push_str(&projections.join("\n\n"));
            }

            schema
        };

        let tools_param = if tools_schema.is_empty() {
            None
        } else {
            Some(json!(tools_schema))
        };

        let mut attempt = 1_u8;
        loop {
            match provider
                .chat_with_tools(messages.clone(), Some(&dynamic_prompt), tools_param.clone())
                .await
            {
                Ok((msg, usage)) => {
                    eprintln!("\n\x1b[45;37m[Token Usage - {}] Prompt: {}, Completion: {}, Cost: ${:.4}, Latency: {}ms\x1b[0m\n",
                        tier_label, usage.prompt_tokens, usage.completion_tokens, usage.cost, usage.duration_ms);
                    return Ok((msg, usage));
                }
                Err(error) => {
                    if brain_tier == crate::domains::model_gateway::model_attempt_router::BrainTier::Fast && attempt == 1 {
                        eprintln!("[Dual-Brain] Fast Brain returned error ({error}); automatically escalating to Slow Brain...");
                        return Box::pin(self.call_llm_api(
                            messages,
                            cognitive_guidance,
                            run_prelude,
                            crate::domains::model_gateway::model_attempt_router::BrainTier::Slow,
                        )).await;
                    }
                    let class = self.model_attempt_router.classify(&error);
                    let Some(delay) = self.model_attempt_router.retry_delay(attempt, &error) else {
                        return Err(format!(
                            "model request failed after {attempt} attempt(s) ({class:?}): {error}"
                        ));
                    };
                    eprintln!(
                        "[ModelAttempt] attempt {attempt} failed as {class:?}; retrying in {}ms",
                        delay.as_millis()
                    );
                    tokio::time::sleep(delay).await;
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }
}

/// Helper: Formats a compact fingerprint for a folded tool result
pub fn format_folded_tool_result(tool_name: &str, result: &Value) -> String {
    let raw_str = if let Some(s) = result.as_str() {
        s.to_string()
    } else {
        result.to_string()
    };
    let char_count = raw_str.len();
    let line_count = raw_str.lines().count();

    // Check if result indicates success or error
    let status_hint = if let Some(status) = result.get("status").and_then(|v| v.as_str()) {
        format!("status: {}", status)
    } else if let Some(code) = result.get("exit_code").and_then(|v| v.as_i64()) {
        format!("exit_code: {}", code)
    } else if raw_str.to_lowercase().contains("error") || raw_str.to_lowercase().contains("fail") {
        "with error warnings".to_string()
    } else {
        "success".to_string()
    };

    // Extract first line or brief excerpt
    let first_line = raw_str
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('{') && !l.starts_with('"'))
        .unwrap_or("");
    let brief_hint = if !first_line.is_empty() && first_line.len() <= 80 {
        format!("; summary: \"{}\"", first_line)
    } else {
        String::new()
    };

    format!(
        "[Tool Result Folded]: {} completed ({}, {} lines/{} chars folded to conserve context{})",
        tool_name, status_hint, line_count, char_count, brief_hint
    )
}

/// Helper: Prunes historical CoT (<think>...</think>) reasoning blocks
pub fn prune_cot_thinking(text: &str) -> String {
    if let (Some(start_tag), Some(end_tag)) = (text.find("<think>"), text.find("</think>")) {
        if start_tag < end_tag {
            let before = &text[..start_tag];
            let after = &text[end_tag + "</think>".len()..];
            let pruned = format!("{}{}", before.trim(), after.trim());
            let final_content = pruned.trim();
            if final_content.is_empty() {
                return "[Historical reasoning completed]".to_string();
            } else {
                return format!("[Reasoning: completed]\n{}", final_content);
            }
        }
    }

    if text.starts_with("<think>") {
        return "[Historical reasoning completed]".to_string();
    }

    if text.len() > 1200 {
        let prefix: String = text.chars().take(250).collect();
        return format!("{}...\n[Historical reasoning folded ({} chars)]", prefix.trim(), text.len());
    }

    text.to_string()
}

/// Event Sourcing 投影器：具备时间衰减指纹化折叠与思考链修剪的高阶投影
pub fn project_events_with_micro_compaction(
    events: &[SessionEvent],
    active_turns: usize,
) -> Vec<Value> {
    if events.is_empty() {
        return Vec::new();
    }

    // 1. 建立 ToolCall ID 到 Tool 名字与参数的快速查找映射
    let mut call_names: HashMap<String, String> = HashMap::new();
    // 2. 为每个事件分配逻辑轮次 (logical turn)
    let mut event_turns = vec![0_usize; events.len()];
    let mut current_turn = 0_usize;
    let mut in_tool_batch = false;

    for (i, evt) in events.iter().enumerate() {
        match evt {
            SessionEvent::UserInput(_) => {
                if i > 0 {
                    current_turn += 1;
                }
                in_tool_batch = false;
                event_turns[i] = current_turn;
            }
            SessionEvent::ModelThought(_) => {
                if i > 0 && !in_tool_batch {
                    current_turn += 1;
                }
                in_tool_batch = false;
                event_turns[i] = current_turn;
            }
            SessionEvent::ToolCall { id, name, .. } => {
                call_names.insert(id.clone(), name.clone());
                if !in_tool_batch && i > 0 {
                    current_turn += 1;
                    in_tool_batch = true;
                }
                event_turns[i] = current_turn;
            }
            SessionEvent::ToolResult { id: _, .. } => {
                // ToolResult 属于触发它的同一个逻辑轮次
                event_turns[i] = current_turn;
            }
            _ => {
                event_turns[i] = current_turn;
            }
        }
    }

    let max_turn = current_turn;
    let active_turn_threshold = max_turn.saturating_sub(active_turns.saturating_sub(1));

    let mut projected = Vec::with_capacity(events.len());

    for (i, evt) in events.iter().enumerate() {
        let is_active = event_turns[i] >= active_turn_threshold;

        if is_active {
            // 活跃窗口：100% 原始投射
            projected.push(evt.to_llm_message());
        } else {
            // 历史窗口：微剪枝与指纹化折叠
            match evt {
                SessionEvent::ToolResult { id, result } => {
                    let raw_str = if let Some(s) = result.as_str() {
                        s.to_string()
                    } else {
                        result.to_string()
                    };

                    if raw_str.len() <= 300 {
                        // 小型返回值直接保留
                        projected.push(evt.to_llm_message());
                    } else {
                        // 动态指纹化折叠
                        let tool_name = call_names.get(id).map(|s| s.as_str()).unwrap_or("tool");
                        let folded_content = format_folded_tool_result(tool_name, result);
                        projected.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": folded_content
                        }));
                    }
                }
                SessionEvent::ModelThought(thought) => {
                    let pruned = prune_cot_thinking(thought);
                    projected.push(serde_json::json!({
                        "role": "assistant",
                        "content": pruned
                    }));
                }
                _ => {
                    projected.push(evt.to_llm_message());
                }
            }
        }
    }

    projected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_micro_compaction_folds_older_large_tool_results_and_prunes_cot() {
        let large_output = "a".repeat(1500);
        let events = vec![
            // Turn 0
            SessionEvent::UserInput("Fix issue in server".to_string()),
            // Turn 1 (Historical)
            SessionEvent::ModelThought("<think>Let me deeply analyze the problem first...</think>I will inspect the file".to_string()),
            SessionEvent::ToolCall {
                id: "call-1".to_string(),
                name: "view_file".to_string(),
                arguments: serde_json::json!({ "path": "src/main.rs" }),
            },
            SessionEvent::ToolResult {
                id: "call-1".to_string(),
                result: serde_json::json!({ "content": large_output }),
            },
            // Turn 2 (Historical if active_turns = 2)
            SessionEvent::ModelThought("Now checking the logs".to_string()),
            SessionEvent::ToolCall {
                id: "call-2".to_string(),
                name: "run_command".to_string(),
                arguments: serde_json::json!({ "cmd": "cargo check" }),
            },
            SessionEvent::ToolResult {
                id: "call-2".to_string(),
                result: serde_json::json!({ "status": "ok", "short": "pass" }), // <= 300 chars, not folded
            },
            // Turn 3 (Active)
            SessionEvent::ModelThought("Modifying the code".to_string()),
            SessionEvent::ToolCall {
                id: "call-3".to_string(),
                name: "replace_in_file".to_string(),
                arguments: serde_json::json!({ "path": "src/main.rs" }),
            },
            SessionEvent::ToolResult {
                id: "call-3".to_string(),
                result: serde_json::json!({ "content": large_output }), // Active, so NOT folded!
            },
            // Turn 4 (Active)
            SessionEvent::ModelThought("All tasks completed!".to_string()),
        ];

        let projected = project_events_with_micro_compaction(&events, 2);
        assert_eq!(projected.len(), events.len());

        // 1. Turn 1 Thought should have <think> pruned
        let turn1_thought = projected[1]["content"].as_str().unwrap();
        assert!(!turn1_thought.contains("<think>"));
        assert!(!turn1_thought.contains("deeply analyze"));
        assert!(turn1_thought.contains("[Reasoning: completed]"));
        assert!(turn1_thought.contains("I will inspect the file"));

        // 2. Turn 1 ToolResult (1500 chars) should be folded to fingerprint summary
        let turn1_result = &projected[3];
        assert_eq!(turn1_result["role"], "tool");
        assert_eq!(turn1_result["tool_call_id"], "call-1");
        let turn1_res_content = turn1_result["content"].as_str().unwrap();
        assert!(turn1_res_content.contains("[Tool Result Folded]: view_file completed"));
        assert!(turn1_res_content.contains("folded to conserve context"));
        assert!(turn1_res_content.len() < 250);

        // 3. Turn 2 ToolResult (short) should remain verbatim
        let turn2_result = &projected[6];
        assert_eq!(turn2_result["tool_call_id"], "call-2");
        assert!(turn2_result["content"].as_str().unwrap().contains("pass"));

        // 4. Turn 3 ToolResult (large, but in active window) must NOT be folded
        let turn3_result = &projected[9];
        assert_eq!(turn3_result["tool_call_id"], "call-3");
        assert!(turn3_result["content"].as_str().unwrap().contains(&large_output));
    }
}
