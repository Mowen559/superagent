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
    facts: super::run_facts::RunFacts,
    read_only_scope: bool,
    subagents: Option<std::sync::Weak<super::subagent_runtime::SubagentRuntime>>,
    run_root: Option<std::path::PathBuf>,
    session_id: String,
    run_id: String,
    parent_verification: Option<serde_json::Value>,
    review_rejection: Option<String>,
    rejected_review_scope: Option<serde_json::Value>,
    memory_only_scope: bool,
    delivery: Option<super::delivery_evidence::DeliveryEvidence>,
    contract: Option<TaskContract>,
    evidence: VecDeque<Evidence>,
    completed: HashMap<String, Vec<String>>,
    resolved: HashMap<usize, Vec<String>>,
    revision: u64,
    plan_revision: u64,
    required_epoch: HashMap<String, u64>,
}

impl TaskProgress {
    pub fn configure_delivery(&mut self, root: &std::path::Path, session: &str, run: &str) -> Result<(), String> {
        let profile=std::env::var_os("HUB_DELIVERY_PROFILE_PATH").map(std::path::PathBuf::from);
        self.configure_delivery_profile(root,session,run,profile.as_deref())
    }

    pub fn configure_delivery_profile(&mut self,root:&std::path::Path,session:&str,run:&str,profile:Option<&std::path::Path>)->Result<(),String> {
        let mut delivery = super::delivery_evidence::DeliveryEvidence::new(root, session, run)?;
        if let Some(path) = profile {
            delivery.load_profile(path)?;
        }
        *self = Self { run_root: Some(std::fs::canonicalize(root).map_err(|e|e.to_string())?), session_id:session.into(), run_id:run.into(), subagents:self.subagents.clone(), delivery: Some(delivery), facts: super::run_facts::RunFacts::new(session,run), read_only_scope:self.read_only_scope, memory_only_scope:self.memory_only_scope, ..Self::default() };
        Ok(())
    }

    pub fn bind_subagents(&mut self, runtime: std::sync::Weak<super::subagent_runtime::SubagentRuntime>) { self.subagents=Some(runtime); }

    pub(crate) fn execution_evidence_context(&self) -> Option<(std::path::PathBuf,String,String)> {
        Some((self.run_root.clone()?,self.session_id.clone(),self.run_id.clone()))
    }

    /// Gateway-only physical observation; no caller/model supplies this fingerprint.
    pub fn verification_stamp(&self) -> Result<Option<serde_json::Value>, String> {
        let Some(runtime)=self.subagents.as_ref().and_then(|r|r.upgrade()) else {return Ok(None);};
        if runtime.delivery_projection(&self.run_id)["childCount"] == 0 {return Ok(None);}
        let root=self.run_root.as_ref().ok_or("delegated delivery is not bound to a parent workspace")?;
        let fingerprint=crate::domains::version_control::change_set::workspace_fingerprint(root)?;
        Ok(Some(serde_json::json!({"parentSessionId":self.session_id,"parentRunId":self.run_id,"workspaceFingerprint":fingerprint,"delegationEpoch":runtime.delivery_projection(&self.run_id)["evidenceEpoch"]})))
    }

    pub fn delegation_snapshot(&self) -> serde_json::Value {
        let Some(runtime)=self.subagents.as_ref().and_then(|r|r.upgrade()) else {return serde_json::Value::Null;};
        let mut state=runtime.delivery_projection(&self.run_id);
        if state["childCount"] == 0 {return state;}
        if state["gap"].is_null() && state["requiresParentVerification"] == true {
            let current=self.verification_stamp();
            let valid=matches!(&current,Ok(Some(stamp)) if self.parent_verification.as_ref().is_some_and(|r| &r["stamp"]==stamp));
            if valid {state["state"]=serde_json::json!("verified_quality");state["parentVerification"]=self.parent_verification.clone().unwrap();}
            else {state["gap"]=serde_json::json!("delegated changes require a successful foreground parent quality check bound to the current code fingerprint; child success text is insufficient");}
        }
        state
    }

    pub fn delivery_gate_gap(&self) -> Option<String> {self.review_rejection.clone().or_else(||self.delegation_gap())}

    pub fn delegation_gap(&self) -> Option<String> {
        self.delegation_snapshot()["gap"].as_str().map(str::to_string)
    }

    pub fn verify_delivery(&mut self, ids: &[String]) -> Result<serde_json::Value, String> {
        self.delivery.as_mut().ok_or("delivery is not bound to a Run workspace")?.verify(ids)
    }

    pub fn perform_delivery_action(&mut self,id:&str)->Result<serde_json::Value,String> {
        self.delivery.as_mut().ok_or("delivery is not bound to a Run workspace")?.perform_action(id)
    }
    pub fn managed_command_guard(&self,name:&str,args:&serde_json::Value)->Option<String> {
        self.delivery.as_ref().and_then(|d|d.managed_command_guard(name,args))
    }
    pub fn receipt_edit_guard(&self,name:&str,args:&serde_json::Value)->Option<String> {
        self.delivery.as_ref().and_then(|d|d.receipt_edit_guard(name,args))
    }
    pub fn delivery_guidance(&self)->String {
        self.delivery.as_ref().map(|d|d.action_guidance()).unwrap_or_default()
    }
    pub fn managed_closeout_strategy(&self)->Option<String> {
        self.delivery.as_ref().and_then(|d|d.managed_closeout_strategy())
    }
    pub fn host_physical_delivery_complete(&self)->bool {
        self.delivery.as_ref().is_some_and(|d|d.host_physical_delivery_complete())
    }
    pub fn uses_host_physical_acceptance(&self)->bool {
        self.delivery.as_ref().is_some_and(|d|d.uses_host_physical_acceptance())
    }
    pub fn internal_trace_guard(&self,name:&str,args:&serde_json::Value)->Option<String> {
        self.delivery.as_ref().and_then(|d|d.internal_trace_guard(name,args))
    }

    /// Returns whether the effective contract changed. Identical retransmission is inert.
    pub fn apply_plan(&mut self, plan: TaskContract) -> bool {
        if self.contract.as_ref() == Some(&plan) { return false; }
        if let Some(delivery) = &mut self.delivery { delivery.declare(plan.delivery.as_ref()); }
        if self.contract.is_some() {
            self.revision = self.revision.saturating_add(1);
        }
        self.contract = Some(plan);
        self.completed.clear();
        self.resolved.clear();
        self.required_epoch.clear();
        self.plan_revision = self.plan_revision.saturating_add(1);
        true
    }

    pub fn normalize_plan_delivery(&self, mut plan: TaskContract) -> Result<TaskContract, String> {
        if self.read_only_scope && plan.steps.iter().any(|s|s.evidence_kind==EvidenceKind::Mutation) {
            return Err("explicit read-only objective cannot acknowledge mutation steps".into());
        }
        if let (Some(delivery), Some(declaration)) = (&self.delivery, &plan.delivery) {
            plan.delivery = Some(delivery.normalize_declaration(declaration)?);
        }
        Ok(plan)
    }

    pub fn has_plan(&self) -> bool {
        self.contract.is_some()
    }

    pub fn set_read_only_scope(&mut self, value: bool) { self.read_only_scope = value; }
    pub fn is_read_only_scope(&self) -> bool { self.read_only_scope }
    pub fn set_memory_only_scope(&mut self,value:bool) { self.memory_only_scope=value; }
    pub fn rejects_mutation(&self, name: &str, risk: ToolRisk) -> bool {
        (self.read_only_scope && (matches!(risk,ToolRisk::WorkspaceMutation|ToolRisk::Destructive) || name=="perform_delivery_action"))
            || (self.memory_only_scope && name!="super_memory" && risk!=ToolRisk::ReadOnly)
    }
    pub fn planned_execution(&self) -> Option<bool> {
        self.contract.as_ref().and_then(|p| {
            if p.steps.iter().any(|s|s.evidence_kind==EvidenceKind::Mutation) {Some(true)}
            else if p.steps.iter().any(|s|s.evidence_kind==EvidenceKind::Verification) {Some(false)} else {None}
        })
    }
    pub fn facts_guidance(&self) -> String { self.facts.render() }
    pub fn note_requirement_update(&mut self) {
        self.facts.note_requirement_update();
        self.revision=self.revision.saturating_add(1);
        if let Some(delivery)=&mut self.delivery { delivery.invalidate(); }
        if let Some(plan)=&self.contract {
            let mut affected:std::collections::HashSet<String>=plan.steps.iter().filter(|s|s.evidence_kind==EvidenceKind::Verification).map(|s|s.id.clone()).collect();
            loop {
                let extra:Vec<_>=plan.steps.iter().filter(|s|!affected.contains(&s.id)&&s.depends_on.iter().any(|id|affected.contains(id))).map(|s|s.id.clone()).collect();
                if extra.is_empty(){break;}affected.extend(extra);
            }
            for id in affected {self.completed.remove(&id);self.required_epoch.insert(id,self.revision);}
        }
    }

    pub fn invalidate_after_rollback(&mut self) {
        if let Some(delivery) = &mut self.delivery { delivery.invalidate(); }
        self.revision = self.revision.saturating_add(1);
        self.evidence.clear();
        self.completed.clear();
        self.resolved.clear();
        self.required_epoch.clear();
    }

    pub fn observe(&mut self, receipt: &ToolExecutionReceipt) {
        self.facts.observe(receipt);
        if receipt.name == "code_review" {
            if receipt.status != ToolExecutionStatus::Succeeded || receipt.result["stats"]["critical_count"].as_u64().is_some_and(|n|n>0) {self.review_rejection=Some(format!("code_review {} rejected or failed; repair and rerun actual review before delivery",receipt.call_id));self.rejected_review_scope=Some(receipt.result["review_scope"].clone());}
            else if receipt.result["total_files_analyzed"].as_u64().is_some_and(|n|n>0) && receipt.result["stats"]["critical_count"]==0
                && (receipt.result["review_scope"].is_null() || self.rejected_review_scope.as_ref()==Some(&receipt.result["review_scope"])) {self.review_rejection=None;self.rejected_review_scope=None;}
        }
        if receipt.result.get("parent_verification_error").is_some() || (receipt.risk==ToolRisk::ProcessControl && matches!(receipt.status,ToolExecutionStatus::Failed|ToolExecutionStatus::Panicked)) {self.parent_verification=None;}
        if receipt.verification {
            if let Some(stamp)=receipt.result.get("parent_workspace_verification") {
                if stamp["parentSessionId"]==self.session_id && stamp["parentRunId"]==self.run_id {self.parent_verification=Some(serde_json::json!({"receiptId":receipt.call_id,"stamp":stamp}));}
            }
        }
        if matches!(
            receipt.name.as_str(),
            "plan_task" | "revise_task_plan" | "update_task_progress" | "manage_task"
        ) {
            return;
        }
        if receipt.name!="super_memory" && !receipt.verification && matches!(
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
        } else if receipt.name == "perform_delivery_action" {
            EvidenceKind::Mutation
        } else if receipt.is_process_observation() {
            EvidenceKind::Observation
        } else if matches!(
            receipt.risk,
            ToolRisk::WorkspaceMutation | ToolRisk::Destructive
        ) {
            EvidenceKind::Mutation
        } else if matches!(receipt.risk, ToolRisk::ReadOnly | ToolRisk::Network)
            || receipt.is_process_observation() {
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
        if let Some(gap)=self.delivery_gate_gap() {return Some(gap); }
        if let Some(gap) = self.delivery.as_ref().and_then(|d| d.completion_gap()) { return Some(gap); }
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
            "current_run_facts": self.facts.snapshot(),
            "delegation": self.delegation_snapshot(),
            "review_rejection":self.review_rejection,
            "has_plan": self.has_plan(),
            "delivery_evidence": self.delivery.as_ref().map(|d| d.snapshot()),
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
        self.contract.as_ref().map(|plan| format!("{}\n[Subgoal Evidence]\n{}\nFor added requirements use revise_task_plan with this plan_revision and only the changed fields; unmentioned requirements and unaffected progress are retained. Do not reread unchanged sources or rerun successful checks merely to advance progress. For each evidenced subgoal call update_task_progress(action=complete_step, step_id=<id>, receipt_ids=[<actual matching receipt ID>]) in dependency order. Confirm that the receipt supports that step's acceptance condition before citing it. Markdown Todo updates and action=status do not complete steps. New or changed steps require fresh evidence; verification must be current. A changed full replacement clears bindings; identical retransmission preserves them.\n", plan.render(), self.snapshot()))
            .or_else(|| self.delivery.as_ref().filter(|d| d.completion_gap().is_some()).map(|d| format!("[Required physical delivery]\n{}\nCreate the declared artifacts and use verify_delivery. A probe or plan does not satisfy this requirement.\n", d.snapshot())))
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
        self.normalize_plan_delivery(next.clone())?;
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
