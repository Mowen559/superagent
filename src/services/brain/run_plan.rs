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
    requires_action: bool,
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
            requires_action: kind == TaskKind::Execution,
            steps,
            consecutive_failures: 0,
            replans: 0,
            replan_notice: None,
        }
    }

    pub fn for_verification() -> Self {
        let mut plan = Self::for_task(TaskKind::Execution);
        plan.requires_action = false;
        plan.steps.retain(|step| step.id != "act");
        plan
    }

    pub fn observe_tool(
        &mut self,
        status: ToolExecutionStatus,
        risk: ToolRisk,
        verification: bool,
    ) {
        // A check belongs to the workspace state it verified. Further writes,
        // including potentially partial failed writes, invalidate that evidence.
        if (matches!(risk, ToolRisk::WorkspaceMutation | ToolRisk::Destructive)
            && matches!(
                status,
                ToolExecutionStatus::Succeeded
                    | ToolExecutionStatus::Failed
                    | ToolExecutionStatus::Panicked
            ))
            || (risk == ToolRisk::ProcessControl
                && matches!(
                    status,
                    ToolExecutionStatus::Failed | ToolExecutionStatus::Panicked
                ))
        {
            self.invalidate_verification();
        }
        if status == ToolExecutionStatus::Succeeded {
            self.consecutive_failures = 0;
            self.advance_success(risk, verification);
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

    pub(crate) fn invalidate_verification(&mut self) {
        for step in &mut self.steps {
            if step.id == "verify" || step.id == "respond" {
                step.status = PlanStepStatus::Pending;
            }
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
                    && (!self.requires_action || self.is_completed("act"))
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

    fn advance_success(&mut self, risk: ToolRisk, verification: bool) {
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
                if verification && (!self.requires_action || action_already_complete) {
                    self.complete_and_advance("inspect", Some("verify"));
                    self.complete_and_advance("verify", Some("respond"));
                    return;
                }
                if matches!(risk, ToolRisk::ReadOnly | ToolRisk::Network) {
                    if !inspect_complete {
                        self.complete_and_advance(
                            "inspect",
                            Some(if action_already_complete || !self.requires_action {
                                "verify"
                            } else {
                                "act"
                            }),
                        );
                    }
                } else if matches!(risk, ToolRisk::WorkspaceMutation | ToolRisk::Destructive) {
                    // 【物理落盘铁律】：仅当真实发生工作区写盘(WorkspaceMutation)时，才能推进 act！
                    if !inspect_complete {
                        self.replan_notice = Some(
                            "The action happened before inspection. Inspect the resulting state now, verify the change, and report the missing pre-change evidence as residual risk."
                                .to_string(),
                        );
                    }
                    self.complete_and_advance("act", Some("verify"));
                } else if risk == ToolRisk::ProcessControl {
                    if !inspect_complete {
                        self.complete_and_advance(
                            "inspect",
                            Some(if self.requires_action {
                                "act"
                            } else {
                                "verify"
                            }),
                        );
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
    fn reading_or_starting_a_process_does_not_verify_a_change() {
        let mut plan = RunPlan::for_task(TaskKind::Execution);
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::WorkspaceMutation,
            false,
        );
        for risk in [
            ToolRisk::ReadOnly,
            ToolRisk::Network,
            ToolRisk::ProcessControl,
        ] {
            plan.observe_tool(ToolExecutionStatus::Succeeded, risk, false);
            plan.begin_completion();
            assert_eq!(plan.completion_gap(), Some("Verify the changed behavior"));
        }
    }

    #[test]
    fn writes_and_failed_checks_invalidate_prior_verification() {
        for (status, risk) in [
            (ToolExecutionStatus::Succeeded, ToolRisk::WorkspaceMutation),
            (ToolExecutionStatus::Failed, ToolRisk::WorkspaceMutation),
            (ToolExecutionStatus::Failed, ToolRisk::ProcessControl),
        ] {
            let mut plan = RunPlan::for_task(TaskKind::Execution);
            plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
            plan.observe_tool(
                ToolExecutionStatus::Succeeded,
                ToolRisk::WorkspaceMutation,
                false,
            );
            plan.observe_tool(
                ToolExecutionStatus::Succeeded,
                ToolRisk::ProcessControl,
                true,
            );
            plan.begin_completion();
            assert!(plan.completion_gap().is_none());
            plan.observe_tool(status, risk, false);
            plan.begin_completion();
            assert_eq!(plan.completion_gap(), Some("Verify the changed behavior"));
        }
    }

    #[test]
    fn verification_objective_needs_no_unrequested_mutation() {
        let mut plan = RunPlan::for_verification();
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::ProcessControl,
            true,
        );
        plan.begin_completion();
        assert!(plan.completion_gap().is_none());
    }

    #[test]
    fn execution_plan_requires_inspect_act_verify_and_response() {
        let mut plan = RunPlan::for_task(TaskKind::Execution);
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::WorkspaceMutation,
            false,
        );
        assert_eq!(plan.completion_gap(), Some("Verify the changed behavior"));
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::ProcessControl,
            true,
        );
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
        plan.observe_tool(ToolExecutionStatus::Failed, ToolRisk::ReadOnly, false);
        assert!(plan.take_replan_notice().is_none());
        plan.observe_tool(ToolExecutionStatus::NotFound, ToolRisk::Unclassified, false);
        assert!(plan.take_replan_notice().unwrap().contains("Replan #1"));
        assert!(plan.take_replan_notice().is_none());
    }

    #[test]
    fn action_before_inspection_recovers_with_explicit_warning() {
        let mut plan = RunPlan::for_task(TaskKind::Execution);
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::WorkspaceMutation,
            false,
        );
        assert!(
            plan.take_replan_notice()
                .unwrap()
                .contains("before inspection")
        );
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        plan.begin_completion();
        assert_eq!(plan.completion_gap(), Some("Verify the changed behavior"));
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::ProcessControl,
            true,
        );
        plan.begin_completion();
        assert_eq!(plan.completion_gap(), None);
    }

    #[test]
    fn process_control_cannot_advance_act_without_mutation() {
        let mut plan = RunPlan::for_task(TaskKind::Execution);
        // 先跑只读探索
        plan.observe_tool(ToolExecutionStatus::Succeeded, ToolRisk::ReadOnly, false);
        // 跑了一个命令（例如 mvn test 或 echo），由于未发生任何文件写盘，act 绝不能被推进为 completed！
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::ProcessControl,
            true,
        );
        assert_eq!(plan.completion_gap(), Some("Apply the coherent change"));

        // 只有真实发生 WorkspaceMutation 时，act 才被推进！
        plan.observe_tool(
            ToolExecutionStatus::Succeeded,
            ToolRisk::WorkspaceMutation,
            false,
        );
        assert_eq!(plan.completion_gap(), Some("Verify the changed behavior"));
    }
}
