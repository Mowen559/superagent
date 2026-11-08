use serde::{Deserialize, Serialize};

/// 智能体主要意图目标分类 (Primary Action Goal)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionGoal {
    /// 架构全景画像与依赖拓扑探索
    ArchitectureExploration,
    /// 针对性排错与符号/堆栈调查
    TargetedInvestigation,
    /// 局部精确外科手术式代码实现
    SurgicalExecution,
    /// 测试套件与回归验证
    VerificationBenchmark,
    /// 模块演进与重构
    RefactoringModernization,
    /// 需求对齐与澄清对话
    ClarificationDialogue,
}

/// 智能体自我能力审视与工具映射结果 (Capability Self-Awareness)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityAssessment {
    /// 识别到的适用感知工具 (如 search_graph, project_index, grep_search)
    pub suggested_inspection_tools: Vec<String>,
    /// 识别到的适用修改工具 (如 replace_in_file)
    pub suggested_mutation_tools: Vec<String>,
    /// 识别到的适用验证工具 (如 run_command, native_harness)
    pub suggested_verification_tools: Vec<String>,
    /// 是否需要使用主动反问工具 (ask_user)
    pub requires_user_inquiry: bool,
    /// 推荐向用户发起的深度澄清问题与方案对比
    pub recommended_inquiries: Vec<String>,
}

/// 否定约束与硬性防线 (Negative Invariants & Prohibitions)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegativeConstraint {
    /// 受物理写保护的目录 (如 ["common/", "database/migration/"])
    pub protected_paths: Vec<String>,
    /// 禁止使用的框架或模式 (如 ["JPA", "Hibernate", "手写 Map 返回值"])
    pub forbidden_patterns: Vec<String>,
    /// 是否为纯只读模式
    pub read_only_mode: bool,
}

/// 动态意图与认知解析产物
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicIntent {
    pub raw_prompt: String,
    pub goal: ActionGoal,
    pub target_service: Option<String>,
    pub target_symbols: Vec<String>,
    pub constraints: NegativeConstraint,
    pub capability: CapabilityAssessment,
    pub ambiguity_score: f32, // 0.0 ~ 1.0
}

pub struct DynamicIntentEngine;

/// Current authorization phase, separated from proposed future actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentRequestStage { Execution, ProposalOnly, InvestigationOnly }

impl CurrentRequestStage {
    pub fn classify(prompt: &str) -> Self {
        let quoted = regex::Regex::new(r#"(?s)```.*?(?:```|$)|“[^”]*”|「[^」]*」|"[^"\n]*"|(?:^|[ \t(])'[^'\n]*'"#).unwrap();
        let normalized = quoted.replace_all(prompt, " ").to_lowercase();
        // A trailing, unqualified no-change instruction governs the current
        // task. Exact clause boundaries avoid treating "don't change file B"
        // as a ban on an otherwise authorized edit to file A.
        let no_changes = regex::Regex::new(r"(?:^|[，,。.;；\n])\s*(?:请|先)?(?:不要|禁止|别)(?:修改|改动)(?:任何)?(?:文件|代码)\s*[。.!！]*\s*$").unwrap();
        if no_changes.is_match(&normalized) { return Self::InvestigationOnly; }
        let mut prefix = normalized.trim_start();
        for _ in 0..3 {
            if let Some(rest) = ["这次", "本轮", "please ", "请你", "请", "帮我"].iter().find_map(|p| prefix.strip_prefix(p)) {
                prefix = rest.trim_start_matches([' ', ':', '：']);
            } else { break; }
        }
        if ["只做规划","只给方案","只要方案","只制定计划","仅做规划","planning only","plan only","proposal only"]
            .iter().any(|p| prefix.starts_with(p)) { return Self::ProposalOnly; }
        if ["只做调查","只做分析","只做复核","只做审查","只分析","仅分析","只检查","只做逆向理解","investigation only","analysis only","read-only investigation"]
            .iter().any(|p| prefix.starts_with(p)) { return Self::InvestigationOnly; }
        let clauses: Vec<_> = normalized.split(['，','。','；',';', '\n']).filter(|c| {
            !["不用等","无需等","不必等","无需确认","不用确认","do not wait","don't wait","no approval needed"].iter().any(|p| c.contains(p))
        }).collect();
        let planning = clauses.iter().any(|c| ["先","first","本轮","这次"].iter().any(|p| c.contains(p)))
            && clauses.iter().any(|c| ["方案","计划","选项","proposal","plan","options"].iter().any(|p| c.contains(p)));
        let deferred = clauses.iter().any(|c| ["等我确认","待我确认","等我选择","等我选定","等我批准","确认后再","同意后再","批准后再","选定后再","选择后再","暂不实施","先不要改","先别改","wait for my approval","wait for my confirmation","until i approve","until i confirm","after my approval"]
            .iter().any(|p| c.contains(p)));
        if planning && deferred { Self::ProposalOnly } else { Self::Execution }
    }
}

impl DynamicIntentEngine {
    /// 对用户输入进行深度意图解析、能力自省与信息鸿沟评估
    pub fn analyze(prompt: &str) -> DynamicIntent {
        let normalized = prompt.to_lowercase();

        // 1. 判定核心目标
        let mut goal = Self::resolve_goal(&normalized);

        // 2. 提取目标微服务与实体
        let target_service = Self::extract_target_service(&normalized);
        let target_symbols = Self::extract_symbols(prompt);

        // 3. 提取否定约束与保护路径 (特别遵循不改动 common 铁律)
        let mut constraints = Self::extract_constraints(&normalized);
        if CurrentRequestStage::classify(prompt) != CurrentRequestStage::Execution { constraints.read_only_mode = true; }
        if constraints.read_only_mode {
            goal = if goal == ActionGoal::ArchitectureExploration {
                goal
            } else {
                ActionGoal::TargetedInvestigation
            };
        }

        // 4. 评估模糊度与信息鸿沟
        let ambiguity_score =
            Self::calculate_ambiguity(&normalized, goal, &target_service, &target_symbols);

        // 5. 进行自我能力盘点与主动反问点生成
        let capability =
            Self::assess_capabilities(&normalized, goal, ambiguity_score, &target_service);

        DynamicIntent {
            raw_prompt: prompt.to_string(),
            goal,
            target_service,
            target_symbols,
            constraints,
            capability,
            ambiguity_score,
        }
    }

    fn resolve_goal(norm: &str) -> ActionGoal {
        let feasibility_question = contains_any(norm,
            &["是否可以", "是否能", "能否", "可不可以", "可行性", "is it possible", "can this"])
            && contains_any(norm, &["看看", "分析", "评估", "检查", "可行性", "is it possible", "can this"]);
        let requests_delivery = contains_any(norm, &[
            "帮我生成", "请生成", "直接生成", "并生成", "再生成", "然后生成",
            "帮我写", "请编写", "直接写", "并实现", "再实现", "然后实现",
            "并修复", "再修复", "然后修复", "请修复", "帮我修复", "直接转换",
            "then implement", "then generate", "please generate", "please create", "please fix",
        ]);
        if feasibility_question && !requests_delivery {
            ActionGoal::TargetedInvestigation
        } else if contains_affirmative(
            norm,
            &[
                "重构",
                "优化架构",
                "解耦",
                "refactor",
                "modernize",
                "restructure",
            ],
        ) {
            ActionGoal::RefactoringModernization
        } else if contains_affirmative(
            norm,
            &[
                "实现",
                "编写",
                "生成",
                "转换",
                "转化",
                "修改",
                "修复",
                "添加",
                "新增",
                "开发",
                "弄",
                "做",
                "搞",
                "建",
                "写",
                "进化",
                "提升",
                "提高",
                "完善",
                "优化",
                "删除",
                "移植",
                "更新",
                "implement",
                "fix",
                "add",
                "create",
                "generate",
                "convert",
                "write",
                "build",
                "improve",
                "evolve",
                "edit",
                "change",
                "update",
                "remove",
                "migrate",
            ],
        ) {
            ActionGoal::SurgicalExecution
        } else if contains_affirmative(
            norm,
            &[
                "架构",
                "画像",
                "拓扑",
                "依赖关系",
                "architecture",
                "overview",
                "topology",
                "modules",
            ],
        ) {
            ActionGoal::ArchitectureExploration
        } else if contains_affirmative(
            norm,
            &[
                "排查",
                "调查",
                "诊断",
                "报错",
                "堆栈",
                "分析",
                "检查",
                "查看",
                "解释",
                "investigate",
                "diagnose",
                "debug",
                "trace",
                "analyze",
                "inspect",
                "explain",
                "review",
            ],
        ) {
            ActionGoal::TargetedInvestigation
        } else if contains_affirmative(
            norm,
            &[
                "测试",
                "单测",
                "回归",
                "benchmark",
                "test",
                "tests",
                "verify",
                "f2p",
                "p2p",
            ],
        ) {
            ActionGoal::VerificationBenchmark
        } else {
            ActionGoal::ClarificationDialogue
        }
    }

    fn extract_target_service(norm: &str) -> Option<String> {
        // A generic word such as "model" is not evidence of a model-service.
        norm.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .find(|word| word.len() > "-service".len() && word.ends_with("-service"))
            .map(str::to_string)
    }

    fn extract_symbols(prompt: &str) -> Vec<String> {
        let mut symbols = Vec::new();
        // 简单提取类似 PascalCase 或 camelCase 的代码标识符
        for word in prompt.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if clean.len() >= 3
                && (clean.chars().next().map_or(false, |c| c.is_uppercase())
                    || clean.contains('_')
                    || clean.ends_with(".java")
                    || clean.ends_with(".rs")
                    || clean.ends_with(".ts"))
            {
                if !["Java", "Rust", "Spring", "Redis", "MySQL"].contains(&clean) {
                    symbols.push(clean.to_string());
                }
            }
        }
        symbols
    }

    fn extract_constraints(norm: &str) -> NegativeConstraint {
        let mut protected_paths = Vec::new();
        let mut forbidden_patterns = Vec::new();

        // 识别否定约束：如“不要改动 common 内容”、“别动 common”、“禁止修改 common”
        let mentions_no_common = contains_any(
            norm,
            &[
                "不要动common",
                "不要改动common",
                "先不要改动common",
                "别动common",
                "禁止改动common",
                "免动common",
                "no common",
                "dont touch common",
            ],
        );
        if mentions_no_common
            || norm.contains("common")
                && contains_any(norm, &["不要", "别", "禁止", "不改", "不能"])
        {
            protected_paths.push("common/".to_string());
            protected_paths.push("common".to_string());
        }

        if contains_any(norm, &["不要jpa", "禁止jpa", "不用jpa", "no jpa"]) {
            forbidden_patterns.push("JPA/Hibernate".to_string());
        }

        let read_only_mode = contains_any(
            norm,
            &[
                "只读",
                "只查不改",
                "只看不动",
                "只分析",
                "read only",
                "read-only",
                "inspect only",
            ],
        );

        NegativeConstraint {
            protected_paths,
            forbidden_patterns,
            read_only_mode,
        }
    }

    fn calculate_ambiguity(
        norm: &str,
        goal: ActionGoal,
        target_service: &Option<String>,
        symbols: &[String],
    ) -> f32 {
        let mut score: f32 = 0.0;

        // 若为执行类任务但字数较少，且未指明微服务或具体文件
        if matches!(
            goal,
            ActionGoal::SurgicalExecution | ActionGoal::RefactoringModernization
        ) {
            if target_service.is_none()
                && symbols.is_empty()
                && !norm.contains('/')
                && !norm.contains('\\')
            {
                score += 0.4;
            }
            if norm.chars().count() < 25 {
                score += 0.3;
            }
            // 包含明显的模糊词
            if contains_any(
                norm,
                &[
                    "看看",
                    "随便",
                    "做个",
                    "弄个",
                    "处理一下",
                    "优化一下",
                    "随便写写",
                ],
            ) {
                score += 0.3;
            }
        }

        score.min(1.0)
    }

    fn assess_capabilities(
        norm: &str,
        goal: ActionGoal,
        ambiguity_score: f32,
        target_service: &Option<String>,
    ) -> CapabilityAssessment {
        let inspection = vec![
            "project_index".to_string(),
            "search_graph".to_string(),
            "view_outline".to_string(),
            "grep_search".to_string(),
        ];

        let mut mutation = Vec::new();
        let is_execution = matches!(
            goal,
            ActionGoal::SurgicalExecution | ActionGoal::RefactoringModernization
        );
        if is_execution {
            mutation.push("replace_in_file".to_string());
            mutation.push("multi_replace_file_content".to_string());
            mutation.push("write_to_file".to_string());
        }

        let verification = vec!["verify_task_tool".to_string(), "run_command".to_string()];

        let requires_user_inquiry = is_execution && ambiguity_score >= 0.5;
        let mut recommended_inquiries = Vec::new();

        if requires_user_inquiry {
            if target_service.is_none() {
                recommended_inquiries.push("先检查当前工作区，识别相关模块与现有约定；仅在仍有影响结果的分支时，询问用户期望的行为和范围。".to_string());
            }
            if matches!(goal, ActionGoal::SurgicalExecution) {
                recommended_inquiries.push(
                    "确认可观察的验收条件、兼容要求，以及现有测试能否覆盖目标行为。".to_string(),
                );
            }
        }

        CapabilityAssessment {
            suggested_inspection_tools: inspection,
            suggested_mutation_tools: mutation,
            suggested_verification_tools: verification,
            requires_user_inquiry,
            recommended_inquiries,
        }
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| {
        value.match_indices(needle).any(|(start, _)| {
            if !needle.is_ascii() {
                return true;
            }
            let end = start + needle.len();
            let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
            !value[..start].chars().next_back().is_some_and(is_word)
                && !value[end..].chars().next().is_some_and(is_word)
        })
    })
}

// Advisory verb cues must not turn a restriction into an instruction to act.
fn contains_affirmative(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.match_indices(needle).any(|(start, _)| {
        if needle.is_ascii() {
            let end=start+needle.len();let word=|c:char|c.is_ascii_alphanumeric()||c=='_';
            if value[..start].chars().next_back().is_some_and(word)||value[end..].chars().next().is_some_and(word){return false;}
        }
        let prefix=value[..start].rsplit(|c:char|matches!(c,','|'，'|'.'|'。'|';'|'；'|'!'|'！'|'?'|'？'|'\n')).next().unwrap_or("").trim_end();
        let mut words:Vec<_>=prefix.split_whitespace().collect();
        while words.last().is_some_and(|w|["to","just","also","automatically","directly","ever","again","immediately"].contains(w)){words.pop();}
        if words.last().is_some_and(|w|["not","never","don't","cannot","can't","without","no","neither","nor"].contains(w)){return false;}
        for negative in ["不要","不必","无需","禁止","避免","没有","不再","别","不","未"] {
            for (at,_) in prefix.match_indices(negative) {
                let mut tail=prefix[at+negative.len()..].trim_start();
                loop {let next=["再","继续","额外","主动","自行","直接","去","急着","进行","另外","马上","立即","自动","尝试","只","仅"].iter().find_map(|m|tail.strip_prefix(m));if let Some(rest)=next{tail=rest.trim_start();}else{break;}}
                if tail.is_empty()||["创","构","新","编","改","填","覆","重"].contains(&tail){return false;}
            }
        }
        true
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_only_choice_request_does_not_require_implementation() {
        for text in ["请用ask_user问我选择A还是B，等我回答后简短确认，不做其他事情", "先别急着做，只问我选哪个", "只问我一个问题，不要立即编写文件"] {
            assert_eq!(DynamicIntentEngine::analyze(text).goal,ActionGoal::ClarificationDialogue,"{}",text);
        }
    }
    #[test]
    fn english_negative_verbs_are_not_positive_action_requests() {
        for text in ["Ask whether I prefer A or B; do not create files", "Ask my choice, never refactor", "Ask my choice; don't automatically write anything"] {
            assert_eq!(DynamicIntentEngine::analyze(text).goal,ActionGoal::ClarificationDialogue,"{}",text);
        }
    }
    #[test]
    fn an_affirmative_request_survives_an_unrelated_restriction() {
        for text in ["不要重构已有模块，请修复边界错误", "不要编写测试脚手架，请实现新功能", "Do not refactor; implement the requested feature"] {
            assert_eq!(DynamicIntentEngine::analyze(text).goal,ActionGoal::SurgicalExecution,"{}",text);
        }
    }
    #[test]
    fn restriction_and_investigation_keep_their_separate_roles() {
        assert_eq!(DynamicIntentEngine::analyze("不要立即编写代码，先解释现有行为").goal,ActionGoal::TargetedInvestigation);
    }

    #[test]
    fn routes_real_objectives_without_assuming_a_framework() {
        for (prompt, expected) in [
            (
                "进化superagent，对比开源的智能体，现在superagent不够智能",
                ActionGoal::SurgicalExecution,
            ),
            ("继续提升这个项目", ActionGoal::SurgicalExecution),
            ("分析当前源码", ActionGoal::TargetedInvestigation),
            (r"D:\太极\动漫共和国概念版帮我看看，我安装的这个应用，是否可以转化为海阔世界的一个可用规则", ActionGoal::TargetedInvestigation),
            ("评估能否转为海阔规则，然后生成规则并验证", ActionGoal::SurgicalExecution),
            ("根据skill生成海阔规则", ActionGoal::SurgicalExecution),
            ("能否帮我生成一个海阔规则", ActionGoal::SurgicalExecution),
            ("解释这段代码", ActionGoal::TargetedInvestigation),
            (
                "implement the architecture described in docs/design.md",
                ActionGoal::SurgicalExecution,
            ),
            (
                "只读分析代码，给出修复建议",
                ActionGoal::TargetedInvestigation,
            ),
            ("run the tests", ActionGoal::VerificationBenchmark),
            ("add tests for AuthHandler", ActionGoal::SurgicalExecution),
            ("tell me the latest news", ActionGoal::ClarificationDialogue),
            ("你好", ActionGoal::ClarificationDialogue),
        ] {
            let intent = DynamicIntentEngine::analyze(prompt);
            assert_eq!(intent.goal, expected, "{prompt}");
            let guidance = intent.capability.recommended_inquiries.join(" ");
            assert!(!guidance.contains("BaseController"));
            assert!(!guidance.contains("Result<T>"));
            if matches!(
                expected,
                ActionGoal::TargetedInvestigation | ActionGoal::ClarificationDialogue
            ) {
                assert!(
                    intent.capability.suggested_mutation_tools.is_empty(),
                    "{prompt}"
                );
            }
        }
    }

    #[test]
    fn concrete_targets_do_not_need_an_invented_microservice() {
        let intent = DynamicIntentEngine::analyze("修复 src/model.rs 中的空指针错误");
        assert!(intent.target_service.is_none());
        assert!(!intent.capability.requires_user_inquiry);
        assert_eq!(
            DynamicIntentEngine::analyze("修改 billing-service 的接口")
                .target_service
                .as_deref(),
            Some("billing-service")
        );
        assert!(
            !DynamicIntentEngine::analyze("你好")
                .capability
                .requires_user_inquiry
        );
    }

    #[test]
    fn test_extracts_negative_constraints_for_common() {
        let prompt = "请帮我重构一下用户鉴权逻辑，但先不要改动 common 内容，严格测试";
        let intent = DynamicIntentEngine::analyze(prompt);

        assert_eq!(intent.goal, ActionGoal::RefactoringModernization);
        assert!(
            intent
                .constraints
                .protected_paths
                .iter()
                .any(|p| p.contains("common"))
        );
    }

    #[test]
    fn test_detects_ambiguity_and_proactive_inquiry() {
        let prompt = "帮我弄个通知推送接口";
        let intent = DynamicIntentEngine::analyze(prompt);

        assert_eq!(intent.goal, ActionGoal::SurgicalExecution);
        assert!(intent.ambiguity_score >= 0.5);
        assert!(intent.capability.requires_user_inquiry);
        assert!(!intent.capability.recommended_inquiries.is_empty());
    }

    #[test]
    fn test_maps_architecture_exploration_capabilities() {
        let prompt = "分析一下项目的整体架构与模块依赖画像";
        let intent = DynamicIntentEngine::analyze(prompt);

        assert_eq!(intent.goal, ActionGoal::ArchitectureExploration);
        assert!(
            intent
                .capability
                .suggested_inspection_tools
                .contains(&"project_index".to_string())
        );
    }

    #[test]
    fn deferred_proposals_are_current_phase_even_after_problem_description() {
        for prompt in [
            "当前代码报错。请先调查，给出修复方案，等我确认后再修改。",
            "这个接口失败，请先给两个可选修复方案，选定后再实施。",
            "First inspect the current module and propose options; wait for my approval before modifying it.",
        ] {
            assert_eq!(CurrentRequestStage::classify(prompt), CurrentRequestStage::ProposalOnly, "{prompt}");
            assert!(DynamicIntentEngine::analyze(prompt).constraints.read_only_mode);
        }
    }

    #[test]
    fn quoted_phases_and_explicit_immediate_execution_are_not_deferred() {
        for prompt in [
            "请修改代码中的字符串“先给方案，确认后再改”，替换成“执行完成”。",
            "请修复当前代码。示例：```先给方案，等我确认后再改```",
            "请先给方案，无需等我确认，直接实施并验证。",
            "方案已确认，现在按方案修改代码并测试。",
        ] {
            assert_eq!(CurrentRequestStage::classify(prompt), CurrentRequestStage::Execution, "{prompt}");
        }
    }

    #[test]
    fn final_global_no_change_clause_is_read_only_but_scoped_or_quoted_text_is_not() {
        for prompt in ["调查本项目，不要修改文件。","PARENT_CONTROL_HOLD 调查本项目，不要修改文件。","请调查调用链；请不要改动任何代码！"] {
            assert_eq!(CurrentRequestStage::classify(prompt),CurrentRequestStage::InvestigationOnly,"{prompt}");
            assert!(DynamicIntentEngine::analyze(prompt).constraints.read_only_mode);
        }
        for prompt in ["请修复A，不要修改文件B。","修改invoice.js，不要改动common。","请修改代码中的字符串“不要修改文件”。","请修复代码：```不要修改文件```","不要修改文件；现在修改invoice.js并测试。"] {
            assert_eq!(CurrentRequestStage::classify(prompt),CurrentRequestStage::Execution,"{prompt}");
        }
    }
}
