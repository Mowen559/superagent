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
}
