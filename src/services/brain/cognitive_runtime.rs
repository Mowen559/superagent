use crate::services::brain::evolution_memory::{DistilledLesson, EvolutionReport};
use crate::services::brain::run_plan::RunPlan;
pub use crate::services::brain::run_plan::TaskKind;
use crate::services::brain::tool_execution_gateway::ToolExecutionStatus;
use crate::services::brain::tool_policy::ToolRisk;
pub use crate::domains::model_gateway::model_attempt_router::BrainTier;

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
}

/// Run-scoped reasoning control plane. It turns raw tool receipts into compact
/// progress guidance and allows exactly one completion review for work that
/// needs evidence or execution.
#[derive(Debug)]
pub struct CognitiveRuntime {
    objective: String,
    mode: ReasoningMode,
    task_kind: TaskKind,
    evidence_required: bool,
    model_turns: usize,
    tool_calls: usize,
    successful_tools: usize,
    failed_tools: usize,
    consecutive_failures: usize,
    completion_review_used: bool,
    max_model_turns: usize,
    plan: RunPlan,
    learned_guidance: Vec<String>,
    anchor_constraints: Vec<String>,
    distilled_lessons: Vec<DistilledLesson>,
}

impl CognitiveRuntime {
    pub fn begin(objective: &str) -> Self {
        Self::with_mode(objective, ReasoningMode::from_env())
    }

    pub fn with_mode(objective: &str, mode: ReasoningMode) -> Self {
        let normalized = objective.to_ascii_lowercase();
        let task_kind = classify_task(&normalized);
        let evidence_required = matches!(task_kind, TaskKind::Execution)
            || contains_any(
                &normalized,
                &[
                    "当前", "项目", "源码", "代码", "文件", "仓库", "日志", "错误", "current",
                    "project", "source", "code", "file", "repo", "log", "error",
                ],
            );
        let default_anchors = vec![
            "Strict Architecture: Return Result<T>, extend BaseController, use LambdaQueryWrapper (no JPA), and verify @RequirePermission.".to_string(),
            "Surgical Edits: Use replace_in_file/multi_replace; preserve unedited comments, docstrings, and formatting.".to_string(),
            "Verification Closed-Loop: Run compile/test checks before claiming completion; never fabricate tool output.".to_string(),
        ];
        Self {
            objective: objective.chars().take(500).collect(),
            mode,
            task_kind,
            evidence_required,
            model_turns: 0,
            tool_calls: 0,
            successful_tools: 0,
            failed_tools: 0,
            consecutive_failures: 0,
            completion_review_used: false,
            max_model_turns: DEFAULT_MAX_MODEL_TURNS,
            plan: RunPlan::for_task(task_kind),
            learned_guidance: Vec::new(),
            anchor_constraints: default_anchors,
            distilled_lessons: Vec::new(),
        }
    }

    pub fn task_kind(&self) -> TaskKind {
        self.task_kind
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
                "Plan fluently: inspect before changing, formulate precise edits, verify immediately, and self-correct with 'Aha' reflections whenever a barrier appears."
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
            "\n[HYPOTHESIS DEADLOCK WARNING] 3 or more consecutive tool actions failed! Your current mental model is stuck in a dead-end. Consider discarding this branch: use `revert_to_checkpoint` to restore workspace to a clean state, formulate an entirely alternative strategy, or use `synthesize_tool` to synthesize a dedicated tool/script to diagnose the problem."
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

        Ok(format!(
            "# Adaptive Meta-Cognitive State (DeepSeek-R1 Fluid Reasoning)\nObjective: {}\nMode: {:?}; Task Kind: {:?}; Model Turn: {}; Tool Stats: {} succeeded / {} failed.\nDynamic Plan:\n{}\nReasoning Guide: {}{}{}{}{}{}{}\nAdvance the in-progress step using live evidence. Do not claim completion without verification. Synthesize tools (`synthesize_tool`) if missing capabilities arise.",
            self.objective,
            self.mode,
            self.task_kind,
            self.model_turns,
            self.successful_tools,
            self.failed_tools,
            self.plan.render(),
            strategy,
            failure_note,
            deadlock_warning,
            replan_note,
            learned_note,
            lessons_block,
            anchor_block,
        ))
    }

    pub fn observe_tool(&mut self, status: ToolExecutionStatus, risk: ToolRisk) {
        self.tool_calls += 1;
        self.plan.observe_tool(status, risk);
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
        if self.completion_review_used || self.mode == ReasoningMode::Fast {
            return CompletionDecision::Accept;
        }

        self.plan.begin_completion();
        let plan_gap = self.plan.completion_gap();
        let needs_review = self.mode == ReasoningMode::Deliberate
            || (self.task_kind != TaskKind::Conversational
                && draft.trim().chars().count() < MIN_COMPLEX_DRAFT_CHARS)
            || (self.evidence_required && self.tool_calls == 0)
            || (self.failed_tools > 0 && self.successful_tools == 0)
            || plan_gap.is_some();

        if !needs_review {
            return CompletionDecision::Accept;
        }

        self.completion_review_used = true;
        let reason = if self.task_kind != TaskKind::Conversational
            && draft.trim().chars().count() < MIN_COMPLEX_DRAFT_CHARS
        {
            "The draft is too short for this task. Check whether it actually resolves the objective."
            .to_string()
        } else if self.evidence_required && self.tool_calls == 0 {
            "The objective depends on workspace evidence, but no tool evidence was collected. Inspect before concluding."
            .to_string()
        } else if self.failed_tools > 0 && self.successful_tools == 0 {
            "All attempted tools failed. Diagnose or clearly explain the blocker before concluding."
                .to_string()
        } else if let Some(gap) = plan_gap {
            format!(
                "The Run plan is incomplete at: {gap}. Complete it with evidence, or clearly explain the blocker."
            )
        } else {
            "Perform one deliberate completion review: check correctness, missing constraints, evidence, and residual risk."
            .to_string()
        };
        CompletionDecision::Revise { reason }
    }

    pub fn evolution_report(&mut self, completed: bool) -> EvolutionReport {
        self.plan.begin_completion();
        EvolutionReport {
            task_kind: self.task_kind,
            completed,
            plan_closed: self.plan.completion_gap().is_none(),
            replans: self.plan.replan_count(),
        }
    }
}

fn classify_task(normalized: &str) -> TaskKind {
    if contains_any(
        normalized,
        &[
            "实现",
            "修改",
            "修复",
            "添加",
            "新增",
            "移植",
            "提升",
            "提高",
            "完善",
            "继续",
            "重构",
            "删除",
            "implement",
            "build",
            "fix",
            "change",
            "edit",
            "add",
            "migrate",
            "improve",
            "refactor",
            "remove",
        ],
    ) {
        TaskKind::Execution
    } else if contains_any(
        normalized,
        &[
            "查看",
            "看下",
            "分析",
            "检查",
            "诊断",
            "调查",
            "review",
            "inspect",
            "analyze",
            "diagnose",
            "debug",
            "investigate",
        ],
    ) {
        TaskKind::Investigative
    } else {
        TaskKind::Conversational
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
        assert_eq!(classify_task("继续提升这个项目"), TaskKind::Execution);
        assert_eq!(classify_task("分析当前源码"), TaskKind::Investigative);
        assert_eq!(classify_task("你好"), TaskKind::Conversational);
    }

    #[test]
    fn evidence_dependent_draft_gets_exactly_one_review() {
        let mut runtime = CognitiveRuntime::with_mode("分析当前项目", ReasoningMode::Adaptive);
        assert!(matches!(
            runtime.review_completion("这是一个未经检查但看似很完整的长回答，它没有任何来自当前工作区的真实证据，因此仍然不能可靠地回答用户提出的问题。"),
            CompletionDecision::Revise { .. }
        ));
        assert_eq!(
            runtime.review_completion("second draft"),
            CompletionDecision::Accept
        );
    }

    #[test]
    fn verified_execution_can_complete_without_forced_review() {
        let mut runtime = CognitiveRuntime::with_mode("修复代码", ReasoningMode::Adaptive);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl);
        let draft = "修改已经完成，并通过读取和验证确认新行为符合目标；同时保留了既有接口，未发现额外风险或尚未处理的失败路径。";
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
    }

    #[test]
    fn fast_mode_never_adds_a_review_turn() {
        let mut runtime = CognitiveRuntime::with_mode("修复代码", ReasoningMode::Fast);
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

        runtime.observe_tool(ToolExecutionStatus::Failed, ToolRisk::WorkspaceMutation);
        assert_eq!(runtime.consecutive_failures(), 1);
        assert!(!runtime.next_guidance().unwrap().contains("HYPOTHESIS DEADLOCK WARNING"));

        runtime.observe_tool(ToolExecutionStatus::Failed, ToolRisk::WorkspaceMutation);
        assert_eq!(runtime.consecutive_failures(), 2);
        assert!(!runtime.next_guidance().unwrap().contains("HYPOTHESIS DEADLOCK WARNING"));

        runtime.observe_tool(ToolExecutionStatus::Failed, ToolRisk::WorkspaceMutation);
        assert_eq!(runtime.consecutive_failures(), 3);
        let guidance = runtime.next_guidance().unwrap();
        assert!(guidance.contains("HYPOTHESIS DEADLOCK WARNING"));
        assert!(guidance.contains("revert_to_checkpoint"));

        // Success resets the consecutive failures counter
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly);
        assert_eq!(runtime.consecutive_failures(), 0);
        assert!(!runtime.next_guidance().unwrap().contains("HYPOTHESIS DEADLOCK WARNING"));
    }

    #[test]
    fn test_anchor_constraints_persistence() {
        let mut runtime = CognitiveRuntime::begin("重构用户模块");
        assert!(!runtime.anchor_constraints().is_empty());

        runtime.add_anchor_constraint("Custom Ironclad Rule: Do not modify schema.sql");
        let guidance = runtime.next_guidance().unwrap();
        assert!(guidance.contains("PINNED ANCHOR CONSTRAINTS"));
        assert!(guidance.contains("Strict Architecture"));
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
}

