use crate::services::brain::tool_execution_gateway::ToolExecutionStatus;
use crate::services::brain::tool_policy::ToolRisk;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Conversational,
    Investigative,
    Execution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub id: &'static str,
    pub title: &'static str,
    pub status: PlanStepStatus,
    pub failed_attempts: usize,
}

/// Deterministic Run plan advanced only by normalized execution receipts.
/// Model prose cannot directly mark evidence, action, or verification complete.
#[derive(Debug)]
pub struct RunPlan {
    kind: TaskKind,
    steps: Vec<PlanStep>,
    consecutive_failures: usize,
    replans: usize,
    replan_notice: Option<String>,
}

impl RunPlan {
    pub fn for_task(kind: TaskKind) -> Self {
        let steps = match kind {
            TaskKind::Conversational => vec![step("respond", "Answer the objective", true)],
            TaskKind::Investigative => vec![
                step("inspect", "Gather authoritative evidence", true),
                step("synthesize", "Test hypotheses against evidence", false),
                step("respond", "Deliver evidence-backed conclusion", false),
            ],
            TaskKind::Execution => vec![
                step("inspect", "Inspect the relevant state", true),
                step("act", "Apply the coherent change", false),
                step("verify", "Verify the changed behavior", false),
                step("respond", "Report outcome and residual risk", false),
            ],
        };
        Self {
            kind,
            steps,
            consecutive_failures: 0,
            replans: 0,
            replan_notice: None,
        }
    }

    pub fn observe_tool(&mut self, status: ToolExecutionStatus, risk: ToolRisk) {
        if status == ToolExecutionStatus::Succeeded {
            self.consecutive_failures = 0;
            self.advance_success(risk);
            return;
        }
        if !is_execution_failure(status) {
            return;
        }

        self.consecutive_failures += 1;
        if let Some(step) = self
            .steps
            .iter_mut()
            .find(|step| step.status == PlanStepStatus::InProgress)
        {
            step.failed_attempts += 1;
        }
        if self.consecutive_failures >= 2 {
            self.replans += 1;
            self.replan_notice = Some(format!(
                "Replan #{} required after two consecutive execution failures. Re-check assumptions, use a different source or tool, and preserve already verified facts.",
                self.replans
            ));
            self.consecutive_failures = 0;
        }
    }

    pub fn begin_completion(&mut self) {
        match self.kind {
            TaskKind::Conversational => self.complete_and_advance("respond", None),
            TaskKind::Investigative => {
                if self.is_completed("inspect") {
                    self.complete_and_advance("synthesize", Some("respond"));
                    self.complete_and_advance("respond", None);
                }
            }
            TaskKind::Execution => {
                if self.is_completed("inspect")
                    && self.is_completed("act")
                    && self.is_completed("verify")
                {
                    self.complete_and_advance("respond", None);
                }
            }
        }
    }

    pub fn completion_gap(&self) -> Option<&'static str> {
        self.steps
            .iter()
            .find(|step| step.status != PlanStepStatus::Completed)
            .map(|step| step.title)
    }

    pub fn take_replan_notice(&mut self) -> Option<String> {
        self.replan_notice.take()
    }

    pub fn replan_count(&self) -> usize {
        self.replans
    }

    pub fn render(&self) -> String {
        self.steps
            .iter()
            .map(|step| {
                format!(
                    "- [{}] {}{}",
                    match step.status {
                        PlanStepStatus::Pending => "pending",
                        PlanStepStatus::InProgress => "in_progress",
                        PlanStepStatus::Completed => "completed",
                    },
                    step.title,
                    if step.failed_attempts > 0 {
                        format!(" (failed attempts: {})", step.failed_attempts)
                    } else {
                        String::new()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn advance_success(&mut self, risk: ToolRisk) {
        match self.kind {
            TaskKind::Conversational => {}
            TaskKind::Investigative => {
                if matches!(risk, ToolRisk::ReadOnly | ToolRisk::Network) {
                    self.complete_and_advance("inspect", Some("synthesize"));
                }
            }
            TaskKind::Execution => {
                let inspect_complete = self.is_completed("inspect");
                let action_already_complete = self.is_completed("act");
                if matches!(risk, ToolRisk::ReadOnly | ToolRisk::Network) {
                    if !inspect_complete {
                        self.complete_and_advance(
                            "inspect",
                            Some(if action_already_complete {
                                "verify"
                            } else {
                                "act"
                            }),
                        );
                        if action_already_complete {
                            self.complete_and_advance("verify", Some("respond"));
                        }
                    } else if action_already_complete {
                        self.complete_and_advance("verify", Some("respond"));
                    }
                } else if matches!(
                    risk,
                    ToolRisk::WorkspaceMutation | ToolRisk::ProcessControl | ToolRisk::Destructive
                ) {
                    if action_already_complete && risk == ToolRisk::ProcessControl {
                        self.complete_and_advance("verify", Some("respond"));
                    } else {
                        if !inspect_complete {
                            self.replan_notice = Some(
                                "The action happened before inspection. Inspect the resulting state now, verify the change, and report the missing pre-change evidence as residual risk."
                                    .to_string(),
                            );
                        }
                        self.complete_and_advance("act", Some("verify"));
                    }
                }
            }
        }
    }

    fn complete_and_advance(&mut self, completed: &str, next: Option<&str>) {
        if let Some(step) = self.steps.iter_mut().find(|step| step.id == completed) {
            step.status = PlanStepStatus::Completed;
        }
        if let Some(next) = next {
            if let Some(step) = self.steps.iter_mut().find(|step| step.id == next) {
                if step.status == PlanStepStatus::Pending {
                    step.status = PlanStepStatus::InProgress;
                }
            }
        }
    }

    fn is_completed(&self, id: &str) -> bool {
        self.steps
            .iter()
            .find(|step| step.id == id)
            .map(|step| step.status == PlanStepStatus::Completed)
            .unwrap_or(false)
    }
}

fn step(id: &'static str, title: &'static str, active: bool) -> PlanStep {
    PlanStep {
        id,
        title,
        status: if active {
            PlanStepStatus::InProgress
        } else {
            PlanStepStatus::Pending
        },
        failed_attempts: 0,
    }
}

fn is_execution_failure(status: ToolExecutionStatus) -> bool {
    matches!(
        status,
        ToolExecutionStatus::Failed
            | ToolExecutionStatus::Panicked
            | ToolExecutionStatus::NotFound
            | ToolExecutionStatus::PolicyDenied
            | ToolExecutionStatus::RepetitionBlocked
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_plan_requires_inspect_act_verify_and_response() {
        let mut plan = RunPlan::for_task(TaskKind::Execution);
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly);
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation);
        assert_eq!(plan.completion_gap(), Some("Verify the changed behavior"));
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ProcessControl);
        plan.begin_completion();
        assert_eq!(plan.completion_gap(), None);
    }

    #[test]
    fn model_draft_cannot_replace_missing_evidence() {
        let mut plan = RunPlan::for_task(TaskKind::Investigative);
        plan.begin_completion();
        assert_eq!(plan.completion_gap(), Some("Gather authoritative evidence"));
    }

    #[test]
    fn two_failures_emit_one_replan_notice() {
        let mut plan = RunPlan::for_task(TaskKind::Execution);
        plan.observe_tool(ToolExecutionStatus::Failed, ToolRisk::ReadOnly);
        assert!(plan.take_replan_notice().is_none());
        plan.observe_tool(ToolExecutionStatus::NotFound, ToolRisk::Unclassified);
        assert!(plan.take_replan_notice().unwrap().contains("Replan #1"));
        assert!(plan.take_replan_notice().is_none());
    }

    #[test]
    fn action_before_inspection_recovers_with_explicit_warning() {
        let mut plan = RunPlan::for_task(TaskKind::Execution);
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation);
        assert!(plan
            .take_replan_notice()
            .unwrap()
            .contains("before inspection"));
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly);
        plan.begin_completion();
        assert_eq!(plan.completion_gap(), None);
    }
}
