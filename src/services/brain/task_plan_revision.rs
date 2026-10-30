//! Additive task-plan revisions. Unmentioned requirements are never removed.
use super::task_understanding::{
    ContextNeed, EvidenceKind, MAX_PLAN_CHARS, MAX_STEPS, TaskContract, TaskStep,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRevision {
    pub base_revision: u64,
    #[serde(default)]
    pub append_success_criteria: Vec<String>,
    #[serde(default)]
    pub append_constraints: Vec<String>,
    #[serde(default)]
    pub upsert_steps: Vec<StepRevision>,
    #[serde(default)]
    pub append_questions: Vec<ContextNeed>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StepRevision {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_kind: Option<EvidenceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
}

impl PlanRevision {
    pub fn from_value(value: Value) -> Result<Self, String> {
        if value.to_string().chars().count() > MAX_PLAN_CHARS {
            return Err("revision exceeds the total plan budget; send only changed fields".into());
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    pub fn schema() -> Value {
        let plan = TaskContract::schema();
        let mut criteria = plan["properties"]["success_criteria"].clone();
        criteria["minItems"] = json!(0);
        let mut step = plan["properties"]["steps"]["items"].clone();
        step["properties"]["evidence_kind"]
            .as_object_mut()
            .unwrap()
            .remove("default");
        step["required"] = json!(["id"]);
        step["description"] = json!(
            "Existing step: send only changed fields; omitted fields and existing dependencies are preserved. New step: include goal, acceptance and the appropriate evidence_kind."
        );
        json!({"type":"object", "additionalProperties":false, "properties": {
            "base_revision":{"type":"integer", "minimum":1, "description":"Use plan_revision from the current runtime state; stale revisions are rejected."},
            "append_success_criteria":criteria,
            "append_constraints":plan["properties"]["constraints"],
            "upsert_steps":{"type":"array","maxItems":MAX_STEPS,"items":step},
            "append_questions":plan["properties"]["open_questions"]
        }, "required":["base_revision"]})
    }

    pub fn merge(&self, current: &TaskContract) -> Result<TaskContract, String> {
        let mut next = current.clone();
        append_unique(&mut next.success_criteria, &self.append_success_criteria);
        append_unique(&mut next.constraints, &self.append_constraints);
        append_unique(&mut next.open_questions, &self.append_questions);
        let mut ids = HashSet::new();
        for step in &self.upsert_steps {
            if !ids.insert(&step.id) {
                return Err("upsert_steps contains duplicate ids".into());
            }
            if let Some(existing) = next.steps.iter_mut().find(|s| s.id == step.id) {
                if step
                    .evidence_kind
                    .is_some_and(|kind| kind != existing.evidence_kind)
                {
                    return Err("a revision cannot change an existing step's evidence kind; submit an explicitly reviewed replacement plan".into());
                }
                if let Some(goal) = &step.goal {
                    existing.goal = goal.clone();
                }
                if let Some(acceptance) = &step.acceptance {
                    existing.acceptance = acceptance.clone();
                }
                if let Some(dependencies) = &step.depends_on {
                    append_unique(&mut existing.depends_on, dependencies);
                }
            } else {
                next.steps.push(TaskStep {
                    id: step.id.clone(),
                    goal: step.goal.clone().unwrap_or_default(),
                    acceptance: step.acceptance.clone().unwrap_or_default(),
                    evidence_kind: step.evidence_kind.unwrap_or_default(),
                    depends_on: step.depends_on.clone().unwrap_or_default(),
                });
            }
        }
        if next.steps.len() > MAX_STEPS {
            return Err(format!(
                "merged steps exceed {MAX_STEPS}; group related work instead of deleting original requirements"
            ));
        }
        // Stable topological order permits adding a new prerequisite to an
        // existing step without forcing the caller to resend every step.
        let all: HashSet<_> = next.steps.iter().map(|s| s.id.clone()).collect();
        if next
            .steps
            .iter()
            .any(|s| s.depends_on.iter().any(|id| !all.contains(id)))
        {
            return Err("revision references an unknown dependency".into());
        }
        let mut pending = std::mem::take(&mut next.steps);
        let mut emitted = HashSet::new();
        while !pending.is_empty() {
            let Some(index) = pending
                .iter()
                .position(|s| s.depends_on.iter().all(|id| emitted.contains(id)))
            else {
                return Err("revision introduces a dependency cycle".into());
            };
            let step = pending.remove(index);
            emitted.insert(step.id.clone());
            next.steps.push(step);
        }
        TaskContract::from_value(serde_json::to_value(next).map_err(|e| e.to_string())?)
    }
}

fn append_unique<T: PartialEq + Clone>(target: &mut Vec<T>, extra: &[T]) {
    for value in extra {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
}
