//! Model-authored task decomposition kept separate from verified execution facts.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const MAX_STEPS: usize = 12;
pub const MAX_CRITERIA: usize = 12;
pub const MAX_CONSTRAINTS: usize = 16;
pub const MAX_QUESTIONS: usize = 8;
pub const MAX_PLAN_CHARS: usize = 12_000;
const MAX_INTERPRETATION: usize = 600;
const MAX_ITEM_CHARS: usize = 240;
const MAX_GOAL_CHARS: usize = 320;
const MAX_ACCEPTANCE_CHARS: usize = 400;
const MAX_QUESTION_CHARS: usize = 320;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    pub interpretation: String,
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub steps: Vec<TaskStep>,
    #[serde(default)]
    pub open_questions: Vec<ContextNeed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStep {
    pub id: String,
    pub goal: String,
    pub acceptance: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub evidence_kind: EvidenceKind,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    #[default]
    Observation,
    Mutation,
    Verification,
    UserInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextNeed {
    pub question: String,
    pub source: ContextSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    Workspace,
    External,
    User,
}

impl TaskContract {
    pub fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "interpretation": {"type": "string", "minLength": 1, "maxLength": MAX_INTERPRETATION},
                "success_criteria": {"type": "array", "minItems": 1, "maxItems": MAX_CRITERIA, "items": {"type": "string", "minLength": 1, "maxLength": MAX_ITEM_CHARS}},
                "constraints": {"type": "array", "maxItems": MAX_CONSTRAINTS, "items": {"type": "string", "minLength": 1, "maxLength": MAX_ITEM_CHARS}},
                "steps": {"type": "array", "minItems": 1, "maxItems": MAX_STEPS, "items": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "id": {"type": "string", "minLength": 1, "maxLength": 32},
                        "goal": {"type": "string", "minLength": 1, "maxLength": MAX_GOAL_CHARS},
                        "acceptance": {"type": "string", "minLength": 1, "maxLength": MAX_ACCEPTANCE_CHARS},
                        "evidence_kind": {"type": "string", "enum": ["observation", "mutation", "verification", "user_input"], "default": "observation", "description": "Use mutation for changes, verification for quality checks, observation for investigation, user_input only for a real user choice."},
                        "depends_on": {"type": "array", "maxItems": MAX_STEPS, "items": {"type": "string", "minLength": 1, "maxLength": 32}}
                    },
                    "required": ["id", "goal", "acceptance"]
                }},
                "open_questions": {"type": "array", "maxItems": MAX_QUESTIONS, "items": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "question": {"type": "string", "minLength": 1, "maxLength": MAX_QUESTION_CHARS},
                        "source": {"type": "string", "enum": ["workspace", "external", "user"]}
                    },
                    "required": ["question", "source"]
                }}
            },
            "required": ["interpretation", "success_criteria", "steps"]
        })
    }

    pub fn from_value(value: Value) -> Result<Self, String> {
        let contract: Self = serde_json::from_value(value).map_err(|error| error.to_string())?;
        let mut issues = Vec::new();
        if serde_json::to_string(&contract)
            .map_err(|e| e.to_string())?
            .chars()
            .count()
            > MAX_PLAN_CHARS
        {
            return Err(format!(
                "task plan exceeds the {MAX_PLAN_CHARS}-character total budget; shorten wording without dropping requirements"
            ));
        }
        text_issue(
            &mut issues,
            &contract.interpretation,
            MAX_INTERPRETATION,
            "interpretation",
        );
        list_issues(
            &mut issues,
            &contract.success_criteria,
            MAX_CRITERIA,
            "success_criteria",
            true,
        );
        list_issues(
            &mut issues,
            &contract.constraints,
            MAX_CONSTRAINTS,
            "constraints",
            false,
        );
        if contract.steps.is_empty() || contract.steps.len() > MAX_STEPS {
            issues.push(format!(
                "steps: got {} items; allowed 1..{MAX_STEPS}",
                contract.steps.len()
            ));
        }
        let mut known = HashSet::new();
        for (index, step) in contract.steps.iter().take(MAX_STEPS).enumerate() {
            let field = format!("steps[{index}]");
            text_issue(&mut issues, &step.id, 32, &format!("{field}.id"));
            if !step
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                issues.push(format!("{field}.id: use ASCII letters, digits, '_' or '-'"));
            }
            text_issue(
                &mut issues,
                &step.goal,
                MAX_GOAL_CHARS,
                &format!("{field}.goal"),
            );
            text_issue(
                &mut issues,
                &step.acceptance,
                MAX_ACCEPTANCE_CHARS,
                &format!("{field}.acceptance"),
            );
            if known.contains(&step.id)
                || step.depends_on.len() > MAX_STEPS
                || step.depends_on.iter().any(|id| !known.contains(id))
            {
                issues.push(format!("{field}.depends_on: ids must be unique and dependencies must reference earlier steps; no cycles"));
            }
            known.insert(step.id.clone());
        }
        if contract.open_questions.len() > MAX_QUESTIONS {
            issues.push(format!(
                "open_questions: got {} items; maximum {MAX_QUESTIONS}",
                contract.open_questions.len()
            ));
        }
        for (index, gap) in contract
            .open_questions
            .iter()
            .take(MAX_QUESTIONS)
            .enumerate()
        {
            text_issue(
                &mut issues,
                &gap.question,
                MAX_QUESTION_CHARS,
                &format!("open_questions[{index}].question"),
            );
        }
        if !issues.is_empty() {
            return Err(serde_json::json!({"kind":"invalid_task_plan", "issues":issues,
                "recovery":"Correct all listed fields in one submission. Group or shorten wording; preserve every original requirement and constraint."}).to_string());
        }
        Ok(contract)
    }

    pub fn render(&self) -> String {
        let mut output = format!(
            "\n[Task Understanding - model-authored plan, not completion evidence]\nInterpretation: {}\nSuccess criteria:\n- {}\nConstraints to check against the original request:\n- {}\nOrdered subgoals:\n",
            self.interpretation,
            self.success_criteria.join("\n- "),
            self.constraints.join("\n- ")
        );
        for step in &self.steps {
            output.push_str(&format!(
                "- {}: {} (after: {}); acceptance: {}; required evidence: {:?}\n",
                step.id,
                step.goal,
                step.depends_on.join(", "),
                step.acceptance,
                step.evidence_kind
            ));
        }
        for gap in &self.open_questions {
            let action = match gap.source {
                ContextSource::Workspace => {
                    "inspect workspace instructions, relevant symbols, or tests before asking the user"
                }
                ContextSource::External => "retrieve an authoritative external source when needed",
                ContextSource::User => {
                    "ask only if the choice remains material and cannot be established from available context"
                }
            };
            output.push_str(&format!(
                "- Context gap: {}; next action: {}\n",
                gap.question, action
            ));
        }
        output.push_str("Follow dependencies, compare outcomes with every success criterion, and update this plan when new evidence changes the understanding. Original user instructions remain authoritative.\n");
        output
    }
}

fn text_issue(issues: &mut Vec<String>, value: &str, max: usize, field: &str) {
    let actual = value.chars().count();
    if value.trim().is_empty() || actual > max {
        issues.push(format!(
            "{field}: got {actual} characters; allowed 1..{max}"
        ));
    }
}

fn list_issues(
    issues: &mut Vec<String>,
    values: &[String],
    max: usize,
    field: &str,
    required: bool,
) {
    if values.len() > max || (required && values.is_empty()) {
        issues.push(format!(
            "{field}: got {} items; allowed {}..{max}",
            values.len(),
            usize::from(required)
        ));
    }
    for (index, value) in values.iter().take(max).enumerate() {
        text_issue(issues, value, MAX_ITEM_CHARS, &format!("{field}[{index}]"));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub fn sample() -> Value {
        json!({
            "interpretation": "Improve task understanding using workspace evidence",
            "success_criteria": ["No invented framework rules", "Each subgoal has an acceptance condition"],
            "constraints": ["Preserve existing public APIs"],
            "steps": [
                {"id": "inspect", "goal": "Read actual project conventions", "acceptance": "Cite the source of each constraint"},
                {"id": "implement", "goal": "Improve decomposition", "acceptance": "Regression checks pass", "depends_on": ["inspect"]}
            ],
            "open_questions": [{"question": "Which module owns intent routing?", "source": "workspace"}]
        })
    }

    #[test]
    fn decomposition_preserves_acceptance_dependencies_and_context_actions() {
        let plan = TaskContract::from_value(sample()).unwrap().render();
        assert!(plan.contains("after: inspect"));
        assert!(plan.contains("Regression checks pass"));
        assert!(plan.contains("inspect workspace instructions"));
        assert!(plan.contains("not completion evidence"));
    }

    #[test]
    fn invalid_dependency_graphs_and_unbounded_plans_are_rejected() {
        for bad in [json!(["implement"]), json!(["missing"])] {
            let mut value = sample();
            value["steps"][0]["depends_on"] = bad;
            assert!(TaskContract::from_value(value).is_err());
        }
        let mut value = sample();
        value["steps"][1]["id"] = json!("inspect");
        assert!(TaskContract::from_value(value).is_err());
        let mut value = sample();
        value["success_criteria"] = json!([]);
        assert!(TaskContract::from_value(value).is_err());
        let mut value = sample();
        value["interpretation"] = json!("知".repeat(MAX_INTERPRETATION + 1));
        assert!(TaskContract::from_value(value).is_err());
    }

    #[test]
    fn practical_complex_plans_are_bounded_without_silently_dropping_requirements() {
        let mut value = sample();
        value["constraints"] = json!(
            (0..9)
                .map(|i| format!("Keep requirement {i}"))
                .collect::<Vec<_>>()
        );
        value["steps"][0]["goal"] = json!("目".repeat(200));
        let contract = TaskContract::from_value(value).unwrap();
        assert_eq!(contract.constraints.len(), 9);
        assert_eq!(contract.steps[0].goal.chars().count(), 200);
        let schema = TaskContract::schema();
        assert_eq!(
            schema["properties"]["constraints"]["maxItems"],
            MAX_CONSTRAINTS
        );
    }

    #[test]
    fn one_validation_response_reports_all_field_limits_with_actual_lengths() {
        let mut value = sample();
        value["interpretation"] = json!("知".repeat(MAX_INTERPRETATION + 1));
        value["steps"][0]["goal"] = json!("知".repeat(MAX_GOAL_CHARS + 1));
        let error = TaskContract::from_value(value).unwrap_err();
        let feedback: Value = serde_json::from_str(&error).unwrap();
        assert_eq!(feedback["kind"], "invalid_task_plan");
        assert_eq!(feedback["issues"].as_array().unwrap().len(), 2);
        assert!(error.contains("interpretation: got 601"));
        assert!(error.contains("steps[0].goal: got 321"));
    }
}
