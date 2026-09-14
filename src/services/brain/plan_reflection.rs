use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub index: usize,
    pub description: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub goal: String,
    pub steps: Vec<PlanStep>,
    pub current_step_index: usize,
}

impl TaskPlan {
    pub fn new(goal: impl Into<String>, steps: Vec<String>) -> Self {
        let plan_steps = steps
            .into_iter()
            .enumerate()
            .map(|(i, desc)| PlanStep {
                index: i + 1,
                description: desc,
                status: if i == 0 { StepStatus::InProgress } else { StepStatus::Pending },
            })
            .collect();

        Self {
            goal: goal.into(),
            steps: plan_steps,
            current_step_index: 0,
        }
    }

    pub fn update_status(&mut self, step_index: usize, status: StepStatus) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.index == step_index) {
            step.status = status.clone();
            if matches!(status, StepStatus::Completed) {
                if let Some(next_step) = self.steps.iter_mut().find(|s| s.index == step_index + 1) {
                    if next_step.status == StepStatus::Pending {
                        next_step.status = StepStatus::InProgress;
                    }
                }
                self.current_step_index = step_index;
            }
        }
    }

    pub fn render_prompt(&self) -> String {
        let mut out = format!("[Execution Plan]\nGoal: {}\nSteps:\n", self.goal);
        for step in &self.steps {
            let mark = match &step.status {
                StepStatus::Pending => "[ ]",
                StepStatus::InProgress => "[>]",
                StepStatus::Completed => "[x]",
                StepStatus::Failed(reason) => return format!("{}! Step {} Failed: {}\n", out, step.index, reason),
            };
            out.push_str(&format!("{} Step {}: {}\n", mark, step.index, step.description));
        }
        out
    }
}

pub struct ReflectionEngine;

impl ReflectionEngine {
    pub fn build_stuck_reflection_prompt(reason_desc: &str, suggested_action: &str) -> String {
        format!(
            "[SYSTEM REFLECTION INSTRUCTION]\n\
            Attention SuperAgent: A execution stuck condition was detected!\n\
            - Cause: {}\n\
            - Recommendation: {}\n\n\
            Please pause your current loop, perform self-reflection (Critic), re-evaluate your assumptions, and state your revised strategy before taking the next tool action.",
            reason_desc, suggested_action
        )
    }

    pub fn build_critic_evaluation_prompt(goal: &str, plan: &TaskPlan, observations: &[String]) -> String {
        let obs_summary = observations.join("\n- ");
        format!(
            "[CRITIC EVALUATION]\n\
            Goal: {}\n\
            Current Plan Progress: Step {}/{}\n\
            Recent Observations:\n- {}\n\n\
            Evaluate if recent actions meaningfully moved towards the goal. If blocked, adjust the step descriptions.",
            goal, plan.current_step_index + 1, plan.steps.len(), obs_summary
        )
    }

    /// 分析 Agent 执行轨迹，针对发生的错误、语法诊断及最终修复过程，自动蒸馏出工程经验准则
    pub fn distill_trajectory_lessons(
        events: &[crate::services::brain::events::SessionEvent],
    ) -> Vec<DistilledLesson> {
        use crate::services::brain::events::SessionEvent;
        let mut lessons = Vec::new();

        // 1. 扫描行级编译器语法错误或工具报错
        for event in events {
            if let SessionEvent::ToolResult { id: _, result } = event {
                // 检查行级诊断
                if let Some(diags) = result.get("diagnostics").and_then(|d| d.as_array()) {
                    for diag in diags {
                        if diag.get("severity").and_then(|s| s.as_str()) == Some("error") {
                            let msg = diag
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("Syntax error");
                            let line = diag.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
                            lessons.push(DistilledLesson {
                                title: "Delimited Syntax & Compiler Guard".to_string(),
                                trigger_pattern: format!("Compiler error at line {}: {}", line, msg),
                                rule_guidance: format!(
                                    "When editing files, strictly ensure delimiter balance and syntax completeness around line {}. Use replace_in_file with exact context matches.",
                                    line
                                ),
                                severity: "warning".to_string(),
                            });
                        }
                    }
                }

                // 检查越界或路径安全错误
                if let Some(err) = result.as_str() {
                    if err.contains("Security Error") && err.contains("outside the workspace") {
                        lessons.push(DistilledLesson {
                            title: "Workspace Boundary Enforcement".to_string(),
                            trigger_pattern: "Security Error: path outside the workspace".to_string(),
                            rule_guidance: "All operations must use paths within the current workspace root. Avoid ../ traversals outside target directories.".to_string(),
                            severity: "blocker".to_string(),
                        });
                    }
                }
            }
        }

        // 去重
        lessons.dedup_by(|a, b| a.title == b.title && a.trigger_pattern == b.trigger_pattern);
        lessons
    }

    /// 将蒸馏后的经验准则持久化沉淀到超级记忆海马体 (SupermemoryStore)
    pub fn distill_and_persist(
        events: &[crate::services::brain::events::SessionEvent],
        config: &crate::config::runtime::RuntimeConfig,
        project_path: Option<&str>,
    ) -> usize {
        let lessons = Self::distill_trajectory_lessons(events);
        if lessons.is_empty() {
            return 0;
        }

        let store =
            crate::services::supermemory_store::SupermemoryStore::new(config, project_path);
        let count = lessons.len();

        for lesson in lessons {
            let record_id = format!("distilled_{}", uuid::Uuid::new_v4().simple());
            let record = crate::models::memory::SupermemoryRecord {
                id: record_id,
                name: lesson.title.clone(),
                summary: format!(
                    "Pattern: {}\nGuideline: {}",
                    lesson.trigger_pattern, lesson.rule_guidance
                ),
                tags: vec!["distilled_rule".to_string(), "reflexion".to_string()],
                file_path: "".to_string(),
                complexity: lesson.severity,
                node_type: "reflexion".to_string(),
                source_graph_path: "".to_string(),
                source_label: "PostMortemReflexion".to_string(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                embedding: None,
            };
            store.write_memory(record);
            eprintln!(
                "\x1b[32m[Reflexion Engine] 🧬 Distilled & persisted rule: {}\x1b[0m",
                lesson.title
            );
        }

        count
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistilledLesson {
    pub title: String,
    pub trigger_pattern: String,
    pub rule_guidance: String,
    pub severity: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_plan_workflow() {
        let mut plan = TaskPlan::new(
            "Fix bug in auth",
            vec![
                "Search codebase for auth".to_string(),
                "Edit auth file".to_string(),
                "Run test".to_string(),
            ],
        );

        assert_eq!(plan.steps[0].status, StepStatus::InProgress);
        assert_eq!(plan.steps[1].status, StepStatus::Pending);

        plan.update_status(1, StepStatus::Completed);
        assert_eq!(plan.steps[0].status, StepStatus::Completed);
        assert_eq!(plan.steps[1].status, StepStatus::InProgress);

        let prompt = plan.render_prompt();
        assert!(prompt.contains("[x] Step 1"));
        assert!(prompt.contains("[>] Step 2"));
    }

    #[test]
    fn test_reflection_prompt() {
        let prompt = ReflectionEngine::build_stuck_reflection_prompt(
            "Repeated tool failure",
            "Change command arguments",
        );
        assert!(prompt.contains("SYSTEM REFLECTION INSTRUCTION"));
        assert!(prompt.contains("Repeated tool failure"));
    }

    #[test]
    fn test_distill_trajectory_lessons() {
        use crate::services::brain::events::SessionEvent;
        use serde_json::json;

        let events = vec![
            SessionEvent::ToolCall {
                id: "call_1".to_string(),
                name: "replace_in_file".to_string(),
                arguments: json!({ "path": "test.rs" }),
            },
            SessionEvent::ToolResult {
                id: "call_1".to_string(),
                result: json!({
                    "diagnostics": [
                        { "severity": "error", "line": 42, "message": "unclosed delimiter" }
                    ]
                }),
            },
            SessionEvent::ToolCall {
                id: "call_2".to_string(),
                name: "replace_in_file".to_string(),
                arguments: json!({ "path": "test.rs" }),
            },
            SessionEvent::ToolResult {
                id: "call_2".to_string(),
                result: json!({
                    "success": true,
                    "diagnostics": []
                }),
            },
        ];

        let lessons = ReflectionEngine::distill_trajectory_lessons(&events);
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].title, "Delimited Syntax & Compiler Guard");
        assert!(lessons[0].trigger_pattern.contains("42"));
        assert!(lessons[0].rule_guidance.contains("delimiter balance"));
    }
}
