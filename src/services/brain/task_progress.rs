//! Run-scoped subgoal evidence and context-gap resolution. Model prose cannot
//! invent receipts, bypass dependencies, or reuse stale verification.
use super::task_plan_revision::PlanRevision;
use super::task_understanding::{ContextSource, EvidenceKind, TaskContract};
use super::tool_execution_gateway::{ToolExecutionReceipt, ToolExecutionStatus};
use super::tool_policy::ToolRisk;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

pub type SharedTaskProgress = Arc<Mutex<TaskProgress>>;
const MAX_EVIDENCE: usize = 256;

#[derive(Debug, Clone, Serialize)]
struct Evidence {
    id: String,
    tool: String,
    kind: EvidenceKind,
    external: bool,
    revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgressUpdate {
    CompleteStep {
        step_id: String,
        receipt_ids: Vec<String>,
    },
    ResolveQuestion {
        question_index: usize,
        receipt_ids: Vec<String>,
    },
    Status,
}

#[derive(Debug, Default)]
pub struct TaskProgress {
    contract: Option<TaskContract>,
    evidence: VecDeque<Evidence>,
    completed: HashMap<String, Vec<String>>,
    resolved: HashMap<usize, Vec<String>>,
    revision: u64,
    plan_revision: u64,
    required_epoch: HashMap<String, u64>,
}

impl TaskProgress {
    pub fn apply_plan(&mut self, plan: TaskContract) {
        if self.contract.is_some() {
            self.revision = self.revision.saturating_add(1);
        }
        self.contract = Some(plan);
        self.completed.clear();
        self.resolved.clear();
        self.required_epoch.clear();
        self.plan_revision = self.plan_revision.saturating_add(1);
    }

    pub fn has_plan(&self) -> bool {
        self.contract.is_some()
    }

    pub fn invalidate_after_rollback(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.evidence.clear();
        self.completed.clear();
        self.resolved.clear();
        self.required_epoch.clear();
    }

    pub fn observe(&mut self, receipt: &ToolExecutionReceipt) {
        if matches!(
            receipt.name.as_str(),
            "plan_task" | "revise_task_plan" | "update_task_progress" | "manage_task"
        ) {
            return;
        }
        if !receipt.verification && matches!(
            receipt.risk,
            ToolRisk::WorkspaceMutation | ToolRisk::Destructive
        ) && matches!(
            receipt.status,
            ToolExecutionStatus::Succeeded
                | ToolExecutionStatus::Failed
                | ToolExecutionStatus::Panicked
        ) {
            self.revision = self.revision.saturating_add(1);
        } else if receipt.risk == ToolRisk::ProcessControl
            && matches!(
                receipt.status,
                ToolExecutionStatus::Failed | ToolExecutionStatus::Panicked
            )
        {
            self.revision = self.revision.saturating_add(1);
        }
        if receipt.status != ToolExecutionStatus::Succeeded
            || receipt.call_id.len() > 128
            || self.evidence.iter().any(|e| e.id == receipt.call_id)
        {
            return;
        }
        let kind = if receipt.verification {
            EvidenceKind::Verification
        } else if receipt.name == "ask_user" {
            if !receipt
                .result
                .get("response")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
            {
                return;
            }
            EvidenceKind::UserInput
        } else if matches!(
            receipt.risk,
            ToolRisk::WorkspaceMutation | ToolRisk::Destructive
        ) {
            EvidenceKind::Mutation
        } else if matches!(receipt.risk, ToolRisk::ReadOnly | ToolRisk::Network) {
            EvidenceKind::Observation
        } else {
            return;
        };
        if self.evidence.len() == MAX_EVIDENCE {
            // Keep the small, bounded set already cited by steps/questions.
            if let Some(index) = self.evidence.iter().position(|e| {
                !self
                    .completed
                    .values()
                    .chain(self.resolved.values())
                    .any(|ids| ids.contains(&e.id))
            }) {
                self.evidence.remove(index);
            }
        }
        self.evidence.push_back(Evidence {
            id: receipt.call_id.clone(),
            tool: receipt.name.chars().take(64).collect(),
            kind,
            external: receipt.risk == ToolRisk::Network,
            revision: self.revision,
        });
    }

    fn references(&self, ids: &[String]) -> Result<Vec<&Evidence>, String> {
        if ids.is_empty() || ids.len() > 8 {
            return Err("cite 1..8 successful receipt IDs from this Run".into());
        }
        ids.iter()
            .map(|id| {
                self.evidence
                    .iter()
                    .find(|e| &e.id == id)
                    .filter(|e| e.kind != EvidenceKind::Verification || e.revision == self.revision)
                    .ok_or_else(|| {
                        "receipt is unknown, unsuccessful, or its verification is stale".into()
                    })
            })
            .collect()
    }

    fn step_satisfied(&self, id: &str) -> bool {
        self.step_satisfied_at_depth(id, 0)
    }

    fn step_satisfied_at_depth(&self, id: &str, depth: usize) -> bool {
        if depth >= super::task_understanding::MAX_STEPS {
            return false;
        }
        let Some(step) = self
            .contract
            .as_ref()
            .and_then(|p| p.steps.iter().find(|s| s.id == id))
        else {
            return false;
        };
        let Some(ids) = self.completed.get(id) else {
            return false;
        };
        self.references(ids)
            .is_ok_and(|refs| self.matches_step(&step.id, step.evidence_kind, &refs))
            && step
                .depends_on
                .iter()
                .all(|id| self.step_satisfied_at_depth(id, depth + 1))
    }

    fn question_satisfied(&self, index: usize) -> bool {
        let Some(gap) = self
            .contract
            .as_ref()
            .and_then(|p| p.open_questions.get(index))
        else {
            return false;
        };
        self.resolved.get(&index).is_some_and(|ids| {
            self.references(ids).is_ok_and(|refs| {
                refs.iter().any(|e| match gap.source {
                    ContextSource::Workspace => e.kind == EvidenceKind::Observation && !e.external,
                    ContextSource::External => e.kind == EvidenceKind::Observation && e.external,
                    ContextSource::User => e.kind == EvidenceKind::UserInput,
                })
            })
        })
    }

    pub fn update(&mut self, update: ProgressUpdate) -> Result<serde_json::Value, String> {
        if matches!(update, ProgressUpdate::Status) {
            return Ok(self.snapshot());
        }
        let plan = self
            .contract
            .as_ref()
            .ok_or("call plan_task before updating progress")?;
        match update {
            ProgressUpdate::CompleteStep {
                step_id,
                receipt_ids,
            } => {
                let step = plan
                    .steps
                    .iter()
                    .find(|s| s.id == step_id)
                    .ok_or("unknown step_id")?;
                if !step.depends_on.iter().all(|id| self.step_satisfied(id)) {
                    return Err("complete the prerequisite steps with valid evidence first".into());
                }
                if !self.references(&receipt_ids)?.iter().any(|e| {
                    e.kind == step.evidence_kind
                        && e.revision >= self.required_epoch.get(&step_id).copied().unwrap_or(0)
                }) {
                    return Err(
                        "receipt kind does not satisfy this step's evidence requirement".into(),
                    );
                }
                self.completed.insert(step_id, receipt_ids);
            }
            ProgressUpdate::ResolveQuestion {
                question_index,
                receipt_ids,
            } => {
                let gap = plan
                    .open_questions
                    .get(question_index)
                    .ok_or("unknown question_index")?;
                if !self
                    .references(&receipt_ids)?
                    .iter()
                    .any(|e| match gap.source {
                        ContextSource::Workspace => {
                            e.kind == EvidenceKind::Observation && !e.external
                        }
                        ContextSource::External => {
                            e.kind == EvidenceKind::Observation && e.external
                        }
                        ContextSource::User => e.kind == EvidenceKind::UserInput,
                    })
                {
                    return Err("receipt source does not resolve this context gap".into());
                }
                self.resolved.insert(question_index, receipt_ids);
            }
            ProgressUpdate::Status => unreachable!(),
        }
        Ok(self.snapshot())
    }

    pub fn completion_gap(&self) -> Option<String> {
        let plan = self.contract.as_ref()?;
        if let Some(step) = plan.steps.iter().find(|s| !self.step_satisfied(&s.id)) {
            return Some(format!(
                "subgoal '{}' needs valid {:?} receipts and completed dependencies",
                step.id, step.evidence_kind
            ));
        }
        plan.open_questions.iter().enumerate().find(|(i, _)| !self.question_satisfied(*i))
            .map(|(i, _)| format!("context question {i} remains unresolved; retrieve the appropriate source or revise the plan with evidence"))
    }

    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "has_plan": self.has_plan(),
            "plan_revision": self.plan_revision,
            "steps": self.contract.as_ref().map(|p| p.steps.iter().map(|s| serde_json::json!({
                "id": s.id, "complete": self.step_satisfied(&s.id), "receipt_ids": self.completed.get(&s.id)
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "questions": self.contract.as_ref().map(|p| p.open_questions.iter().enumerate().map(|(i, _)| serde_json::json!({
                "index": i, "resolved": self.question_satisfied(i)
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "recent_evidence": self.evidence.iter().rev().take(12).collect::<Vec<_>>()
        })
    }

    pub fn render(&self) -> Option<String> {
        self.contract.as_ref().map(|plan| format!("{}\n[Subgoal Evidence]\n{}\nFor added requirements use revise_task_plan with this plan_revision and only the changed fields; unmentioned requirements and unaffected progress are retained. Do not reread unchanged sources or rewrite the entire plan. Use update_task_progress to bind receipts. New or changed steps require fresh evidence; verification must be current. Full replacement clears bindings.\n", plan.render(), self.snapshot()))
    }

    fn matches_step(&self, id: &str, kind: EvidenceKind, evidence: &[&Evidence]) -> bool {
        let minimum = self.required_epoch.get(id).copied().unwrap_or(0);
        evidence
            .iter()
            .any(|e| e.kind == kind && e.revision >= minimum)
    }

    pub fn revise(&mut self, patch: PlanRevision) -> Result<serde_json::Value, String> {
        let previous = self
            .contract
            .as_ref()
            .ok_or("call plan_task before revising")?;
        if patch.base_revision != self.plan_revision {
            return Err(format!(
                "stale plan revision: expected {}; inspect current state and retry only the delta",
                self.plan_revision
            ));
        }
        // Validate and compute everything before mutating state.
        let next = patch.merge(previous)?;
        if &next == previous {
            return Ok(
                serde_json::json!({"status":"unchanged","plan_revision":self.plan_revision,"task_contract":previous,"requires_verification":false}),
            );
        }
        let mut invalidated: std::collections::HashSet<String> = next
            .steps
            .iter()
            .filter(|s| previous.steps.iter().find(|old| old.id == s.id) != Some(*s))
            .map(|s| s.id.clone())
            .collect();
        let requires_verification = !invalidated.is_empty()
            || previous.success_criteria != next.success_criteria
            || previous.constraints != next.constraints;
        if requires_verification {
            invalidated.extend(
                next.steps
                    .iter()
                    .filter(|s| s.evidence_kind == EvidenceKind::Verification)
                    .map(|s| s.id.clone()),
            );
        }
        loop {
            let extra: Vec<_> = next
                .steps
                .iter()
                .filter(|s| {
                    !invalidated.contains(&s.id)
                        && s.depends_on.iter().any(|id| invalidated.contains(id))
                })
                .map(|s| s.id.clone())
                .collect();
            if extra.is_empty() {
                break;
            }
            invalidated.extend(extra);
        }
        let retained: Vec<_> = next
            .steps
            .iter()
            .filter(|s| !invalidated.contains(&s.id) && self.step_satisfied(&s.id))
            .map(|s| s.id.clone())
            .collect();
        if requires_verification {
            self.revision = self.revision.saturating_add(1);
        }
        for id in &invalidated {
            self.completed.remove(id);
            self.required_epoch.insert(id.clone(), self.revision);
        }
        self.plan_revision = self.plan_revision.saturating_add(1);
        self.contract = Some(next);
        let invalidated: Vec<_> = self
            .contract
            .as_ref()
            .unwrap()
            .steps
            .iter()
            .filter(|s| invalidated.contains(&s.id))
            .map(|s| s.id.clone())
            .collect();
        Ok(
            serde_json::json!({"status":"revised","plan_revision":self.plan_revision,
            "retained_steps":retained,"invalidated_steps":invalidated,
            "requires_verification":requires_verification,"task_contract":self.contract}),
        )
    }
}
