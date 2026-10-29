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
use crate::domains::version_control::regression_guard::{AuditVerdict, RegressionRollbackGuard};
use crate::domains::agent_cognitive::circuit_breaker::CircuitBreakerGuard;
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
pub use super::event_projection::{format_folded_tool_result, prune_cot_thinking, project_events_with_micro_compaction};

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
            | "code_review"
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
    pub stuck_detector: crate::domains::agent_cognitive::stuck_detector::StuckDetector,
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
            "{}\n\n[Elite Agent Tooling & Operating Protocol]\n\
             - Primary Core Toolset: Focus your actions around the 8 standard core tools:\n\
               1. `view_file` (alias: `read_file`): Read and inspect file contents with pagination and RTK filters.\n\
               2. `replace_in_file` (alias: `edit_file`): Localized, surgical code edits using exact search and replace text.\n\
               3. `write_to_file` (alias: `write_file`): Create new files or perform necessary full rewrites.\n\
               4. `run_command` (alias: `bash`): Execute builds, tests, lint checks, and terminal commands.\n\
               5. `grep_search`: Fast regex/text code search across the workspace.\n\
               6. `find_by_name`: Rapidly locate files by filename or glob pattern.\n\
               7. `list_dir`: Explore directory tree structures.\n\
               8. `manage_task`: Inspect or maintain your .superagent_todo.md physical execution checklist.\n\
             - Working Memory on Disk (Manus/Devin Pattern):\n\
               For any task that requires multi-step planning or modifying code across multiple files:\n\
               1. First create an execution checklist in `.superagent_todo.md` using `manage_task` (Action: \"write\").\n\
               2. Execute step-by-step; mark items completed physically using `manage_task`.\n\
               3. Do not mark completed or conclude before targeted tests run and pass via `run_command`.\n\n\
             [Deterministic Engineering & TDD Protocol]\n\
             - Test-Driven Verification: When resolving bugs or developing features, write or identify targeted test cases first (Red -> Green -> Refactor), then verify against these tests.\n\
             - Compiler-in-the-Loop: When file editing tools return non-empty `diagnostics` or compiler errors, prioritize correcting these syntax/compiler issues immediately before proceeding.\n\n\
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
            stuck_detector: crate::domains::agent_cognitive::stuck_detector::StuckDetector::new(10, 2),
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
                    // 内部系统质检/反思指令只作为 thinking 思考流或内部事件记录，绝对严禁作为正文 text 发射给用户
                    println!(
                        "{}",
                        serde_json::json!({ "type": "thinking", "content": format!("【系统自检反思】: {}", w) })
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
        self.run_with_images(prompt, Vec::new()).await
    }

    /// 多模态入口：`images` 为 data URL（由 worker 从落盘文件读回并内联）。
    /// 空图片列表等价于 [`Self::run`]。
    pub async fn run_with_images(
        &mut self,
        prompt: &str,
        images: Vec<String>,
    ) -> Result<String, String> {
        self.tool_gateway.begin_run();
        self.transition_session(SessionStatus::Running, None)?;
        let run_id = uuid::Uuid::new_v4().to_string();
        start_run(&mut self.events, run_id.clone())?;
        self.save_session();
        let result = self.run_active(prompt, images).await;
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

    async fn run_active(
        &mut self,
        prompt: &str,
        initial_images: Vec<String>,
    ) -> Result<String, String> {
        // Clean legacy completed todo file if no tasks remain unfinished
        let todo_path = std::env::current_dir()
            .unwrap_or_default()
            .join(".superagent_todo.md");
        if todo_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&todo_path) {
                let has_unfinished = content.lines().any(|l| {
                    let t = l.trim();
                    t.starts_with("- [ ]")
                        || t.starts_with("* [ ]")
                        || t.contains("[in_progress]")
                        || t.contains("[pending]")
                });
                if !has_unfinished {
                    let _ = std::fs::remove_file(&todo_path);
                }
            }
        }

        // 1. 发射用户输入事件（含图片时走多模态事件，投影为 content 数组）
        if initial_images.is_empty() {
            self.events
                .push(SessionEvent::UserInput(prompt.to_string()));
        } else {
            self.events.push(SessionEvent::UserInputWithImages {
                text: prompt.to_string(),
                images: initial_images,
            });
        }
        self.save_session();
        let run_prelude = self.run_prelude_runtime.prepare(prompt);
        eprintln!("[Run Prelude] capability route: {:?}", run_prelude.routes);

        // 初始化防改坏守卫并毫秒级捕获入场基线快照 (Git Baseline Checkpoint)
        let mut rollback_guard = RegressionRollbackGuard::new(
            self.run_prelude_runtime.project_path(),
            &uuid::Uuid::new_v4().to_string()[..8],
        ).with_default_architectural_rules();
        if let Err(e) = rollback_guard.capture_baseline() {
            eprintln!("[RegressionGuard] Warning: Failed to capture baseline snapshot: {e}");
        }
        let mut circuit_breaker = CircuitBreakerGuard::default();
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
        let mut consecutive_meta_only_turns = 0;

        loop {
            loop_count += 1;
            if loop_count > 60 {
                eprintln!("\x1b[33m[AgentLoop Guard] Turn limit reached (60 turns). Forcing final synthesis.\x1b[0m");
                if let Some(last_thought) = self.events.iter().rev().find_map(|e| match e {
                    SessionEvent::ModelThought(t) => Some(t.clone()),
                    _ => None,
                }) {
                    self.save_session();
                    return Ok(last_thought);
                }
                return Ok("Task execution completed (turn limit reached).".to_string());
            }


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
                    if !rollback_guard.is_rolled_back && !rollback_guard.mutated_files.is_empty() {
                let _ = rollback_guard.rollback_workspace("智能体异常中断，自动还原工作区以保护代码纯净");
            }
            self.record_evolution(&mut cognition, prompt, false);
                    return Err(error);
                }
            };

            let brain_tier = cognition.select_brain_tier();

            // 2. 将事件源投影为 LLM 可读消息
            let projected_messages = self.project_llm_messages();

            let mut dynamic_guidance = cognitive_guidance.clone();
            let todo_path = std::env::current_dir()
                .unwrap_or_default()
                .join(".superagent_todo.md");
            if let Ok(todo_content) = std::fs::read_to_string(&todo_path) {
                let trimmed = todo_content.trim();
                if !trimmed.is_empty() {
                    let has_unfinished = trimmed.lines().any(|l| {
                        let t = l.trim();
                        t.starts_with("- [ ]")
                            || t.starts_with("* [ ]")
                            || t.contains("[in_progress]")
                            || t.contains("[pending]")
                    });
                    let directive = if has_unfinished {
                        "Checklist Directive: Refer to your active checklist above. Mark tasks as complete using `manage_task` when finished."
                    } else {
                        "Checklist Directive: ALL active tasks are marked completed! Do NOT call `manage_task` again. Provide your final delivery summary directly and conclude the turn."
                    };
                    dynamic_guidance.push_str(&format!(
                        "\n\n[CURRENT ACTIVE TASK CHECKLIST]\n\
                        --------------------------------------------------\n\
                        {}\n\
                        --------------------------------------------------\n\
                        {}",
                        trimmed, directive
                    ));
                }
            } else {
                dynamic_guidance.push_str(
                    "\n\n[Manus Protocol: Working Memory]\nIf this task involves multiple steps or code modifications, please start by calling `manage_task` (Action: 'write') to initialize your .superagent_todo.md execution checklist."
                );
            }

            if consecutive_meta_only_turns >= 1 {
                dynamic_guidance.push_str(
                    "\n\n[NOTICE: CHECKLIST SETTLED] You have already updated the checklist. Do NOT call `manage_task` again in this turn. Perform real tool actions or provide your final response."
                );
            }

            // 注入蜂群全局黑板共识 (Hive Collective Blackboard Active Projection)
            let bb_entries = crate::domains::agent_cognitive::agent_blackboard::AgentBlackboard::global().get_all_history();
            if !bb_entries.is_empty() {
                let recent_bb: Vec<_> = bb_entries.iter().rev().take(3).collect();
                dynamic_guidance.push_str("\n\n[HIVE BLACKBOARD COLLECTIVE CONSENSUS]\n");
                for entry in recent_bb {
                    dynamic_guidance.push_str(&format!("- [{}] {}: {}\n", entry.publisher, entry.tag, entry.value.trim()));
                }
            }

            // 3. 向 LLM 发起请求 (快慢脑双模型协同编排)
            let (llm_message, usage) = match self
                .call_llm_api(
                    projected_messages,
                    &dynamic_guidance,
                    run_prelude.prompt_projection(),
                    brain_tier,
                )
                .await
            {
                Ok(res) => res,
                Err(error) => {
                    if !rollback_guard.is_rolled_back && !rollback_guard.mutated_files.is_empty() {
                let _ = rollback_guard.rollback_workspace("智能体异常中断，自动还原工作区以保护代码纯净");
            }
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

            // 4. 解析并高亮模型的思维推演流 (Inner Thinking / Extended Reasoning)
            // 仅将真正的 reasoning_content 归类为 thinking 事件；content 的兜底
            // 只进会话账本，避免与正文 text 事件在前端重复渲染。
            let reasoning_only = llm_message
                .get("reasoning_content")
                .and_then(|v| v.as_str());
            let thought_content = reasoning_only.or_else(|| {
                llm_message.get("content").and_then(|v| v.as_str())
            });
            if let Some(thought) = thought_content {
                let trimmed = thought.trim();
                if !trimmed.is_empty() {
                    eprintln!("\x1b[36;1m[Inner Thinking / 推演沙盘]\x1b[0m\n{}\n", trimmed);
                    if let Some(reasoning) = reasoning_only.map(str::trim).filter(|s| !s.is_empty()) {
                        if self.emit_jsonl {
                            println!(
                                "{}",
                                serde_json::json!({ "type": "thinking", "content": reasoning })
                            );
                            use std::io::Write;
                            let _ = std::io::stdout().flush();
                        }
                    }
                    self.events.push(SessionEvent::ModelThought(trimmed.to_string()));
                    self.save_session();

                    // 同步到蜂群意识流中枢 (Hive-Mind Collective Consciousness Stream)
                    if let Ok(mut consciousness) = crate::domains::swarm_intelligence::GLOBAL_CONSCIOUSNESS.write() {
                        consciousness.record_thought(
                            &self.role,
                            crate::domains::swarm_intelligence::ThoughtType::Reflection,
                            trimmed,
                            vec!["reasoning".into(), "inner_thinking".into()]
                        );
                    }
                    // 沉淀至全局黑板供蜂群与多角色智能体实时共享
                    crate::domains::agent_cognitive::agent_blackboard::AgentBlackboard::global().post(
                        format!("th_{}", &uuid::Uuid::new_v4().to_string()[..8]),
                        trimmed.chars().take(200).collect::<String>(),
                        &self.role,
                        "deliberation",
                    );
                }
            }

            // 5. 解析 LLM 回复，发射工具调用事件
            // 5. 解析 LLM 回复（结合小模型脏 JSON 自愈与内嵌工具提取器）
            let raw_tool_calls_opt = llm_message.get("tool_calls").and_then(|v| v.as_array());
            let synthesized_calls: Vec<Value> = if raw_tool_calls_opt.is_none() || raw_tool_calls_opt.map(|a| a.is_empty()).unwrap_or(true) {
                // 如果模型没有发射原生 tool_calls，尝试从小模型文本 content 中嗅探内嵌工具
                if let Some(content_str) = llm_message.get("content").and_then(|v| v.as_str()) {
                    if let Some((tool_name, tool_args)) = super::fuzzy_json_repair::FuzzyJsonRepair::extract_embedded_tool_call(content_str) {
                        eprintln!("\x1b[36;1m[FuzzyRepair] ⚡ 从开源小模型正文中成功捕获内嵌工具调用: '{}'\x1b[0m", tool_name);
                        vec![json!({
                            "id": format!("call_fuzzy_{}", &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]),
                            "type": "function",
                            "function": {
                                "name": tool_name,
                                "arguments": serde_json::to_string(&tool_args).unwrap_or_default()
                            }
                        })]
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                raw_tool_calls_opt.cloned().unwrap_or_default()
            };

            if !synthesized_calls.is_empty() {
                let tool_calls = &synthesized_calls;
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
                    let args: Value = super::fuzzy_json_repair::FuzzyJsonRepair::parse_or_repair(args_str).unwrap_or(json!({}));
                    let call_id = call["id"].as_str().unwrap_or("").to_string();
                    let is_read_only = is_read_only_tool(&func_name);

                    parsed_calls.push(ParsedCall {
                        call_id,
                        func_name,
                        args,
                        is_read_only,
                    });
                }

                let all_manage_task = !parsed_calls.is_empty()
                    && parsed_calls.iter().all(|c| c.func_name == "manage_task");
                if all_manage_task {
                    consecutive_meta_only_turns += 1;
                } else {
                    consecutive_meta_only_turns = 0;
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

                        let mut pending_reflections = Vec::new();
                        for handle in handles {
                            if let Ok(receipt) = handle.await {
                                eprintln!(
                                    "[Brain B] Tool {} (parallel) finished with {:?} in {}ms",
                                    receipt.name, receipt.status, receipt.duration_ms
                                );
                                cognition.observe_tool(receipt.status, receipt.risk);
                                let is_succeeded = receipt.status == crate::services::brain::tool_execution_gateway::ToolExecutionStatus::Succeeded;
                                let output_str = serde_json::to_string(&receipt.result).unwrap_or_default();
                                cognition.observe_tool_execution_details(&receipt.name, None, is_succeeded, &output_str);
                                let is_succeeded = receipt.status == crate::services::brain::tool_execution_gateway::ToolExecutionStatus::Succeeded;
                                let stuck_state = self.stuck_detector.record_call(&receipt.name, "{}", is_succeeded);
                                if let crate::domains::agent_cognitive::stuck_detector::StuckState::StuckDetected { reason, suggested_action } = stuck_state {
                                    let reflection_prompt = crate::domains::agent_cognitive::plan_reflection::ReflectionEngine::build_stuck_reflection_prompt(
                                        &reason.to_string(),
                                        &suggested_action,
                                    );
                                    eprintln!("\x1b[31;1m[Reflexion Guard] ⚠️ 触发卡死防护并注入反思指令: {}\x1b[0m", reason);
                                    pending_reflections.push(reflection_prompt);
                                }

                                self.push_event(SessionEvent::ToolResult {
                                    id: receipt.call_id,
                                    result: receipt.result,
                                });
                            }
                        }
                        for r in pending_reflections {
                            self.push_event(SessionEvent::SystemWarning(r));
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

                            // 防改坏差分作用域审计 (Diff Scope Guard)
                            if crate::domains::agent_cognitive::verification_gate::VerificationGateGuard::is_mutation_tool(&item.func_name) {
                                let target_path_opt = item.args.get("path")
                                    .or_else(|| item.args.get("file_path"))
                                    .or_else(|| item.args.get("target_path"))
                                    .or_else(|| item.args.get("file"))
                                    .and_then(|v| v.as_str());

                                if let Some(target_path_str) = target_path_opt {
                                    match rollback_guard.audit_mutation_path(target_path_str) {
                                        AuditVerdict::Violated { rule_description, suggested_action, .. } => {
                                            let block_msg = format!("【架构铁律越界硬拦截】: {}\n【必须遵守的行动指引】: {}", rule_description, suggested_action);
                                            eprintln!("\x1b[31;1m[RegressionGuard Block] 🛡️ {}\x1b[0m", block_msg);
                                            self.push_event(SessionEvent::ToolResult {
                                                id: item.call_id,
                                                result: json!({ "error": block_msg }),
                                            });
                                            self.save_session();
                                            continue;
                                        }
                                        AuditVerdict::Allowed => {
                                            rollback_guard.observe_mutation(target_path_str);
                                        }
                                    }
                                }
                            }

                            let awaits_user = item.func_name == "ask_user";
                            if awaits_user {
                                self.transition_session(
                                    SessionStatus::NeedsInput,
                                    Some("waiting for correlated Interaction Request".to_string()),
                                )?;
                            }

                            let args_str = serde_json::to_string(&item.args).unwrap_or_default();
                            let cmd_arg = item.args.get("command").or_else(|| item.args.get("cmd")).and_then(|v| v.as_str()).map(|s| s.to_string());
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
                            let is_succeeded = receipt.status == crate::services::brain::tool_execution_gateway::ToolExecutionStatus::Succeeded;
                            let output_str = serde_json::to_string(&receipt.result).unwrap_or_default();
                            cognition.observe_tool_execution_details(&receipt.name, cmd_arg, is_succeeded, &output_str);
                            let tool_result = receipt.result;

                            self.push_event(SessionEvent::ToolResult {
                                id: item.call_id,
                                result: tool_result,
                            });

                            let is_succeeded = receipt.status == crate::services::brain::tool_execution_gateway::ToolExecutionStatus::Succeeded;
                            let stuck_state = self.stuck_detector.record_call(&item.func_name, &args_str, is_succeeded);
                            if let crate::domains::agent_cognitive::stuck_detector::StuckState::StuckDetected { reason, suggested_action } = stuck_state {
                                let reflection_prompt = crate::domains::agent_cognitive::plan_reflection::ReflectionEngine::build_stuck_reflection_prompt(
                                    &reason.to_string(),
                                    &suggested_action,
                                );
                                eprintln!("\x1b[31;1m[Reflexion Guard] ⚠️ 触发卡死防护并注入反思指令: {}\x1b[0m", reason);
                                self.push_event(SessionEvent::SystemWarning(reflection_prompt));
                            }

                            // 认知熔断器防卡死突围逻辑 (Meta-Cognitive Circuit Breaker)
                            if is_succeeded {
                                circuit_breaker.record_success();
                            } else if let Some(directive) = circuit_breaker.record_failure(&item.func_name, &output_str) {
                                eprintln!("\x1b[31;1m[CircuitBreaker Alert] ⚡ {}\x1b[0m", directive.trip_reason);
                                self.push_event(SessionEvent::SystemWarning(format!(
                                    "{}\n\n{}\n\n【突围破局建议】: {}",
                                    directive.trip_reason,
                                    directive.aha_epiphany_prompt,
                                    directive.suggested_isolated_probe
                                )));

                                // 熔断器红牌强制自动回滚脏代码，跳出死胡同！
                                if directive.should_rollback_workspace && !rollback_guard.is_rolled_back && !rollback_guard.mutated_files.is_empty() {
                                    if let Ok(rc) = rollback_guard.rollback_workspace("认知熔断器红牌触发：相同错误连续死循环，强制自动秒级还原工作区") {
                                        let rb_msg = format!(
                                            "【熔断沙箱自动回滚】: 检测到同一错误模式连续失败 {} 次，为防止产生沉没成本已自动秒级还原 {} 个脏文件，清理 {} 个临时新增文件。请彻底更换解题思路！",
                                            directive.consecutive_failures, rc.reverted_files.len(), rc.cleaned_untracked_files.len()
                                        );
                                        eprintln!("\x1b[33;1m[CircuitBreaker] 🛡️ {}\x1b[0m", rb_msg);
                                        self.push_event(SessionEvent::SystemWarning(rb_msg));
                                    }
                                }
                            }

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

                if all_manage_task {
                    let todo_all_done = std::fs::read_to_string(&todo_path)
                        .map(|c| {
                            let has_unfinished = c.lines().any(|l| {
                                let t = l.trim();
                                t.starts_with("- [ ]")
                                    || t.starts_with("* [ ]")
                                    || t.contains("[in_progress]")
                                    || t.contains("[pending]")
                            });
                            !has_unfinished
                        })
                        .unwrap_or(true);

                    if todo_all_done {
                        if let Some(content) = llm_message.get("content").and_then(|v| v.as_str()) {
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                eprintln!("\x1b[32;1m[Terminal Guard] All tasks verified & checklist settled. Returning final delivery report.\x1b[0m");
                                self.events.push(SessionEvent::ModelThought(trimmed.to_string()));
                                self.save_session();
                                if cognition.verification_gate.rejection_count > cognition.verification_gate.max_retries && !rollback_guard.is_rolled_back && !rollback_guard.mutated_files.is_empty() {
                    if let Ok(receipt) = rollback_guard.rollback_workspace("智能体自检超限退化，触发秒级物理回滚以保护工作区代码纯净") {
                        let alert = format!(
                            "【防改坏自动物理回滚已生效】: 智能体多次自愈未通过测试验证。为杜绝在代码库中残留半成品脏代码，工作区已秒级恢复至入场前状态（已还原文件: {:?}，已清理新增垃圾文件: {:?}）。",
                            receipt.reverted_files, receipt.cleaned_untracked_files
                        );
                        eprintln!("\x1b[33;1m[RegressionGuard] 🛡️ {}\x1b[0m", alert);
                        self.push_event(SessionEvent::SystemWarning(alert));
                    }
                }
                self.record_evolution(&mut cognition, prompt, true);
                                return Ok(trimmed.to_string());
                            }
                        }

                        // Content was empty in this tool call, but checklist is fully settled! Pull previous substantial delivery report.
                        if let Some(recent_thought) = self.events.iter().rev().find_map(|e| match e {
                            SessionEvent::ModelThought(t) if t.trim().chars().count() > 50 => Some(t.clone()),
                            _ => None,
                        }) {
                            eprintln!("\x1b[32;1m[Terminal Guard] Checklist fully settled. Returning previous comprehensive report.\x1b[0m");
                            self.save_session();
                            if cognition.verification_gate.rejection_count > cognition.verification_gate.max_retries && !rollback_guard.is_rolled_back && !rollback_guard.mutated_files.is_empty() {
                    if let Ok(receipt) = rollback_guard.rollback_workspace("智能体自检超限退化，触发秒级物理回滚以保护工作区代码纯净") {
                        let alert = format!(
                            "【防改坏自动物理回滚已生效】: 智能体多次自愈未通过测试验证。为杜绝在代码库中残留半成品脏代码，工作区已秒级恢复至入场前状态（已还原文件: {:?}，已清理新增垃圾文件: {:?}）。",
                            receipt.reverted_files, receipt.cleaned_untracked_files
                        );
                        eprintln!("\x1b[33;1m[RegressionGuard] 🛡️ {}\x1b[0m", alert);
                        self.push_event(SessionEvent::SystemWarning(alert));
                    }
                }
                self.record_evolution(&mut cognition, prompt, true);
                            return Ok(recent_thought);
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
                if cognition.verification_gate.rejection_count > cognition.verification_gate.max_retries && !rollback_guard.is_rolled_back && !rollback_guard.mutated_files.is_empty() {
                    if let Ok(receipt) = rollback_guard.rollback_workspace("智能体自检超限退化，触发秒级物理回滚以保护工作区代码纯净") {
                        let alert = format!(
                            "【防改坏自动物理回滚已生效】: 智能体多次自愈未通过测试验证。为杜绝在代码库中残留半成品脏代码，工作区已秒级恢复至入场前状态（已还原文件: {:?}，已清理新增垃圾文件: {:?}）。",
                            receipt.reverted_files, receipt.cleaned_untracked_files
                        );
                        eprintln!("\x1b[33;1m[RegressionGuard] 🛡️ {}\x1b[0m", alert);
                        self.push_event(SessionEvent::SystemWarning(alert));
                    }
                }
                self.record_evolution(&mut cognition, prompt, true);
                let _ = crate::domains::agent_cognitive::plan_reflection::ReflectionEngine::distill_and_persist(
                    &self.events,
                    &self.runtime_config,
                    Some(self.run_prelude_runtime.project_path()),
                );
                return Ok(content.to_string());
            }

            if !rollback_guard.is_rolled_back && !rollback_guard.mutated_files.is_empty() {
                let _ = rollback_guard.rollback_workspace("智能体异常中断，自动还原工作区以保护代码纯净");
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

            // 核心精锐工程工具白名单（对齐 Claude Code / Manus 黄金工程工具集）
            const ALLOWED_CORE_TOOLS: &[&str] = &[
                "view_file",
                "replace_in_file",
                "write_to_file",
                "run_command",
                "grep_search",
                "find_by_name",
                "list_dir",
                "manage_task",
                "web_fetch",
                "read_tool_result",
                "ask_user",
                "invoke_subagent",
                "view_outline",
            ];

            // 过滤：仅暴露核心工程工具及带 prompt_projection 的动态 Skill 工具，杜绝 38 个工具造成的选择瘫痪
            let filtered_tools: Vec<_> = tools_guard
                .values()
                .filter(|t| ALLOWED_CORE_TOOLS.contains(&t.name()) || t.prompt_projection().is_some())
                .cloned()
                .collect();

            let mut sorted_tools = filtered_tools;
            sorted_tools.sort_by_key(|t| {
                ALLOWED_CORE_TOOLS
                    .iter()
                    .position(|&p| p == t.name())
                    .unwrap_or(usize::MAX)
            });

            let schema = sorted_tools
                .iter()
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

