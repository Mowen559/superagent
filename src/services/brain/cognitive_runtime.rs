use crate::domains::agent_cognitive::intent::{ActionGoal, DynamicIntent, DynamicIntentEngine};
pub use crate::domains::model_gateway::model_attempt_router::BrainTier;
use crate::services::brain::evolution_memory::{DistilledLesson, EvolutionReport};
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
    failed_tools: usize,
    consecutive_failures: usize,
    completion_review_used: bool,
    max_model_turns: usize,
    plan: RunPlan,
    learned_guidance: Vec<String>,
    anchor_constraints: Vec<String>,
    distilled_lessons: Vec<DistilledLesson>,
    task_progress: SharedTaskProgress,
    proposal_only: bool,
}

impl CognitiveRuntime {
    pub fn begin(objective: &str) -> Self {
        Self::with_mode(objective, ReasoningMode::from_env())
    }

    pub fn with_mode(objective: &str, mode: ReasoningMode) -> Self {
        let mut intent = DynamicIntentEngine::analyze(objective);
        let normalized = objective.to_lowercase();
        let mut scope_prefix = normalized.trim_start();
        for _ in 0..3 {
            if let Some(rest) = ["这次", "本轮", "please ", "请你", "请", "帮我"]
                .iter()
                .find_map(|prefix| scope_prefix.strip_prefix(prefix))
            {
                scope_prefix = rest.trim_start_matches([' ', ':', '：']);
            } else {
                break;
            }
        }
        let proposal_only = [
            "只做规划",
            "只给方案",
            "只要方案",
            "只制定计划",
            "仅做规划",
            "planning only",
            "plan only",
            "proposal only",
        ]
        .iter()
        .any(|marker| scope_prefix.starts_with(marker));
        let proposal_needs_workspace = contains_any(
            &normalized,
            &[
                "当前",
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

        for path in &intent.constraints.protected_paths {
            default_anchors.push(format!("【IRONCLAD INVARIANT】Path '{}' is strictly protected from mutation under user mandate!", path));
        }
        for pattern in &intent.constraints.forbidden_patterns {
            default_anchors.push(format!(
                "【IRONCLAD INVARIANT】Forbidden pattern: {}",
                pattern
            ));
        }

        let plan = if !proposal_only && intent.goal == ActionGoal::VerificationBenchmark {
            RunPlan::for_verification()
        } else {
            RunPlan::for_task(task_kind)
        };
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
            failed_tools: 0,
            consecutive_failures: 0,
            completion_review_used: false,
            max_model_turns: DEFAULT_MAX_MODEL_TURNS,
            plan,
            learned_guidance: Vec::new(),
            anchor_constraints: default_anchors,
            distilled_lessons: Vec::new(),
            task_progress: std::sync::Arc::new(std::sync::Mutex::new(TaskProgress::default())),
            proposal_only,
        }
    }

    pub fn task_kind(&self) -> TaskKind {
        self.task_kind
    }

    pub fn task_progress(&self) -> SharedTaskProgress {
        self.task_progress.clone()
    }

    pub fn consecutive_failures(&self) -> usize {
        self.consecutive_failures
    }

    pub fn select_brain_tier(&self) -> BrainTier {
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

        let strategy = match self.task_kind {
            TaskKind::Conversational => {
                "Answer directly. Use tools only when the answer depends on workspace or live evidence."
            }
            TaskKind::Investigative => {
                "Form dynamic hypotheses, inspect authoritative evidence first, distinguish facts from inference, and pivot rapidly if assumptions fail."
            }
            TaskKind::Execution => {
                "Inspect the relevant state, make the smallest coherent change, run a quality check, and use actual failures to revise the next action."
            }
        };

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
            buf.push_str("\nAdopt these golden solutions directly to prevent repeated traps!");
            buf
        };

        let anchor_block = if self.anchor_constraints.is_empty() {
            String::new()
        } else {
            format!(
                "\n[PINNED ANCHOR CONSTRAINTS - IMMUNE TO FORGETTING/DRIFT]\n- {}",
                self.anchor_constraints.join("\n- ")
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
            self.intent
                .capability
                .suggested_verification_tools
                .join(", "),
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
                "\n[CONTEXT-FIRST REQUIREMENT CLARIFICATION]\nThe request may have unresolved choices. First inspect supplied workspace context and relevant code, then use plan_task to distinguish known constraints from context gaps. Use tools to resolve factual gaps. Choose reasonable defaults for reversible details; ask the user only when a remaining choice materially changes scope or behavior.\nInvestigation angles:\n- {}",
                self.intent.capability.recommended_inquiries.join("\n- ")
            )
        } else {
            String::new()
        };

        let understanding = self.task_progress.lock().unwrap_or_else(|e| e.into_inner()).render()
            .unwrap_or_else(|| {
                "\n[Task Understanding]\nFor multiple deliverables, dependencies, or material ambiguity: use supplied context and targeted inspection, then call `plan_task` with the intended outcome, success criteria, concise dependency-ordered subgoals, constraints and remaining gaps. Group work by independently verifiable outcome; do not expand speculative API signatures or duplicate checks. If inspection cannot obtain a fact, record a context gap instead of repeatedly reading unavailable sources. Distinguish known requirements from proposed design. Ask only about material unresolved choices. For a simple request, proceed directly. A plan is a hypothesis, never execution evidence.\n".to_string()
            });

        Ok(format!(
            "# Run Decision State\nObjective: {}\nMode: {:?}; Task Kind: {:?}; Model Turn: {}; Tool Stats: {} succeeded / {} failed (Mutations: {}).\nDynamic Plan:\n{}\nReasoning Guide: {}{}{}{}{}{}{}{}{}{}\nAdvance the in-progress step using live evidence. Do not claim completion without verification.",
            self.objective,
            self.mode,
            self.task_kind,
            self.model_turns,
            self.successful_tools,
            self.failed_tools,
            self.mutation_tool_calls,
            self.plan.render(),
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
        ))
    }

    pub fn observe_receipt(&mut self, receipt: &ToolExecutionReceipt) {
        self.task_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe(receipt);
        if receipt.name == "plan_task" && receipt.status == ToolExecutionStatus::Succeeded {
            if let Some(value) = receipt.result.get("task_contract") {
                if let Ok(contract) = TaskContract::from_value(value.clone()) {
                    if self.task_progress.lock().unwrap_or_else(|e| e.into_inner()).has_plan() {
                        self.plan.invalidate_verification();
                    }
                    self.task_progress
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .apply_plan(contract);
                }
            }
            // Planning is useful context, but not an observation of the world.
            return;
        }
        if receipt.name == "manage_task" && receipt.status == ToolExecutionStatus::Succeeded {
            return;
        }
        if receipt.name == "revise_task_plan" && receipt.status == ToolExecutionStatus::Succeeded {
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
        self.observe_tool(receipt.status, receipt.risk, receipt.verification);
    }

    fn observe_tool(&mut self, status: ToolExecutionStatus, risk: ToolRisk, verification: bool) {
        self.tool_calls += 1;
        if risk == ToolRisk::WorkspaceMutation && status == ToolExecutionStatus::Succeeded {
            self.mutation_tool_calls += 1;
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

    pub fn review_completion(&mut self, draft: &str) -> CompletionDecision {
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

        let integrity_gap = if self.task_kind == TaskKind::Execution
            && claims_written
            && self.mutation_tool_calls == 0
        {
            Some("Claim-Action 违背拦截: No successful WorkspaceMutation supports the claimed changes. Inspect the workspace, perform the authorized change, and verify it.".to_string())
        } else if self.intent.capability.requires_user_inquiry
            && self.tool_calls == 0
            && self.task_kind == TaskKind::Execution
        {
            Some("主动对齐拦截: Inspect the workspace first. Ask the user only about material choices that remain unresolved after inspection.".to_string())
        } else if self.evidence_required && self.successful_tools == 0 {
            Some("No successful tool evidence supports this objective. Inspect authoritative sources and address tool failures before concluding.".to_string())
        } else {
            self.plan.completion_gap().map(|gap| format!(
                "The Run plan is incomplete at: {gap}. Complete it with tool evidence; verification requires a successful quality check after the latest change."
            )).or_else(|| self.task_contract_gap())
        };

        if let Some(reason) = integrity_gap {
            // Review is bounded, but its use must never become a bypass for
            // missing evidence. Failed Runs must not train success strategies.
            if self.completion_review_used || self.mode == ReasoningMode::Fast {
                return CompletionDecision::Blocked { reason };
            }
            self.completion_review_used = true;
            return CompletionDecision::Revise { reason };
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
    use super::*;

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
}
