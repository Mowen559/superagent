use crate::services::brain::evolution_memory::{DistilledLesson, EvolutionReport};
use crate::domains::agent_cognitive::intent::{ActionGoal, DynamicIntent, DynamicIntentEngine};
use crate::domains::agent_cognitive::verification_gate::{GateDecision, VerificationGateGuard, VerificationPolicy};
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
    pub socratic_payload: Option<crate::domains::agent_cognitive::socratic_inquiry::SocraticInquiryPayload>,
    pub verification_gate: VerificationGateGuard,
}

impl CognitiveRuntime {
    pub fn begin(objective: &str) -> Self {
        Self::with_mode(objective, ReasoningMode::from_env())
    }

    pub fn with_mode(objective: &str, mode: ReasoningMode) -> Self {
        let intent = DynamicIntentEngine::analyze(objective);
        let task_kind = match intent.goal {
            ActionGoal::ArchitectureExploration | ActionGoal::TargetedInvestigation => TaskKind::Investigative,
            ActionGoal::SurgicalExecution | ActionGoal::VerificationBenchmark | ActionGoal::RefactoringModernization => TaskKind::Execution,
            ActionGoal::ClarificationDialogue => TaskKind::Conversational,
        };

        let evidence_required = matches!(task_kind, TaskKind::Execution | TaskKind::Investigative)
            || contains_any(
                &objective.to_lowercase(),
                &[
                    "当前", "项目", "源码", "代码", "文件", "仓库", "日志", "错误", "current",
                    "project", "source", "code", "file", "repo", "log", "error",
                ],
            );

        let mut default_anchors = vec![
            "High-Cohesion Architecture: Respect module boundaries and existing framework patterns; strictly prevent cross-domain contamination.".to_string(),
            "Surgical Edits: Use replace_in_file/multi_replace; preserve unedited comments, docstrings, and formatting.".to_string(),
            "Verification Closed-Loop: Run compile/test checks before claiming completion; never fabricate tool output.".to_string(),
        ];

        for path in &intent.constraints.protected_paths {
            default_anchors.push(format!("【IRONCLAD INVARIANT】Path '{}' is strictly protected from mutation under user mandate!", path));
        }
        for pattern in &intent.constraints.forbidden_patterns {
            default_anchors.push(format!("【IRONCLAD INVARIANT】Forbidden pattern: {}", pattern));
        }

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
            plan: RunPlan::for_task(task_kind),
            learned_guidance: Vec::new(),
            anchor_constraints: default_anchors,
            distilled_lessons: Vec::new(),
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

        let pivot_mandate = if self.consecutive_failures >= 2 {
            "\n[COGNITIVE PIVOT MANDATE - 连续受阻强制变轨]\n⚠️ You have encountered 2 or more consecutive tool failures! Continuing down this identical line of thought is STRICTLY FORBIDDEN.\nExecute one of these 3 pivot actions immediately:\n1. Pivot strategy: If static scraping fails, pivot to mobile APIs or the local Python scraper engine (127.0.0.1:18000). If code edits fail, inspect the exact file with `view_file` to verify whitespace.\n2. Verify ground facts: Call a read-only tool (`view_file`, `grep_search`) to inspect the real underlying code.\n3. Re-evaluate assumptions: Discard invalid premises and try an alternative approach.\n"
        } else {
            ""
        };

        let current_phase_desc = if self.tool_calls == 0 {
            "Phase 1: Grounding & Investigation [IN PROGRESS]\n- Mandatory: Inspect current codebase, reverse-engineer mobile APIs or inspect specs first.\n- Action: Invoke sensory tools (`view_file`, `grep_search`, `read_file`, `web_fetch`). Do NOT mutate files or claim completion yet!"
        } else if self.mutation_tool_calls == 0 {
            "Phase 2: Surgical Implementation & Rule Architecture [READY]\n- Evidence gathered. Formulate precision edits or isomorphic rule logic.\n- Action: Use `replace_in_file` / `write_to_file`. Keep mutations minimal and surgical."
        } else if self.successful_tools > 0 {
            "Phase 3: Verification Closed-Loop [MANDATORY]\n- Mutations landed. Run compiler/linter/test suites to verify claims.\n- Action: Run commands (`cargo test`, `npm test`, `validate-rule`) to ensure 0 errors."
        } else {
            "Phase 4: Synthesis & Delivery [ACTIVE]\n- Validations complete. Deliver final summary, Diff, or import tokens clearly."
        };

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
            if self.intent.capability.suggested_mutation_tools.is_empty() { "None (Read-Only)".to_string() } else { self.intent.capability.suggested_mutation_tools.join(", ") },
            self.intent.capability.suggested_verification_tools.join(", "),
            self.intent.ambiguity_score
        );

        let inquiry_block = if let Some(ref payload) = self.socratic_payload {
            if self.inquiry_tool_calls == 0 && self.tool_calls == 0 {
                format!(
                    "\n[PROACTIVE SOCRATIC INQUIRY MANDATE - 结构化技术推敲要求]\n{}\n\n【行动指引】: 面对高歧义需求，禁止直接动手盲目修改代码！你必须优先调用轻量感知工具（`project_index`/`search_graph`）摸清现状，然后通过 `ask_user` 或在本次回复中清晰输出上述三段式推敲（事实+权衡+推荐），引导用户对齐后再执行！",
                    payload.render_socratic_prompt()
                )
            } else {
                String::new()
            }
        } else if self.intent.capability.requires_user_inquiry && self.inquiry_tool_calls == 0 && self.tool_calls == 0 {
            format!(
                "\n[PROACTIVE INQUIRY REQUIRED - Socratic Principal Mindset]\nWait! This requirement has key technical branches or underspecified details (Score: {:.2}).\nDO NOT rush into blind implementation! Adopt the dual-track approach:\n1. Use read-only tools (`project_index`, `search_graph`) to inspect the current codebase.\n2. In parallel, formulate proactive, structured questions to the user (via `ask_user` or text) with:\n   - Verified codebase facts\n   - Trade-offs (Option A vs Option B)\n   - Concrete recommendations\nSuggested inquiry angles:\n- {}",
                self.intent.ambiguity_score,
                self.intent.capability.recommended_inquiries.join("\n- ")
            )
        } else {
            String::new()
        };

        let mut res = format!(
            "# Adaptive Meta-Cognitive State (DeepSeek-R1 Fluid Reasoning)\nObjective: {}\nMode: {:?}; Task Kind: {:?}; Model Turn: {}; Tool Stats: {} succeeded / {} failed (Mutations: {}).\nDynamic Plan:\n{}\nReasoning Guide: {}{}{}{}{}{}{}{}{}\nAdvance the in-progress step using live evidence. Do not claim completion without verification. Synthesize tools (`synthesize_tool`) if missing capabilities arise.",
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
        );
        res.push_str(&format!("{}{}", phase_progression_block, pivot_mandate));
        Ok(res)
    }

    pub fn observe_tool(&mut self, status: ToolExecutionStatus, risk: ToolRisk) {
        self.tool_calls += 1;
        if risk == ToolRisk::WorkspaceMutation && status == ToolExecutionStatus::Succeeded {
            self.mutation_tool_calls += 1;
            self.verification_gate.observe_tool_execution("write_to_file", None, true, "");
        }
        if risk == ToolRisk::ProcessControl && status == ToolExecutionStatus::Succeeded {
            self.verification_gate.observe_tool_execution("run_command", None, true, "Execution succeeded");
        }
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

    pub fn observe_tool_execution_details(
        &mut self,
        tool_name: &str,
        command_arg: Option<String>,
        success: bool,
        output_snippet: &str,
    ) {
        self.tool_calls += 1;
        self.verification_gate
            .observe_tool_execution(tool_name, command_arg, success, output_snippet);
    }

    pub fn review_completion(&mut self, draft: &str) -> CompletionDecision {
        let is_conversational = self.task_kind == TaskKind::Conversational;
        match self.verification_gate.evaluate_conclusion_attempt(is_conversational) {
            GateDecision::RejectAndSteer {
                reason,
                suggested_action,
                current_attempt,
                max_attempts,
            } => {
                return CompletionDecision::Revise {
                    reason: format!(
                        "【DoD 自动化验证门禁拦截 (第 {}/{} 次)】: {}\n【必须行动】: {}",
                        current_attempt, max_attempts, reason, suggested_action
                    ),
                };
            }
            GateDecision::MaxRetriesDegraded { attempts, diagnostic } => {
                eprintln!(
                    "\x1b[33;1m[DoD Gate Alert] 自检重试超限 ({} 次)，转为诚实透明报告结题: {}\x1b[0m",
                    attempts, diagnostic
                );
            }
            GateDecision::Passed => {}
        }
        if self.completion_review_used || self.mode == ReasoningMode::Fast {
            return CompletionDecision::Accept;
        }

        // 【OpenClaw 核心防线 1】：声称写盘但物理落盘为 0，坚决通过 Revise 打回！
        let claims_written = [
            "写入", "落盘", "创建了", "已创建", "修改了", "已修改", "修改完毕", "已经更新", "代码已更新",
            "created", "written", "modified", "added the test"
        ].iter().any(|kw| draft.contains(kw));

        if claims_written && self.mutation_tool_calls == 0 {
            return CompletionDecision::Revise {
                reason: "【OpenClaw Claim-Action 违背拦截】：你在回答中声称已经将代码或测试写入了项目，但在实际执行中没有发生任何 WorkspaceMutation（写盘）工具调用！禁止空口宣称完成！你必须立即调用 write_to_file 或 replace_in_file 工具将改动物理落盘，再进行验证！".to_string(),
            };
        }

        // 【主动对齐核心防线 2】：高模糊度需求在未调查未对齐前严禁直接结题
        if self.intent.capability.requires_user_inquiry && self.tool_calls == 0 && self.task_kind == TaskKind::Execution {
            return CompletionDecision::Revise {
                reason: "【主动对齐拦截】：该需求包含关键架构分支或细节未定。你在未翻查代码现状且未与用户对齐确认的情况下试图直接结题。请先调用工具调查或使用 ask_user 反问用户澄清关键细节！".to_string(),
            };
        }

        let has_code_block = draft.contains("```");
        let has_substantial_output = draft.trim().chars().count() >= 100
            && (draft.contains("```") || draft.contains("###") || draft.contains("【方案") || draft.contains("💡") || draft.contains("1.") || draft.contains("一、"));

        // 【阶段锁硬门禁 3】：代码执行/排障任务严禁未查证代码事实就单轮直接结题 (Anti-Greedy Conclude)
        let is_investigate_or_execute = matches!(self.task_kind, TaskKind::Execution | TaskKind::Investigative);
        if is_investigate_or_execute && self.evidence_required && self.tool_calls == 0 && !has_code_block {
            return CompletionDecision::Revise {
                reason: "【认知阶段门禁拦截 (Phase Barrier)】：当前任务涉及工程代码落地或疑难排障，严禁在未调用任何感知工具（grep_search / view_file / read_file / project_index 等）查实代码事实的情况下单轮直接结题！请立即调用工具探查真实代码后再做结论！".to_string(),
            };
        }
        let is_empty_promise = [
            "让我为你", "让我来为你", "我们将", "我们可以", "请稍候", "稍后为你", "稍后整理", "稍后修复"
        ].iter().any(|kw| draft.contains(kw));

        // 【海阔规则工程智能门禁】：区分修复排障与新建口令交付，坚决打假同时赋能自主推理
        let raw_p_lower = self.intent.raw_prompt.to_lowercase();
        let is_repair_or_debug = [
            "修复", "报错", "改动", "解决", "为什么", "问题", "bug", "error", "fix", "debug", "分析"
        ].iter().any(|kw| raw_p_lower.contains(kw));

        // 【反画饼刚性门禁】：若为排障/修复任务，但既未修改文件又未给出代码块，且含空口承诺，坚决打回！
        if is_repair_or_debug && self.mutation_tool_calls == 0 && !has_code_block && is_empty_promise {
            return CompletionDecision::Revise {
                reason: "【严禁画饼应付拦截】：你在回答中声称“让我为你详细整理并修复”，但当前回复中既没有调用文件修改工具写入工程，也没有提供任何具体的修复代码块（```）！禁止以空口承诺结束任务！请立即调用 replace_in_file 修改文件，或在回答中输出完整的修复代码与 Diff！".to_string(),
            };
        }

        let is_hiker_engineering = (raw_p_lower.contains("海阔") || raw_p_lower.contains("hiker") || raw_p_lower.contains("视界"))
            && (raw_p_lower.contains("规则") || raw_p_lower.contains("小程序") || raw_p_lower.contains("写") || raw_p_lower.contains("做") || raw_p_lower.contains("生成") || raw_p_lower.contains("口令") || raw_p_lower.contains("爬虫"));

        if is_hiker_engineering {
            // 场景 A：若是报错修复或问题排查，只要给出了深入分析与实质修复代码，直接验收通过！
            if is_repair_or_debug && has_substantial_output && (self.mutation_tool_calls > 0 || has_code_block || !is_empty_promise) {
                eprintln!("\x1b[32;1m[Hiker Gate] ✅ 规则排障/修复任务已给出详尽分析与实质代码方案，验收通过！\x1b[0m");
                return CompletionDecision::Accept;
            }

            // 场景 B：若用户在生成全新规则或索要口令
            let claims_any_token = draft.contains("pasteme.tyrantg.com/") || draft.contains("云6oooole/") || draft.contains("￥home_rule_v2￥");
            if claims_any_token && self.tool_calls == 0 {
                return CompletionDecision::Revise {
                    reason: "【物理收据硬门禁拦截 (Fabrication Guard)】：你在未执行任何工具命令（tool_calls == 0）的情况下凭空捏造了海阔口令或 PasteMe 分享链接！真实的口令必须通过运行规则导出工具 (如 export-command.cjs) 真实上传后获取！严禁凭空编造！请立即调用命令工具真实生成！".to_string(),
                };
            }

            if has_substantial_output && self.tool_calls > 0 && claims_any_token {
                eprintln!("\x1b[32;1m[Hiker Gate] ✅ 规则工程已真实调用工具闭环执行，物理收据完备，验收通过！\x1b[0m");
                return CompletionDecision::Accept;
            }
        }

        // 规则编写、方案咨询、代码生成等创作与架构设计任务，若草稿充分且未声称物理写盘，允许直接高质量结题，绝不机械强求收集工作区证据
        let is_generative_or_consultative = [
            "编写", "生成", "怎么写", "如何", "方案", "样例", "demo", "设计", "转换",
            "rule", "generate", "create a rule", "how to", "example", "architecture", "design"
        ].iter().any(|kw| self.intent.raw_prompt.to_lowercase().contains(kw));

        if is_generative_or_consultative && has_substantial_output && !claims_written {
            return CompletionDecision::Accept;
        }

        self.plan.begin_completion();
        let plan_gap = self.plan.completion_gap();
        let needs_review = self.mode == ReasoningMode::Deliberate
            || (self.task_kind != TaskKind::Conversational
                && draft.trim().chars().count() < MIN_COMPLEX_DRAFT_CHARS)
            || (self.evidence_required && self.tool_calls == 0 && !is_generative_or_consultative)
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
            "The objective depends on workspace evidence, but no tool evidence was collected. Proactively call agile inspection tools (`project_index` to map architecture/AST topology in milliseconds, or `view_outline`/`search_graph`) before concluding. Do not run heavy full-repo builds like `mvn test` during discovery."
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
        assert!(guidance.contains("High-Cohesion Architecture"));
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
        let mut runtime = CognitiveRuntime::begin("在 iam-service 中为用户注册逻辑实现一个单元测试用例 AuthTest.java");
        // 模型口头宣称“已写入代码”，但没有调用任何 WorkspaceMutation 工具！
        let false_claim_draft = "我已经成功将代码写入了 AuthController.java，接口已经修改完毕！";
        let decision = runtime.review_completion(false_claim_draft);

        // OpenClaw 门禁必须严厉驳回！
        match decision {
            CompletionDecision::Revise { reason } => {
                assert!(
                    reason.contains("OpenClaw Claim-Action 违背拦截"),
                    "Actual reason was: {}",
                    reason
                );
                assert!(reason.contains("WorkspaceMutation"));
            }
            _ => panic!("Expected Revise decision from OpenClaw Claim-Action Verifier!"),
        }

        // 当真实发生只读探索、写盘与验证后，形成闭环允许放行
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl);
        assert_eq!(runtime.review_completion(false_claim_draft), CompletionDecision::Accept);
    }

    #[test]
    fn test_proactive_inquiry_guidance_and_unaligned_completion_gate() {
        let mut runtime = CognitiveRuntime::begin("帮我弄个接口");
        let guidance = runtime.next_guidance().unwrap();
        // 必须包含自我能力审视与主动反问引导
        assert!(guidance.contains("CAPABILITY INTROSPECTION"));
        assert!(guidance.contains("PROACTIVE SOCRATIC INQUIRY MANDATE") || guidance.contains("PROACTIVE INQUIRY REQUIRED"));

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
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly);

        // 1. 模拟写盘修改
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation);
        
        // 2. 试图直接交卷，必须被 DoD 门禁拦截打回
        let decision1 = runtime.review_completion("我已经写好了 spider.py 脚本。");
        match decision1 {
            CompletionDecision::Revise { reason } => {
                assert!(reason.contains("DoD 自动化验证门禁拦截"));
                assert!(reason.contains("尚未执行任何运行验证或测试"));
            }
            _ => panic!("Expected DoD rejection due to unverified mutation!"),
        }

        // 3. 运行了验证，但是验证报错了
        runtime.observe_tool_execution_details("run_command", Some("python spider.py".into()), false, "ConnectionRefusedError: port 8080");

        // 4. 再次试图交卷，必须被拦截提示报错未修复
        let decision2 = runtime.review_completion("我运行了，现在为您提供结果。");
        match decision2 {
            CompletionDecision::Revise { reason } => {
                assert!(reason.contains("DoD 自动化验证门禁拦截"));
                assert!(reason.contains("ConnectionRefusedError"));
            }
            _ => panic!("Expected DoD rejection due to failed test!"),
        }

        // 5. 修复代码并重新运行，测试全绿通过
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation);
        runtime.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl);

        // 6. 再次交卷，顺利通过门禁放行
        let decision3 = runtime.review_completion("修改已经完成，并通过读取和验证确认新行为符合目标；同时保留了既有接口，未发现额外风险或尚未处理的失败路径。");
        assert_eq!(decision3, CompletionDecision::Accept);
    }
}

