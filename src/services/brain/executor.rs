use serde_json::{json, Value};
use std::collections::HashMap;
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

/// DSH 鑼冨紡鐨勮繍琛屽紩鎿庯細涓€鍒囧熀浜?Context 瀹瑰櫒涓?SessionEvent 浜嬩欢娴?
pub struct AgentLoop {
    pub ctx: Context,
    pub plugin_registry: crate::services::brain::tools::registry::PluginRegistry,
    pub events: Vec<SessionEvent>, // 浼氳瘽鍘嗗彶锛堝己绫诲瀷浜嬩欢婧簮锛?
    pub event_bus: EventBus,
    system_prompt: String,
    llm_provider: LlmProvider,
    pub steering_queue: Arc<SteeringQueue>,
    pub interaction_runtime: Arc<InteractionRuntime>,
    pub role: String,
    pub runtime_config: crate::config::runtime::RuntimeConfig,
    session_ledger: SessionLedger,
    session_lifecycle: SessionLifecycle,
    tool_gateway: ToolExecutionGateway,
    evolution_memory: EvolutionMemory,
    compaction_runtime: CompactionRuntime,
    model_attempt_router: ModelAttemptRouter,
    run_prelude_runtime: RunPreludeRuntime,
    pub emit_jsonl: bool,
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

        let system_prompt = format!(
            "{}\n\n[Deterministic Engineering & TDD Protocol]\n\
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
        if events.is_empty() {
            if let Ok(data) = std::fs::read_to_string(".brain_events.json") {
                if let Ok(loaded) = serde_json::from_str::<Vec<SessionEvent>>(&data) {
                    events = loaded;
                    if let Err(error) = session_ledger.sync(&events) {
                        eprintln!("[Brain B] Failed to migrate legacy event log: {}", error);
                    } else {
                        eprintln!(
                            "[Brain B] Migrated {} legacy events into session ledger",
                            events.len()
                        );
                    }
                }
            }
        }
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
        let tool_gateway = ToolExecutionGateway::new(plugin_registry.get_tools_ref());
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

        Self {
            ctx: Context::new(),
            plugin_registry,
            events,
            event_bus,
            system_prompt,
            llm_provider: LlmProvider::new(client),
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
                    println!(
                        "{}",
                        serde_json::json!({ "type": "tool_result", "tool": id, "result": result })
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

    pub fn get_tools_ref(&self) -> Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>> {
        self.plugin_registry.get_tools_ref()
    }

    /// 鏍规嵁 Event Sourcing 鍔ㄦ€佹瀯寤哄綋鍓嶄笂涓嬫枃缁欏ぇ妯″瀷 (Projection)
    fn project_llm_messages(&self) -> Vec<Value> {
        let mut msgs = Vec::new();
        for event in &self.events {
            msgs.push(event.to_llm_message());
        }
        msgs
    }

    /// 鏍稿績鎬濈淮寰幆锛氬畬鍏ㄧ敱浜嬩欢椹卞姩
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
                    self.record_evolution(&mut cognition, false);
                    return Err(error);
                }
            };

            // 2. 灏嗕簨浠舵簮鎶曞奖涓?LLM 鍙娑堟伅
            let projected_messages = self.project_llm_messages();

            // 3. 鍚?LLM 鍙戣捣璇锋眰
            let llm_message = match self
                .call_llm_api(
                    projected_messages,
                    &cognitive_guidance,
                    run_prelude.prompt_projection(),
                )
                .await
            {
                Ok(message) => message,
                Err(error) => {
                    self.record_evolution(&mut cognition, false);
                    return Err(error);
                }
            };

            // 4. 瑙ｆ瀽 LLM 鍥炲锛屽彂灏勬ā鍨嬫€濊€冧簨浠?/ 宸ュ叿璋冪敤浜嬩欢
            if let Some(tool_calls) = llm_message.get("tool_calls").and_then(|v| v.as_array()) {
                for call in tool_calls {
                    let func_name = call["function"]["name"].as_str().unwrap_or("").to_string();
                    let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                    let call_id = call["id"].as_str().unwrap_or("").to_string();

                    // 鍙戝皠宸ュ叿璋冪敤浜嬩欢
                    self.push_event(SessionEvent::ToolCall {
                        id: call_id.clone(),
                        name: func_name.clone(),
                        arguments: args.clone(),
                    });
                    self.save_session();

                    let awaits_user = func_name == "ask_user";
                    if awaits_user {
                        self.transition_session(
                            SessionStatus::NeedsInput,
                            Some("waiting for correlated Interaction Request".to_string()),
                        )?;
                    }

                    let receipt = self.tool_gateway.execute(ToolExecutionRequest {
                        call_id: call_id.clone(),
                        name: func_name.clone(),
                        arguments: args,
                    });
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

                    // 鍙戝皠宸ュ叿鎵ц缁撴灉浜嬩欢
                    self.push_event(SessionEvent::ToolResult {
                        id: call_id,
                        result: tool_result,
                    });
                }
                self.save_session();
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
                self.save_session();
                self.record_evolution(&mut cognition, true);
                return Ok(content.to_string());
            }

            self.record_evolution(&mut cognition, false);
            return Err("Unexpected empty response from LLM".to_string());
        }
    }

    fn record_evolution(&self, cognition: &mut CognitiveRuntime, completed: bool) {
        let report = cognition.evolution_report(completed);
        if let Err(error) = self.evolution_memory.observe(&report) {
            eprintln!("[Brain B] Failed to record evolution evidence: {error}");
        }
    }

    async fn call_llm_api(
        &self,
        messages: Vec<Value>,
        cognitive_guidance: &str,
        run_prelude: &str,
    ) -> Result<Value, String> {
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
            match self
                .llm_provider
                .chat_with_tools(messages.clone(), Some(&dynamic_prompt), tools_param.clone())
                .await
            {
                Ok((msg, usage)) => {
                    eprintln!("\n\x1b[45;37m[Token Usage] Prompt: {}, Completion: {}, Cost: ${:.4}, Latency: {}ms\x1b[0m\n",
                        usage.prompt_tokens, usage.completion_tokens, usage.cost, usage.duration_ms);
                    return Ok(msg);
                }
                Err(error) => {
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
