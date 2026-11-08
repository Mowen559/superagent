use crate::domains::agent_cognitive::intent::{ActionGoal, CurrentRequestStage, DynamicIntent, DynamicIntentEngine};
pub use crate::domains::model_gateway::model_attempt_router::BrainTier;
use crate::services::brain::evolution_memory::{DistilledLesson, EvolutionReport};
use crate::domains::agent_cognitive::verification_gate::{GateDecision, VerificationGateGuard, VerificationPolicy};
use crate::services::brain::run_plan::RunPlan;
pub use crate::services::brain::run_plan::TaskKind;
use crate::services::brain::task_progress::{SharedTaskProgress, TaskProgress};
use crate::services::brain::task_understanding::TaskContract;
use crate::services::brain::tool_execution_gateway::{ToolExecutionReceipt, ToolExecutionStatus};
use crate::services::brain::tool_policy::ToolRisk;

const DEFAULT_MAX_MODEL_TURNS: usize = 40;
const MIN_COMPLEX_DRAFT_CHARS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningMode {
    Fast,
    Adaptive,
    Deliberate,
}

impl ReasoningMode {
    fn from_env() -> Self {
        match std::env::var("HUB_REASONING_MODE")
            .unwrap_or_else(|_| "adaptive".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "fast" => Self::Fast,
            "deliberate" | "deep" => Self::Deliberate,
            _ => Self::Adaptive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionDecision {
    Accept,
    Revise { reason: String },
    Blocked { reason: String },
}

/// Run-scoped reasoning control plane. It turns raw tool receipts into compact
/// progress guidance and enforces Claim-Action integrity & proactive inquiry.
#[derive(Debug)]
pub struct CognitiveRuntime {
    objective: String,
    mode: ReasoningMode,
    task_kind: TaskKind,
    intent: DynamicIntent,
    evidence_required: bool,
    model_turns: usize,
    tool_calls: usize,
    mutation_tool_calls: usize,
    inquiry_tool_calls: usize,
    successful_tools: usize,
    project_context_observations: usize,
    failed_tools: usize,
    consecutive_failures: usize,
    observations_since_progress: usize,
    completion_review_used: bool,
    max_model_turns: usize,
    plan: RunPlan,
    learned_guidance: Vec<String>,
    anchor_constraints: Vec<String>,
    distilled_lessons: Vec<DistilledLesson>,
    task_progress: SharedTaskProgress,
    proposal_only: bool,
    memory_only_scope: bool,
    memory_operation_verified: bool,
    recovery_hint: Option<String>,
    pub socratic_payload: Option<crate::domains::agent_cognitive::socratic_inquiry::SocraticInquiryPayload>,
    pub verification_gate: VerificationGateGuard,
}

impl CognitiveRuntime {
    pub fn begin(objective: &str) -> Self {
        Self::with_mode(objective, ReasoningMode::from_env())
    }

    pub fn with_mode(objective: &str, mode: ReasoningMode) -> Self {
        let mut intent = DynamicIntentEngine::analyze(objective);
        let normalized = objective.to_lowercase();
        // Conservative explicit native-note routing. Mixed code requests retain
        // their normal implementation gates; note receipts never certify code.
        let note_prefix=normalized.trim_start().trim_start_matches("请").trim_start();
        let note_prefix=note_prefix.strip_prefix("使用 ").or_else(||note_prefix.strip_prefix("使用")).or_else(||note_prefix.strip_prefix("please use ")).or_else(||note_prefix.strip_prefix("use ")).unwrap_or(note_prefix).trim_start();
        let memory_only_scope=note_prefix.starts_with("super_memory")
            && contains_any(&normalized,&["保存","记住","记录","撤销","store","revoke"])
            && !matches!(intent.goal,ActionGoal::SurgicalExecution|ActionGoal::RefactoringModernization)
            && (intent.goal!=ActionGoal::VerificationBenchmark || contains_any(&normalized,&["不用运行代码测试","不需要运行测试","无需代码测试","no code tests"]));
        let stage = CurrentRequestStage::classify(objective);
        let proposal_only = stage == CurrentRequestStage::ProposalOnly;
        let investigation_only = stage == CurrentRequestStage::InvestigationOnly;
        let proposal_needs_workspace = contains_any(
            &normalized,
            &[
                "当前",
                "文件",
                "代码",
                "报错",
                "现有",
                "仓库",
                "readme",
                "context.md",
                "agents.md",
                "src/",
                "src\\",
                "workspace",
                "existing",
                "repository",
                "current project",
            ],
        );
        let mut task_kind = match intent.goal {
            ActionGoal::ArchitectureExploration | ActionGoal::TargetedInvestigation => {
                TaskKind::Investigative
            }
            ActionGoal::SurgicalExecution
            | ActionGoal::VerificationBenchmark
            | ActionGoal::RefactoringModernization => TaskKind::Execution,
            ActionGoal::ClarificationDialogue => TaskKind::Conversational,
        };
        if proposal_only {
            task_kind = if proposal_needs_workspace {
                TaskKind::Investigative
            } else {
                TaskKind::Conversational
            };
            intent.capability.suggested_mutation_tools.clear();
            intent.capability.suggested_verification_tools.clear();
        }
        if investigation_only {
            task_kind=TaskKind::Investigative;
            intent.capability.suggested_mutation_tools.clear();
            intent.capability.suggested_verification_tools.clear();
        }
        if memory_only_scope {
            task_kind=TaskKind::Investigative;
            intent.capability.suggested_inspection_tools=vec!["super_memory".into()];
            intent.capability.suggested_mutation_tools=vec!["super_memory".into()];
            intent.capability.suggested_verification_tools.clear();
        }

        let evidence_required = if proposal_only {
            proposal_needs_workspace
        } else {
            matches!(task_kind, TaskKind::Execution | TaskKind::Investigative)
                || contains_any(
                    &objective.to_lowercase(),
                    &[
                        "当前", "项目", "源码", "代码", "文件", "仓库", "日志", "错误", "current",
                        "project", "source", "code", "file", "repo", "log", "error",
                    ],
                )
        };

        let mut default_anchors = vec![
            "Project Conventions: Inspect workspace instructions and existing code before choosing frameworks, interfaces, or architecture. Do not assume a language or service layout.".to_string(),
            "Surgical Edits: Use replace_in_file/multi_replace; preserve unedited comments, docstrings, and formatting.".to_string(),
            "Verification Closed-Loop: Run compile/test checks before claiming completion; never fabricate tool output.".to_string(),
        ];
        if proposal_only {
            default_anchors.push("Proposal-only scope: inspect only as needed and deliver the requested plan. Proposed future implementation steps are not authorized actions in this Run; do not execute or claim them completed.".to_string());
        }
        if investigation_only {
            default_anchors.push("Investigation-only scope: gather evidence and report findings. Do not change business source code or create an implementation step just to satisfy a completion gate; code mutation and implementation tests are not required for this read-only objective.".into());
        }
        if memory_only_scope {
            default_anchors.retain(|anchor|!anchor.starts_with("Verification Closed-Loop"));
            default_anchors.push("Native project-note scope: only the requested super_memory mutation is authorized. Its runtime readback verifies the storage operation, never the truth of the note or code quality. Do not run code tests or edit project files for this task.".into());
        }

        for path in &intent.constraints.protected_paths {
            default_anchors.push(format!("【IRONCLAD INVARIANT】Path '{}' is strictly protected from mutation under user mandate!", path));
        }
        for pattern in &intent.constraints.forbidden_patterns {
            default_anchors.push(format!(
                "【IRONCLAD INVARIANT】Forbidden pattern: {}",
                pattern
            ));
        }

        let plan = if !proposal_only && !memory_only_scope && intent.goal == ActionGoal::VerificationBenchmark {
            RunPlan::for_verification()
        } else {
            RunPlan::for_task(task_kind)
        };
        let mut progress=TaskProgress::default();
        progress.set_read_only_scope(proposal_only || investigation_only || intent.constraints.read_only_mode);
        progress.set_memory_only_scope(memory_only_scope);
        Self {
            objective: objective.chars().take(500).collect(),
            mode,
            task_kind,
            intent,
            evidence_required,
            model_turns: 0,
            tool_calls: 0,
            mutation_tool_calls: 0,
            inquiry_tool_calls: 0,
            successful_tools: 0,
            project_context_observations:0,
            failed_tools: 0,
            consecutive_failures: 0,
            observations_since_progress: 0,
            completion_review_used: false,
            max_model_turns: DEFAULT_MAX_MODEL_TURNS,
            plan,
            learned_guidance: Vec::new(),
            anchor_constraints: default_anchors,
            distilled_lessons: Vec::new(),
            task_progress: std::sync::Arc::new(std::sync::Mutex::new(progress)),
            proposal_only,
            memory_only_scope,
            memory_operation_verified:false,
            recovery_hint:None,
            socratic_payload: crate::domains::agent_cognitive::socratic_inquiry::SocraticInquiryEngine::evaluate(objective, None),
            verification_gate: VerificationGateGuard::new(
                if task_kind == TaskKind::Conversational {
                    VerificationPolicy::Permissive
                } else {
                    VerificationPolicy::Strict
                },
                3,
            ),
        }
    }

    pub fn task_kind(&self) -> TaskKind {
        self.task_kind
    }

    /// Hosts may impose a smaller deadline, but cannot expand an existing one.
    pub fn limit_model_turns(&mut self, max_turns: usize) {
        self.max_model_turns = self.max_model_turns.min(max_turns.max(1));
    }

    pub fn task_progress(&self) -> SharedTaskProgress {
        self.task_progress.clone()
    }

    fn promote_execution(&mut self, require_action: bool) {
        if self.memory_only_scope { return; }
        if self.proposal_only || self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).is_read_only_scope() { return; }
        self.task_kind=TaskKind::Execution;
        self.evidence_required=true;
        self.verification_gate.policy=VerificationPolicy::Strict;
        self.plan.promote_execution(require_action);
        if require_action && self.intent.capability.suggested_mutation_tools.is_empty() {
            self.intent.capability.suggested_mutation_tools=vec!["write_to_file".into(),"replace_in_file".into()];
        }
    }

    fn follow_structured_plan(&mut self) {
        let execution=self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).planned_execution();
        if let Some(require_action)=execution { self.promote_execution(require_action); }
    }
    pub fn note_requirement_update(&mut self) {
        self.project_context_observations=0;
        self.memory_operation_verified=false;
        self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).note_requirement_update();
        self.plan.invalidate_verification();
        self.verification_gate.invalidate_verification();
        self.completion_review_used=false;
    }

    pub fn invalidate_after_rollback(&mut self) {
        self.project_context_observations=0;
        self.memory_operation_verified=false;
        self.task_progress.lock().unwrap_or_else(|e| e.into_inner()).invalidate_after_rollback();
        self.plan.reset_evidence();
        self.recovery_hint=None;
        self.verification_gate = VerificationGateGuard::new(self.verification_gate.policy, self.verification_gate.max_retries);
    }

    pub fn consecutive_failures(&self) -> usize {
        self.consecutive_failures
    }
    /// Only the executor supplies the count of complete runtime-validated note
    /// blocks already present in the request. Historical data is not a test.
    pub fn observe_project_context(&mut self,count:usize) {
        if count==0 {return;}
        self.project_context_observations=count;
        self.plan.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::ReadOnly,false);
    }

    pub fn select_brain_tier(&self) -> BrainTier {
        if let Ok(level) = std::env::var("HUB_THINKING_LEVEL") {
            let l = level.trim().to_ascii_lowercase();
            if l == "low" || l == "off" || l == "fast" {
                return BrainTier::Fast;
            } else if l == "high" || l == "deep" || l == "max" {
                return BrainTier::Slow;
            }
        }
        crate::domains::model_gateway::model_attempt_router::DualBrainRouter::select_tier(
            self.mode,
            self.task_kind,
            self.tool_calls,
            self.failed_tools,
            self.consecutive_failures,
        )
    }

    pub fn apply_promoted_guidance(&mut self, guidance: Vec<String>) {
        self.learned_guidance = guidance.into_iter().take(5).collect();
    }

    pub fn apply_distilled_lessons(&mut self, lessons: Vec<DistilledLesson>) {
        self.distilled_lessons = lessons.into_iter().take(5).collect();
    }

    pub fn anchor_constraints(&self) -> &[String] {
        &self.anchor_constraints
    }

    pub fn add_anchor_constraint(&mut self, constraint: impl Into<String>) {
        self.anchor_constraints.push(constraint.into());
    }

    pub fn next_guidance(&mut self) -> Result<String, String> {
        self.model_turns += 1;
        if self.model_turns > self.max_model_turns {
            return Err(format!(
                "cognitive turn budget exceeded: {} model turns",
                self.max_model_turns
            ));
        }

        let physical_acceptance = self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).uses_host_physical_acceptance();
        let closeout_strategy = self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).managed_closeout_strategy();
        let strategy = if self.memory_only_scope {
            "Perform only the requested super_memory store/revoke operation. Its operation_verified receipt is runtime readback of that note, not code verification. Report the actual result and candidate/unverified scope; no unrelated files or tests are needed."
        } else if let Some(strategy) = closeout_strategy.as_deref() {
            strategy
        } else if physical_acceptance {
            "Read only the evidence needed for the requested files, make the coherent edit, then call verify_delivery. The host requests physical artifacts; runtime dependency probes and code tests are unnecessary unless essential to create those artifacts or explicitly requested. Reuse supplied facts, preserve files outside scope, and do not recalculate the same hashes to advance progress. Historical document claims are not proof of past checks: attribute them to their source or state they were not independently revalidated. Do not invent test samples, AST certification, or runtime checks in a documentation-only Run."
        } else { match self.task_kind {
            TaskKind::Conversational => {
                "Answer directly. Use tools only when the answer depends on workspace or live evidence."
            }
            TaskKind::Investigative => {
                "Form dynamic hypotheses, inspect authoritative evidence first, distinguish facts from inference, and pivot rapidly if assumptions fail."
            }
            TaskKind::Execution => {
                "Inspect the relevant state, make the smallest coherent change, run a quality check, and use actual failures to revise the next action."
            }
        }};

        let failure_note = if self.consecutive_failures > 0 {
            format!(
                "\n[META-COGNITIVE REFLECTION / AHA-MOMENT REQUIRED]\nWait, a previous tool attempt failed! Do not blindly repeat the same command or edit. Explicitly reflect: Why did it fail? Was the path incorrect, did the file content change, or was the assumption wrong? Adjust your hypothesis and take a different approach."
            )
        } else if self.failed_tools > 0 {
            "\n[AWARENESS] Earlier tools encountered failures. Ensure current steps address residual risks and verify outcomes carefully.".to_string()
        } else {
            String::new()
        };

        let deadlock_warning = if self.consecutive_failures >= 3 {
            "\n[HYPOTHESIS DEADLOCK WARNING] Three or more consecutive tool actions failed. Inspect the actual error and current state with read-only tools, then change the hypothesis, arguments, or tool. Preserve existing user changes; do not roll back unrelated work."
        } else {
            ""
        };

        let pivot_mandate = if self.consecutive_failures >= 2 {
            "\n[COGNITIVE PIVOT MANDATE - 连续受阻强制变轨]\n⚠️ You have encountered 2 or more consecutive tool failures! Continuing down this identical line of thought is STRICTLY FORBIDDEN.\nExecute one of these 3 pivot actions immediately:\n1. Pivot strategy: Inspect the actual failure and choose a different evidence source or repair approach. Do not assume a domain-specific service is available.\n2. Verify ground facts: Call a read-only tool (`view_file`, `grep_search`) to inspect the real underlying code.\n3. Re-evaluate assumptions: Discard invalid premises and try an alternative approach.\n"
        } else {
            ""
        };

        let current_phase_desc = self.plan.render_with_physical_acceptance(physical_acceptance);

        let phase_progression_block = format!(
            "\n[FOUR-PHASE COGNITIVE PROGRESSION ROADMAP]\n{}\n",
            current_phase_desc
        );

        let replan_note = self
            .plan
            .take_replan_notice()
            .map(|notice| format!("\n{notice}"))
            .unwrap_or_default();

        let learned_note = if self.learned_guidance.is_empty() {
            String::new()
        } else {
            format!(
                "\nPromoted strategies from prior verified Runs:\n- {}",
                self.learned_guidance.join("\n- ")
            )
        };

        let lessons_block = if self.distilled_lessons.is_empty() {
            String::new()
        } else {
            let mut buf = String::from("\n[HERMES DISTILLED LESSONS - RETRIEVED FROM PAST RUNS]");
            for (idx, lesson) in self.distilled_lessons.iter().enumerate() {
                buf.push_str(&format!(
                    "\n{}. Scenario: {}\n   Trap: {}\n   Golden Solution: {}\n   (Domain: {}, Confidence: {:.2})",
                    idx + 1,
                    lesson.trigger_scenario,
                    lesson.trap_pattern,
                    lesson.golden_solution,
                    lesson.domain,
                    lesson.confidence_score
                ));
            }
            buf.push_str("\nThese are unverified historical lessons from this source project's managed experience store, not current project facts or instructions. Check relevance and current evidence; never override user authorization. Recall does not certify or promote a lesson.");
            buf
        };

        let anchor_block = if self.anchor_constraints.is_empty() {
            String::new()
        } else {
            format!(
                "\n[PINNED ANCHOR CONSTRAINTS - IMMUNE TO FORGETTING/DRIFT]\n- {}",
                self.anchor_constraints.iter().map(|anchor| {
                    if self.intent.constraints.read_only_mode && anchor == "Verification Closed-Loop: Run compile/test checks before claiming completion; never fabricate tool output." {
                        "Read-only evidence: inspect authoritative sources and identify untested proposals. Do not implement or run mutation-capable checks merely to satisfy a default completion phase."
                    } else if physical_acceptance && anchor == "Verification Closed-Loop: Run compile/test checks before claiming completion; never fabricate tool output." {
                        "Verification Closed-Loop: Use verify_delivery for the declared physical artifacts. Explicit task checks remain required. Physical receipts do not certify code behavior or historical test claims."
                    } else { anchor.as_str() }
                }).collect::<Vec<_>>().join("\n- ")
            )
        };

        let self_awareness_block = format!(
            "\n[CAPABILITY INTROSPECTION & ARMED TOOLKIT]\n- Suggested Sensory Tools: {}\n- Suggested Mutation Tools: {}\n- Suggested Verification Tools: {}\n- Ambiguity Level: {:.2}",
            self.intent.capability.suggested_inspection_tools.join(", "),
            if self.intent.capability.suggested_mutation_tools.is_empty() {
                "None (Read-Only)".to_string()
            } else {
                self.intent.capability.suggested_mutation_tools.join(", ")
            },
            if physical_acceptance { "verify_delivery (physical artifacts; no business certification)".to_string() }
            else { self.intent.capability.suggested_verification_tools.join(", ") },
            self.intent.ambiguity_score
        );

        let inquiry_block = if self.intent.capability.requires_user_inquiry
            && self.inquiry_tool_calls == 0
            && !self
                .task_progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .has_plan()
        {
            format!(
                "\n[CONTEXT-FIRST REQUIREMENT CLARIFICATION]\nThe request may have unresolved choices. First inspect supplied workspace context and relevant code, then use plan_task to distinguish known constraints from context gaps. Use tools to resolve factual gaps. Choose reasonable defaults for reversible details; ask the user only when a remaining choice materially changes scope or behavior.\nInvestigation angles:\n- {}\n{}",
                self.intent.capability.recommended_inquiries.join("\n- "),
                "Use facts to resolve uncertainty before asking about material choices."
            )
        } else if self.task_kind != TaskKind::Conversational && !self.intent.constraints.read_only_mode {
            "\n[ACTION-FIRST DIRECTIVE]\nAdopt Sensible Defaults. Deliver working code, verified rules, or direct technical solutions. Do NOT interrogate the user with unnecessary questionnaires unless a destructive action requires explicit authorization.".to_string()
        } else {
            String::new()
        };

        let understanding = self.task_progress.lock().unwrap_or_else(|e| e.into_inner()).render()
            .unwrap_or_else(|| {
                "\n[Task Understanding]\nFor multiple deliverables, dependencies, or material ambiguity: use supplied context and targeted inspection, then call `plan_task` with the intended outcome, success criteria, concise dependency-ordered subgoals, constraints and remaining gaps. Group work by independently verifiable outcome; do not expand speculative API signatures or duplicate checks. If inspection cannot obtain a fact, record a context gap instead of repeatedly reading unavailable sources. Distinguish known requirements from proposed design. Ask only about material unresolved choices. For a simple request, proceed directly. A plan is a hypothesis, never execution evidence.\n".to_string()
            });

        let mut res = format!(
            "# Run Decision State\nObjective: {}\nMode: {:?}; Task Kind: {:?}; Model Turn: {}; Tool Stats: {} succeeded / {} failed (Mutations: {}).\nDynamic Plan:\n{}\nReasoning Guide: {}{}{}{}{}{}{}{}{}{}\nAdvance the in-progress step using live evidence. Do not claim completion without verification.",
            self.objective,
            self.mode,
            self.task_kind,
            self.model_turns,
            self.successful_tools,
            self.failed_tools,
            self.mutation_tool_calls,
            current_phase_desc,
            strategy,
            failure_note,
            deadlock_warning,
            replan_note,
            learned_note,
            lessons_block,
            anchor_block,
            self_awareness_block,
            inquiry_block,
            understanding,
        );
        res.push_str(&format!("{}{}", phase_progression_block, pivot_mandate));
        if self.project_context_observations>0 {
            res.push_str(&format!("\n[Provided project-note history]\n{} complete scoped historical evidence blocks are already supplied by the Run Prelude. They support a sourced read-only answer, not code changes, quality verification, or promotion to truth. Reuse them without calling tools solely to satisfy completion; inspect fresh sources when the requested implementation or current state requires it. These are context observations, not successful tool calls.\n",self.project_context_observations));
        }
        if self.task_kind == TaskKind::Execution && self.observations_since_progress >= 8 {
            res.push_str(&format!("\n[Inspection plateau]\n{} successful observations since the last actual edit, certified check, or user answer. Reuse evidence already obtained. State the remaining hypothesis and choose a check that can distinguish it, make a justified scoped edit, or ask for genuinely missing user information. Do not repeatedly print the same facts. This is an advisory: it does not require an unsafe edit or relax verification, scope, or permissions.\n",self.observations_since_progress));
        }
        res.push_str(&self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).delivery_guidance());
        res.push_str(&self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).facts_guidance());
        if self.plan.ready_for_response() && self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).completion_gap().is_none() {
            res.push_str("\n[Execution stages satisfied]\nThe default execution stages and explicit contract have no remaining gap. If the requested outcome is covered, deliver the evidence-backed final answer now. Do not invent another plan or delivery declaration merely to call a tool. If a real user requirement remains, address only that gap.\n");
        }
        if let Some(hint)=&self.recovery_hint {res.push_str(&format!("\n[Evidence-based recovery]\n{hint}\n"));}
        res.push_str(&format!(
            "\n[Turn Budget]\nRemaining model turns including this request: {} of {}. Batch independent read-only calls when useful and reuse facts already supplied in workspace excerpts or prior results. Read further only to resolve a material gap; once the requested outcome is supported, advance to the plan, action or delivery. Record unresolved facts honestly instead of exhausting the budget browsing unrelated files. The deadline never relaxes verification or authorization requirements.\n",
            self.max_model_turns.saturating_sub(self.model_turns) + 1,
            self.max_model_turns,
        ));
        Ok(res)
    }

    pub fn observe_receipt(&mut self, receipt: &ToolExecutionReceipt) {
        if let Some(state)=receipt.result.get("recovery_state") {
            self.recovery_hint=state.get("hint").and_then(serde_json::Value::as_str).map(str::to_string);
        }
        if let Some(evidence)=receipt.result.get("quality_evidence") {
            if let Some(hint)=evidence.get("recovery_hint").and_then(serde_json::Value::as_str) {self.recovery_hint=Some(hint.into());}
            else if evidence["eligible"]==true && self.recovery_hint.as_ref().is_some_and(|h|h.starts_with("[Quality evidence]")) {self.recovery_hint=None;}
        }
        self.task_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe(receipt);
        if receipt.name == "plan_task" && receipt.status == ToolExecutionStatus::Succeeded {
            if let Some(value) = receipt.result.get("task_contract") {
                if let Ok(contract) = TaskContract::from_value(value.clone()) {
                    let mut progress = self.task_progress.lock().unwrap_or_else(|e| e.into_inner());
                    let had_plan = progress.has_plan();
                    let changed = progress.apply_plan(contract);
                    if had_plan && changed { self.plan.invalidate_verification(); }
                    drop(progress);
                    self.follow_structured_plan();
                }
            }
            // Planning is useful context, but not an observation of the world.
            return;
        }
        if receipt.name == "manage_task" && receipt.status == ToolExecutionStatus::Succeeded {
            return;
        }
        if receipt.name == "revise_task_plan" && receipt.status == ToolExecutionStatus::Succeeded {
            self.follow_structured_plan();
            if receipt
                .result
                .get("requires_verification")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                self.plan.invalidate_verification();
            }
            return;
        }
        if receipt.name == "update_task_progress"
            && receipt.status == ToolExecutionStatus::Succeeded
        {
            return;
        }
        if receipt.name == "ask_user" && receipt.status == ToolExecutionStatus::Succeeded {
            self.inquiry_tool_calls += 1;
        }
        if receipt.name=="super_memory" {
            if receipt.risk==ToolRisk::WorkspaceMutation {
                self.memory_operation_verified=receipt.status==ToolExecutionStatus::Succeeded
                    && receipt.result.get("operation_verified").and_then(serde_json::Value::as_bool)==Some(true);
            }
            // Permission risk still gates the actual mutation. A project note
            // is not a source edit or a successful compile/test receipt.
            self.observe_tool(receipt.status,ToolRisk::ReadOnly,false);
            return;
        }
        // A conservative permission risk can invalidate old verification, but
        // process success alone cannot prove a physical file change.
        if receipt.is_process_observation() && !receipt.verification
            && matches!(receipt.risk, ToolRisk::WorkspaceMutation | ToolRisk::Destructive) {
            self.plan.invalidate_verification();
        }
        let evidence_risk = if receipt.name=="perform_delivery_action" && receipt.status==ToolExecutionStatus::Succeeded {
            ToolRisk::WorkspaceMutation
        } else if receipt.is_process_observation() {
            ToolRisk::ReadOnly
        } else { receipt.risk };
        self.observe_tool(receipt.status, evidence_risk, receipt.verification);
        if receipt.name == "ask_user" && receipt.status == ToolExecutionStatus::Succeeded
            && receipt.result.get("response").and_then(serde_json::Value::as_str).is_some_and(|s|!s.trim().is_empty()) {
            self.observations_since_progress = 0;
        }
    }

    fn observe_tool(&mut self, status: ToolExecutionStatus, risk: ToolRisk, verification: bool) {
        if status == ToolExecutionStatus::Succeeded {
            if verification || matches!(risk, ToolRisk::WorkspaceMutation | ToolRisk::Destructive) {
                self.observations_since_progress = 0;
            } else if matches!(risk, ToolRisk::ReadOnly | ToolRisk::Network | ToolRisk::ProcessControl) {
                self.observations_since_progress = self.observations_since_progress.saturating_add(1);
            }
        }
        if status==ToolExecutionStatus::Succeeded && matches!(risk,ToolRisk::WorkspaceMutation|ToolRisk::Destructive) && !verification {
            self.promote_execution(true);
        }
        self.tool_calls += 1;
        if !verification && risk == ToolRisk::WorkspaceMutation && status == ToolExecutionStatus::Succeeded {
            self.mutation_tool_calls += 1;
            self.verification_gate.observe_tool_execution("write_to_file", None, true, "");
        }
        // Synchronize the supplemental DoD gate only from canonical verification
        // facts. An arbitrary successful process is not verification evidence.
        if verification && status == ToolExecutionStatus::Succeeded {
            self.verification_gate.observe_tool_execution("run_command", None, true, "");
        }
        self.plan.observe_tool(status, risk, verification);
        if status == ToolExecutionStatus::Succeeded {
            self.successful_tools += 1;
            self.consecutive_failures = 0;
        } else if matches!(
            status,
            ToolExecutionStatus::Failed
                | ToolExecutionStatus::Panicked
                | ToolExecutionStatus::NotFound
                | ToolExecutionStatus::PolicyDenied
                | ToolExecutionStatus::RepetitionBlocked
        ) {
            self.failed_tools += 1;
            self.consecutive_failures += 1;
        }
    }

    pub fn observe_tool_execution_details(
        &mut self,
        tool_name: &str,
        command_arg: Option<String>,
        success: bool,
        output_snippet: &str,
    ) {
        self.verification_gate
            .observe_tool_execution(tool_name, command_arg, success, output_snippet);
    }

    pub fn observe_executed_tool_details(&mut self, receipt:&ToolExecutionReceipt, command_arg:Option<String>, output:&str) {
        if matches!(receipt.status,ToolExecutionStatus::Succeeded|ToolExecutionStatus::Failed|ToolExecutionStatus::Panicked) {
            self.observe_tool_execution_details(&receipt.name,command_arg,receipt.status==ToolExecutionStatus::Succeeded,output);
        }
    }

    pub fn review_completion(&mut self, draft: &str) -> CompletionDecision {
        let host_physical=self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).host_physical_delivery_complete();
        if host_physical {self.plan.accept_host_physical_delivery();}
        self.plan.begin_completion();
        let normalized = draft.to_lowercase();
        let claims_written = [
            "写入",
            "落盘",
            "创建了",
            "已创建",
            "修改了",
            "已修改",
            "修改完毕",
            "已经更新",
            "代码已更新",
            "created",
            "written",
            "modified",
            "added the test",
        ]
        .iter()
        .any(|kw| normalized.contains(kw));

        let delivery_gate_gap = {self.task_progress.lock().unwrap_or_else(|e|e.into_inner()).delivery_gate_gap()};
        let integrity_gap = delivery_gate_gap.or_else(|| if self.memory_only_scope && !self.memory_operation_verified {
            Some("The requested project-note operation has no successful runtime readback. Use super_memory for the authorized note operation; do not create code or tests to satisfy this scope.".into())
        } else if self.task_kind == TaskKind::Execution
            && claims_written
            && self.mutation_tool_calls == 0
        {
            Some("Claim-Action 违背拦截: No successful WorkspaceMutation supports the claimed changes. Inspect the workspace, perform the authorized change, and verify it.".to_string())
        } else if self.intent.capability.requires_user_inquiry
            && self.tool_calls == 0
            && self.task_kind == TaskKind::Execution
        {
            Some("主动对齐拦截: Inspect the workspace first. Ask the user only about material choices that remain unresolved after inspection.".to_string())
        } else if self.evidence_required && self.successful_tools == 0 && self.project_context_observations==0 {
            Some("No successful tool evidence supports this objective. Inspect authoritative sources and address tool failures before concluding.".to_string())
        } else {
            self.plan.completion_gap().map(|gap| format!(
                "The Run plan is incomplete at: {gap}. Complete it with tool evidence; verification requires a successful quality check after the latest change."
            )).or_else(|| self.task_contract_gap())
        });

        if let Some(reason) = integrity_gap {
            // Review is bounded, but its use must never become a bypass for
            // missing evidence. Failed Runs must not train success strategies.
            if self.completion_review_used || self.mode == ReasoningMode::Fast {
                return CompletionDecision::Blocked { reason };
            }
            self.completion_review_used = true;
            return CompletionDecision::Revise { reason };
        }

        let is_conversational = self.task_kind == TaskKind::Conversational;
        match if host_physical {GateDecision::Passed} else {self.verification_gate.evaluate_conclusion_attempt(is_conversational)} {
            GateDecision::RejectAndSteer {
                reason,
                suggested_action,
                current_attempt,
                max_attempts,
            } => {
                let reason = format!(
                        "【DoD 自动化验证门禁拦截 (第 {}/{} 次)】: {}\n【必须行动】: {}",
                        current_attempt, max_attempts, reason, suggested_action
                    );
                if self.completion_review_used || self.mode == ReasoningMode::Fast {
                    return CompletionDecision::Blocked { reason };
                }
                self.completion_review_used = true;
                return CompletionDecision::Revise { reason };
            }
            GateDecision::MaxRetriesDegraded { attempts, diagnostic } => {
                return CompletionDecision::Blocked {
                    reason: format!("DoD verification exhausted after {attempts} attempts: {diagnostic}"),
                };
            }
            GateDecision::Passed => {}
        }
        if self.completion_review_used || self.mode == ReasoningMode::Fast {
            return CompletionDecision::Accept;
        }
        let short_draft = self.task_kind != TaskKind::Conversational
            && draft.trim().chars().count() < MIN_COMPLEX_DRAFT_CHARS;
        if self.mode == ReasoningMode::Deliberate || short_draft {
            self.completion_review_used = true;
            return CompletionDecision::Revise {
                reason: "Check that the draft resolves the objective, explains the evidence, and covers remaining limitations.".to_string(),
            };
        }
        CompletionDecision::Accept
    }

    pub fn evolution_report(&mut self, completed: bool) -> EvolutionReport {
        self.plan.begin_completion();
        EvolutionReport {
            task_kind: self.task_kind,
            completed,
            plan_closed: self.plan.completion_gap().is_none() && self.task_contract_gap().is_none(),
            replans: self.plan.replan_count(),
        }
    }

    fn task_contract_gap(&self) -> Option<String> {
        if self.proposal_only {
            return None;
        }
        self.task_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completion_gap()
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

#[cfg(test)]
mod tests {
    #[test]
    fn complete_project_context_supports_read_only_answer_without_fabricated_tools() {
        let mut runtime=CognitiveRuntime::with_mode("只读查询：本项目已有约定是什么，说明来源和范围",ReasoningMode::Adaptive);
        runtime.observe_project_context(1);
        assert_eq!(runtime.tool_calls,0);assert_eq!(runtime.successful_tools,0);assert_eq!(runtime.mutation_tool_calls,0);
        assert!(runtime.next_guidance().unwrap().contains("These are context observations, not successful tool calls"));
        assert_eq!(runtime.review_completion("根据运行时提供的项目候选笔记，约定值如下；该笔记尚未经过业务验证。"),CompletionDecision::Accept);
        runtime.note_requirement_update();
        assert_ne!(runtime.review_completion("新要求也已核查完毕，全部无需进一步取证。"),CompletionDecision::Accept);
    }
    #[test]
    fn absent_context_and_project_history_do_not_certify_changes_tests_or_note_writes() {
        let mut empty=CognitiveRuntime::with_mode("只读查询本项目约定",ReasoningMode::Adaptive);
        empty.observe_project_context(0);
        assert_ne!(empty.review_completion("项目全部约定已经完整核实。"),CompletionDecision::Accept);
        for prompt in ["请修复 calc.cjs 并测试", "运行当前项目测试", "请使用 super_memory 保存项目笔记"] {
            let mut runtime=CognitiveRuntime::with_mode(prompt,ReasoningMode::Adaptive);
            runtime.observe_project_context(1);
            assert_ne!(runtime.review_completion("已经完成全部要求，源码与测试及笔记操作都已验证。"),CompletionDecision::Accept,"{prompt}");
        }
    }
    fn note_receipt(verified:bool)->ToolExecutionReceipt {
        ToolExecutionReceipt {call_id:"note".into(),name:"super_memory".into(),status:ToolExecutionStatus::Succeeded,duration_ms:0,
            result:serde_json::json!({"status":"stored","operation_verified":verified}),original_result_chars:0,truncated:false,stored_result_reference:None,
            risk:ToolRisk::WorkspaceMutation,verification:false}
    }
    #[test]
    fn explicit_project_note_uses_runtime_readback_without_code_tests() {
        for prompt in ["请使用 super_memory 保存项目笔记，不要修改项目文件，也不用运行代码测试。", "请使用 super_memory 撤销本项目约定，不要改项目文件，只报告实际结果。"] {
            let mut runtime=CognitiveRuntime::with_mode(prompt,ReasoningMode::Adaptive);
            assert!(runtime.memory_only_scope,"{prompt}");
            assert_eq!(runtime.task_kind,TaskKind::Investigative);
            assert!(runtime.task_progress.lock().unwrap().rejects_mutation("run_command",ToolRisk::ProcessControl));
            assert!(runtime.task_progress.lock().unwrap().rejects_mutation("write_to_file",ToolRisk::WorkspaceMutation));
            assert!(!runtime.task_progress.lock().unwrap().rejects_mutation("super_memory",ToolRisk::WorkspaceMutation));
            runtime.observe_receipt(&note_receipt(true));
            assert_eq!(runtime.mutation_tool_calls,0,"a note is not a source edit");
            assert_eq!(runtime.review_completion("项目笔记的操作已读回确认；笔记本身仍是未经业务验证的历史资料。"),CompletionDecision::Accept);
        }
        let mut runtime=CognitiveRuntime::with_mode("请使用 super_memory 保存项目笔记",ReasoningMode::Adaptive);
        runtime.observe_receipt(&note_receipt(false));
        assert_ne!(runtime.review_completion("笔记已全部保存并且核对完成。"),CompletionDecision::Accept);
    }
    #[test]
    fn project_note_receipt_never_certifies_source_changes_or_requested_tests() {
        for prompt in ["请修复 calc.cjs 并使用 super_memory 保存经验", "请使用 super_memory 保存经验，然后修复 calc.cjs", "请使用 super_memory 保存约定，然后运行代码测试"] {
            let mut runtime=CognitiveRuntime::with_mode(prompt,ReasoningMode::Adaptive);
            assert!(!runtime.memory_only_scope,"{prompt}");
            runtime.observe_receipt(&note_receipt(true));
            assert_eq!(runtime.mutation_tool_calls,0);
            assert_ne!(runtime.review_completion("代码已经修改完毕，而且所有代码质量验证都已完成。"),CompletionDecision::Accept);
        }
    }
    fn process_inspection_receipt() -> ToolExecutionReceipt {
        ToolExecutionReceipt { call_id:"inspect-json".into(),name:"run_command".into(),
            status:ToolExecutionStatus::Succeeded,duration_ms:0,result:serde_json::json!({"exit_code":0,"stdout":"20 lines"}),
            original_result_chars:0,truncated:false,stored_result_reference:None,
            risk:ToolRisk::WorkspaceMutation,verification:false }
    }
    #[test]
    fn repeated_successful_inspection_prompts_strategy_review_without_action_evidence() {
        let mut r=CognitiveRuntime::with_mode("请修复规则并验证",ReasoningMode::Adaptive);
        for _ in 0..7 {r.observe_receipt(&process_inspection_receipt());}
        assert!(!r.next_guidance().unwrap().contains("[Inspection plateau]"));
        r.observe_receipt(&process_inspection_receipt());
        assert!(r.next_guidance().unwrap().contains("[Inspection plateau]"));
        assert_eq!(r.mutation_tool_calls,0);
    }
    #[test]
    fn investigation_scope_is_not_forced_into_implementation_by_observation_count() {
        let mut r=CognitiveRuntime::with_mode("只做调查：分析当前代码",ReasoningMode::Adaptive);
        for _ in 0..12 {r.observe_receipt(&process_inspection_receipt());}
        assert!(!r.next_guidance().unwrap().contains("[Inspection plateau]"));
        assert_eq!(r.task_kind(),TaskKind::Investigative);
    }
    #[test]
    fn edit_or_certified_check_resets_the_advisory() {
        for verification in [false,true] {
            let mut r=CognitiveRuntime::with_mode("请修复规则并验证",ReasoningMode::Adaptive);
            for _ in 0..8 {r.observe_receipt(&process_inspection_receipt());}
            r.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::WorkspaceMutation,verification);
            assert!(!r.next_guidance().unwrap().contains("[Inspection plateau]"));
        }
    }
    #[test]
    fn a_real_user_answer_resets_the_observation_advisory() {
        let mut r=CognitiveRuntime::with_mode("请修复规则并验证",ReasoningMode::Adaptive);
        for _ in 0..8 {r.observe_receipt(&process_inspection_receipt());}
        let mut reply=process_inspection_receipt();reply.name="ask_user".into();reply.risk=ToolRisk::ReadOnly;
        reply.result=serde_json::json!({"response":"使用项目内的参考"});
        r.observe_receipt(&reply);
        assert!(!r.next_guidance().unwrap().contains("[Inspection plateau]"));
    }
    #[test]
    fn process_risk_does_not_fabricate_a_file_change() {
        let mut r=CognitiveRuntime::with_mode("请修复规则并验证",ReasoningMode::Adaptive);
        let receipt=process_inspection_receipt();
        r.observe_receipt(&receipt);
        assert_eq!(receipt.risk,ToolRisk::WorkspaceMutation);
        assert_eq!(r.mutation_tool_calls,0);
        assert!(!r.plan.render().contains("[completed] Apply"));
        assert_ne!(r.review_completion("已经修改规则并完成验证，可交付使用。"),CompletionDecision::Accept);
    }
    #[test]
    fn actual_editor_receipt_still_advances_action() {
        let mut r=CognitiveRuntime::with_mode("请修复规则并验证",ReasoningMode::Adaptive);
        let mut receipt=process_inspection_receipt();r.observe_receipt(&receipt);
        receipt.name="write_to_file".into();receipt.call_id="edit".into();receipt.result=serde_json::json!({"success":true});
        r.observe_receipt(&receipt);
        assert_eq!(r.mutation_tool_calls,1);
        assert!(r.plan.render().contains("[completed] Apply"));
    }
    #[test]
    fn potentially_mutating_process_still_invalidates_prior_verification() {
        let mut r=CognitiveRuntime::with_mode("请修复规则并验证",ReasoningMode::Adaptive);
        r.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::ReadOnly,false);
        r.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::WorkspaceMutation,false);
        r.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::ProcessControl,true);
        assert!(r.plan.ready_for_response());
        r.observe_receipt(&process_inspection_receipt());
        assert!(!r.plan.ready_for_response());
    }
    #[test]
    fn natural_delivery_scopes_and_new_turns_keep_their_execution_requirements() {
        for (prompt, expected) in [
            ("只做复核：核对已有报告，不修改业务文件", TaskKind::Investigative),
            ("先调查计费故障，再修复代码并运行测试", TaskKind::Execution),
            ("根据skill生成海阔规则并验证播放", TaskKind::Execution),
            ("继续上轮未完成的修改，保留用户注释", TaskKind::Execution),
        ] {
            let runtime = CognitiveRuntime::with_mode(prompt, ReasoningMode::Adaptive);
            assert_eq!(runtime.task_kind(), expected, "{prompt}");
            assert!(!runtime.task_progress().lock().unwrap().has_plan(), "new Run must not reuse another plan");
        }
    }
    use super::*;

    #[test]
    fn repeated_plan_receipt_preserves_default_verification_but_changed_plan_invalidates_it() {
        let mut runtime = CognitiveRuntime::begin("实现本地函数并验证");
        let mut contract = serde_json::json!({"interpretation":"implement local function",
            "success_criteria":["expected behavior"],"steps":[{"id":"inspect","goal":"inspect",
            "acceptance":"source read","evidence_kind":"observation"}]});
        let make = |value: serde_json::Value| ToolExecutionReceipt {
            call_id: "plan".into(), name: "plan_task".into(), status: ToolExecutionStatus::Succeeded,
            duration_ms: 0, result: serde_json::json!({"task_contract":value}), original_result_chars: 0,
            truncated: false, stored_result_reference: None, risk: ToolRisk::ReadOnly, verification: false,
        };
        runtime.observe_receipt(&make(contract.clone()));
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl, true);
        let before = runtime.plan.render();
        runtime.observe_receipt(&make(contract.clone()));
        assert_eq!(runtime.plan.render(), before);
        assert_eq!(runtime.task_progress.lock().unwrap().snapshot()["plan_revision"], 1);
        contract["success_criteria"] = serde_json::json!(["expected behavior", "additional behavior"]);
        runtime.observe_receipt(&make(contract));
        assert_ne!(runtime.plan.render(), before);
        assert_eq!(runtime.task_progress.lock().unwrap().snapshot()["plan_revision"], 2);
    }

    #[test]
    fn explicit_read_only_investigation_never_requires_an_unrelated_mutation() {
        for objective in ["只做调查：检查代码错误与调用链，不修改业务源码", "请只做分析：当前代码为什么失败", "只做复核：读取已有报告，不重新跑探针，不改安装目录", "只做审查：根据现有证据纠正报告", "analysis only: inspect the current code error"] {
            let mut runtime=CognitiveRuntime::with_mode(objective,ReasoningMode::Adaptive);
            assert_eq!(runtime.task_kind(),TaskKind::Investigative);
            assert!(runtime.intent.capability.suggested_mutation_tools.is_empty());
            runtime.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::ReadOnly,false);
            assert_eq!(runtime.review_completion("已读取当前源码并核对调用链，边界错误的原因和影响已由实际工作区证据确认，本轮没有修改业务源码。"),CompletionDecision::Accept);
        }
        assert_eq!(CognitiveRuntime::with_mode("先调查代码错误，再修改并验证",ReasoningMode::Adaptive).task_kind(),TaskKind::Execution);
        assert_eq!(CognitiveRuntime::with_mode(
            r"D:\太极\动漫共和国概念版帮我看看，我安装的这个应用，是否可以转化为海阔世界的一个可用规则",
            ReasoningMode::Adaptive).task_kind(), TaskKind::Investigative);
        assert_eq!(CognitiveRuntime::with_mode("请根据skill生成海阔规则并验证", ReasoningMode::Adaptive)
            .task_kind(), TaskKind::Execution);
    }

    #[test]
    fn real_deadline_is_visible_and_cannot_be_expanded() {
        let mut runtime = CognitiveRuntime::with_mode("只做规划：分析当前项目", ReasoningMode::Adaptive);
        runtime.limit_model_turns(2);
        assert!(runtime.next_guidance().unwrap().contains("Remaining model turns including this request: 2 of 2"));
        runtime.limit_model_turns(99);
        assert!(runtime.next_guidance().unwrap().contains("Remaining model turns including this request: 1 of 2"));
        assert!(runtime.next_guidance().is_err());
    }

    #[test]
    fn classifies_execution_investigation_and_conversation() {
        assert_eq!(
            CognitiveRuntime::begin("继续提升这个项目").task_kind(),
            TaskKind::Execution
        );
        assert_eq!(
            CognitiveRuntime::begin("分析当前源码").task_kind(),
            TaskKind::Investigative
        );
        assert_eq!(
            CognitiveRuntime::begin("你好").task_kind(),
            TaskKind::Conversational
        );
    }

    #[test]
    fn evidence_dependent_draft_gets_exactly_one_review() {
        let mut runtime = CognitiveRuntime::with_mode("分析当前项目", ReasoningMode::Adaptive);
        assert!(matches!(
            runtime.review_completion("这是一个未经检查但看似很完整的长回答，它没有任何来自当前工作区的真实证据，因此仍然不能可靠地回答用户提出的问题。"),
            CompletionDecision::Revise { .. }
        ));
        assert!(matches!(
            runtime.review_completion("second draft"),
            CompletionDecision::Blocked { .. }
        ));
    }

    #[test]
    fn verified_execution_can_complete_without_forced_review() {
        let mut runtime = CognitiveRuntime::with_mode("修复代码", ReasoningMode::Adaptive);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::WorkspaceMutation,
            false,
        );
        runtime.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::ProcessControl,
            true,
        );
        let draft = "修改已经完成，并通过读取和验证确认新行为符合目标；同时保留了既有接口，未发现额外风险或尚未处理的失败路径。";
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
    }

    #[test]
    fn fast_mode_never_adds_a_review_turn() {
        let mut runtime = CognitiveRuntime::with_mode("你好", ReasoningMode::Fast);
        assert_eq!(
            runtime.review_completion("短回答"),
            CompletionDecision::Accept
        );
    }

    #[test]
    fn model_turn_budget_stops_unbounded_reasoning() {
        let mut runtime = CognitiveRuntime::with_mode("任务", ReasoningMode::Adaptive);
        runtime.max_model_turns = 2;
        assert!(runtime.next_guidance().is_ok());
        assert!(runtime.next_guidance().is_ok());
        assert!(runtime.next_guidance().is_err());
    }

    #[test]
    fn hypothesis_deadlock_warning_triggers_after_consecutive_failures() {
        let mut runtime = CognitiveRuntime::with_mode("修复Bug", ReasoningMode::Adaptive);
        assert_eq!(runtime.consecutive_failures(), 0);

        runtime.observe_tool(
            ToolExecutionStatus::Failed,
            ToolRisk::WorkspaceMutation,
            false,
        );
        assert_eq!(runtime.consecutive_failures(), 1);
        assert!(
            !runtime
                .next_guidance()
                .unwrap()
                .contains("HYPOTHESIS DEADLOCK WARNING")
        );

        runtime.observe_tool(
            ToolExecutionStatus::Failed,
            ToolRisk::WorkspaceMutation,
            false,
        );
        assert_eq!(runtime.consecutive_failures(), 2);
        assert!(
            !runtime
                .next_guidance()
                .unwrap()
                .contains("HYPOTHESIS DEADLOCK WARNING")
        );

        runtime.observe_tool(
            ToolExecutionStatus::Failed,
            ToolRisk::WorkspaceMutation,
            false,
        );
        assert_eq!(runtime.consecutive_failures(), 3);
        let guidance = runtime.next_guidance().unwrap();
        assert!(guidance.contains("HYPOTHESIS DEADLOCK WARNING"));
        assert!(guidance.contains("read-only tools"));

        // Success resets the consecutive failures counter
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        assert_eq!(runtime.consecutive_failures(), 0);
        assert!(
            !runtime
                .next_guidance()
                .unwrap()
                .contains("HYPOTHESIS DEADLOCK WARNING")
        );
    }

    #[test]
    fn test_anchor_constraints_persistence() {
        let mut runtime = CognitiveRuntime::begin("重构用户模块");
        assert!(!runtime.anchor_constraints().is_empty());

        runtime.add_anchor_constraint("Custom Ironclad Rule: Do not modify schema.sql");
        let guidance = runtime.next_guidance().unwrap();
        assert!(guidance.contains("PINNED ANCHOR CONSTRAINTS"));
        assert!(guidance.contains("Project Conventions"));
        assert!(!guidance.contains("LambdaQueryWrapper"));
        assert!(!guidance.contains("BaseController"));
        assert!(guidance.contains("Custom Ironclad Rule: Do not modify schema.sql"));
    }

    #[test]
    fn test_distilled_lessons_injection() {
        use crate::services::brain::evolution_memory::DistilledLesson;
        let mut runtime = CognitiveRuntime::begin("修改配置");
        runtime.apply_distilled_lessons(vec![DistilledLesson {
            recovery_evidence:None,
            id: "les_1".into(),
            updated_at: "2026-09-14T10:00:00Z".into(),
            domain: "config".into(),
            trigger_scenario: "updating yaml".into(),
            trap_pattern: "tab character indentation error".into(),
            golden_solution: "use 2 spaces instead of tabs".into(),
            confidence_score: 0.95,
            usage_count: 1,
        }]);

        let guidance = runtime.next_guidance().unwrap();
        assert!(guidance.contains("HERMES DISTILLED LESSONS"));
        assert!(guidance.contains("tab character indentation error"));
        assert!(guidance.contains("use 2 spaces instead of tabs"));
    }

    #[test]
    fn test_openclaw_claim_action_verifier_blocks_unmutated_claims() {
        let mut runtime = CognitiveRuntime::begin(
            "在 iam-service 中为用户注册逻辑实现一个单元测试用例 AuthTest.java",
        );
        // 模型口头宣称“已写入代码”，但没有调用任何 WorkspaceMutation 工具！
        let false_claim_draft = "我已经成功将代码写入了 AuthController.java，接口已经修改完毕！";
        let decision = runtime.review_completion(false_claim_draft);

        // OpenClaw 门禁必须严厉驳回！
        match decision {
            CompletionDecision::Revise { reason } => {
                assert!(
                    reason.contains("Claim-Action 违背拦截"),
                    "Actual reason was: {}",
                    reason
                );
                assert!(reason.contains("WorkspaceMutation"));
            }
            _ => panic!("Expected Revise decision from OpenClaw Claim-Action Verifier!"),
        }

        // 当真实发生只读探索、写盘与验证后，形成闭环允许放行
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::WorkspaceMutation,
            false,
        );
        runtime.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::ProcessControl,
            true,
        );
        assert_eq!(
            runtime.review_completion(false_claim_draft),
            CompletionDecision::Accept
        );
    }

    #[test]
    fn test_proactive_inquiry_guidance_and_unaligned_completion_gate() {
        let mut runtime = CognitiveRuntime::begin("帮我弄个接口");
        let guidance = runtime.next_guidance().unwrap();
        // 必须包含自我能力审视与主动反问引导
        assert!(guidance.contains("CAPABILITY INTROSPECTION"));
        assert!(guidance.contains("CONTEXT-FIRST REQUIREMENT CLARIFICATION"));

        // 未调查未反问直接试图完工，必须被主动对齐拦截器拦截！
        let blind_completion_draft = "我已经设计好了方案，这是所有的代码逻辑...";
        let decision = runtime.review_completion(blind_completion_draft);
        match decision {
            CompletionDecision::Revise { reason } => {
                assert!(reason.contains("主动对齐拦截"));
            }
            _ => panic!("Expected Revise decision from Proactive Alignment Gate!"),
        }
    }

    #[test]
    fn test_dod_verification_gate_end_to_end() {
        let mut runtime = CognitiveRuntime::begin("编写并验证爬虫脚本 spider.py");

        // 0. 先期探索 (满足 RunPlan 的 Inspect 阶段)
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);

        // 1. 模拟写盘修改
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation, false);

        // 2. Canonical receipt evidence is the first completion gate.
        let decision1 = runtime.review_completion("我已经写好了 spider.py 脚本。");
        match decision1 {
            CompletionDecision::Revise { reason } => {
                assert!(reason.contains("Run plan is incomplete"));
            }
            _ => panic!("Expected DoD rejection due to unverified mutation!"),
        }

        // 3. 运行了验证，但是验证报错了
        runtime.observe_tool_execution_details("run_command", Some("python spider.py".into()), false, "ConnectionRefusedError: port 8080");

        // 4. A failed check cannot buy another review or a successful ending.
        let decision2 = runtime.review_completion("我运行了，现在为您提供结果。");
        match decision2 {
            CompletionDecision::Blocked { reason } => {
                assert!(reason.contains("Run plan is incomplete"));
            }
            _ => panic!("Expected DoD rejection due to failed test!"),
        }

        // 5. A later run with actual successful verification can complete.
        let mut runtime = CognitiveRuntime::with_mode("编写并验证爬虫脚本 spider.py", ReasoningMode::Adaptive);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl, true);
        runtime.observe_tool_execution_details("run_command", Some("python -m pytest".into()), true, "1 passed");

        // 6. 再次交卷，顺利通过门禁放行
        let decision3 = runtime.review_completion("修改已经完成，并通过读取和验证确认新行为符合目标；同时保留了既有接口，未发现额外风险或尚未处理的失败路径。");
        assert_eq!(decision3, CompletionDecision::Accept);
    }

    #[test]
    fn verification_details_preserve_single_call_accounting() {
        let mut runtime = CognitiveRuntime::with_mode("实现并验证功能", ReasoningMode::Adaptive);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl, true);
        runtime.observe_tool_execution_details("run_command", Some("cargo test".into()), true, "1 passed");
        assert_eq!(runtime.tool_calls, 3);
        assert_eq!(runtime.review_completion("修改已经完成，检查通过且保留既有接口行为，执行结果由成功的验证收据支持。"), CompletionDecision::Accept);
    }

    #[test]
    fn supplemental_gate_cannot_extend_review_or_degrade_to_success() {
        let mut runtime = CognitiveRuntime::with_mode("解释这段代码", ReasoningMode::Adaptive);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool_execution_details("run_command", None, false, "verification failed");
        let draft = "已完成代码调查，以下结论根据实际读取的实现说明，并列出相关限制与证据。";
        assert!(matches!(runtime.review_completion(draft), CompletionDecision::Revise { .. }));
        assert!(matches!(runtime.review_completion(draft), CompletionDecision::Blocked { .. }));
        runtime.verification_gate.max_retries = 0;
        assert!(matches!(runtime.review_completion(draft), CompletionDecision::Blocked { .. }));
    }

    #[test]
    fn denied_process_is_not_a_failed_test_but_actual_execution_failure_still_blocks() {
        let mut runtime=CognitiveRuntime::with_mode("当前代码报错，先给修复方案，等我确认后再改。",ReasoningMode::Adaptive);
        let guidance=runtime.next_guidance().unwrap();
        assert!(!guidance.contains("[ACTION-FIRST DIRECTIVE]"));
        assert!(!guidance.contains("Run compile/test checks before claiming completion"));
        runtime.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::ReadOnly,false);
        let mut receipt=ToolExecutionReceipt {call_id:"denied".into(),name:"run_command".into(),status:ToolExecutionStatus::PolicyDenied,
            duration_ms:0,result:serde_json::json!({"kind":"read_only_task_scope"}),original_result_chars:0,truncated:false,stored_result_reference:None,risk:ToolRisk::WorkspaceMutation,verification:false};
        runtime.observe_receipt(&receipt);
        runtime.observe_executed_tool_details(&receipt,Some("node --test".into()),"not executed: read-only scope");
        assert_eq!(runtime.review_completion("已读取当前源码，建议修复计算方式；尚未实施或执行测试，等待用户确认。"),CompletionDecision::Accept);
        let mut actual=CognitiveRuntime::with_mode("调查当前文件",ReasoningMode::Adaptive);
        actual.observe_tool(ToolExecutionStatus::Succeeded,ToolRisk::ReadOnly,false);
        receipt.status=ToolExecutionStatus::Failed;receipt.result=serde_json::json!({"exit_code":1});
        actual.observe_receipt(&receipt);
        actual.observe_executed_tool_details(&receipt,Some("node --test".into()),"actual test failed");
        assert!(matches!(actual.review_completion("根据当前源码给出调查结论。"),CompletionDecision::Revise{..}|CompletionDecision::Blocked{..}));
    }

    #[test]
    fn rollback_reopens_stages_and_prevents_successful_evolution() {
        let mut runtime = CognitiveRuntime::with_mode("实现并验证功能", ReasoningMode::Adaptive);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl, true);
        runtime.invalidate_after_rollback();
        assert!(!matches!(runtime.review_completion("修改已经完成并经过验证，可交付所有目标与约束。"), CompletionDecision::Accept));
        assert!(!runtime.evolution_report(false).plan_closed);
    }

    #[test]
    fn baseline_quality_check_is_not_an_implementation_receipt() {
        let mut runtime = CognitiveRuntime::with_mode("实现并验证功能", ReasoningMode::Adaptive);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation, true);
        assert_eq!(runtime.mutation_tool_calls, 0);
        assert!(matches!(runtime.review_completion("已检查基线，尚没有实施目标功能，不能将其当作已经交付的变更。"), CompletionDecision::Revise { .. }));
        assert!(!runtime.evolution_report(false).plan_closed);
    }

    #[test]
    fn confirmed_user_choice_can_finish_without_a_fabricated_file_change() {
        let mut r=CognitiveRuntime::with_mode("请用ask_user问我选择A还是B，等我回答后简短确认，不做其他事情",ReasoningMode::Adaptive);
        assert_eq!(r.task_kind(),TaskKind::Conversational);
        r.observe_receipt(&ToolExecutionReceipt{call_id:"choice".into(),name:"ask_user".into(),status:ToolExecutionStatus::Succeeded,duration_ms:0,result:serde_json::json!({"requestId":"q","response":"A"}),original_result_chars:0,truncated:false,stored_result_reference:None,risk:ToolRisk::ReadOnly,verification:false});
        assert_eq!(r.review_completion("收到，你已选择A，本轮仅确认这个选择。"),CompletionDecision::Accept);
    }

    #[test]
    fn user_choice_does_not_complete_a_real_implementation_goal() {
        let mut r=CognitiveRuntime::with_mode("请生成一个页面，先问我选哪个颜色",ReasoningMode::Adaptive);
        r.observe_receipt(&ToolExecutionReceipt{call_id:"choice".into(),name:"ask_user".into(),status:ToolExecutionStatus::Succeeded,duration_ms:0,result:serde_json::json!({"requestId":"q","response":"蓝色"}),original_result_chars:0,truncated:false,stored_result_reference:None,risk:ToolRisk::ReadOnly,verification:false});
        assert_ne!(r.review_completion("已经确认你的颜色选择，任务全部完成。"),CompletionDecision::Accept);
    }
}
