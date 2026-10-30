//! Explicit, bounded synthetic planning evaluation. This harness never executes
//! model-proposed code or touches a real project. Lexical coverage is a proxy,
//! not a claim that a real implementation works.
use super::cognitive_runtime::{CognitiveRuntime, ReasoningMode};
use super::task_understanding::TaskContract;
use super::tool_execution_gateway::{ToolExecutionReceipt, ToolExecutionStatus};
use super::tool_policy::classify_tool_risk;
use super::tools::plan_task::PlanTaskTool;
use super::tools::revise_task_plan::ReviseTaskPlanTool;
use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::domains::model_gateway::providers::{LlmProvider, TokenUsage};
use crate::domains::model_gateway::routing::ProviderAttemptOutcome;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

const MAX_TURNS: usize = 4;

#[derive(Debug, Clone, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub request: String,
    pub steering: Option<String>,
    pub files: BTreeMap<String, String>,
    pub expected: Vec<ExpectedTerm>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedTerm {
    pub id: String,
    pub any_of: Vec<String>,
}

pub fn cases() -> Vec<EvalCase> {
    serde_json::from_str(include_str!("task_evaluation_cases.json"))
        .expect("built-in evaluation fixtures")
}

#[derive(Debug, Serialize)]
pub struct CaseReport {
    pub case_id: String,
    pub variant: String,
    pub status: String,
    pub valid_plan: bool,
    pub covered_term_checks: Vec<String>,
    pub missing_term_checks: Vec<String>,
    pub unnecessary_questions: usize,
    pub context_reads: usize,
    pub missing_context_reads: usize,
    pub unknown_tool_calls: usize,
    pub invalid_plan_calls: usize,
    pub revision_calls: usize,
    pub successful_revisions: usize,
    pub original_requirement_text_retained: Option<bool>,
    pub early_final_response: bool,
    pub steering_applied: bool,
    pub model_turns: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub usage_reported: bool,
    pub cost: f64,
    pub duration_ms: u64,
    pub actual_models: BTreeSet<String>,
    pub plan: Option<TaskContract>,
    pub trace: Vec<ActionTrace>,
}

#[derive(Debug, Serialize)]
pub struct ActionTrace {
    pub turn: usize,
    pub tool: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EvalSettings {
    pub requested_model: String,
    pub temperature: f64,
    pub max_output_tokens: u32,
}

#[derive(Debug, Serialize)]
pub struct EvaluationReport {
    pub schema_version: u32,
    pub suite_fingerprint: String,
    pub settings: Option<EvalSettings>,
    pub scope: String,
    pub baseline: String,
    pub enhanced: String,
    pub same_actual_model: bool,
    pub max_model_turns: usize,
    pub results: Vec<CaseReport>,
}

#[async_trait]
pub trait EvaluationModel: Send + Sync {
    async fn turn(
        &self,
        messages: Vec<Value>,
        system: &str,
        tools: Value,
    ) -> Result<(Value, TokenUsage), String>;
}

#[async_trait]
impl EvaluationModel for LlmProvider {
    async fn turn(
        &self,
        messages: Vec<Value>,
        system: &str,
        tools: Value,
    ) -> Result<(Value, TokenUsage), String> {
        self.chat_with_tools(messages, Some(system), Some(tools))
            .await
    }
}

pub async fn evaluate(model: &impl EvaluationModel, cases: &[EvalCase]) -> EvaluationReport {
    let mut results = Vec::new();
    'campaign: for case in cases.iter().take(3) {
        for enhanced in [false, true] {
            let result = evaluate_case(model, case, enhanced).await;
            let unavailable =
                matches!(result.status.as_str(), "provider_error" | "request_timeout");
            eprintln!(
                "[TaskEval] {} / {}: {} ({} turns)",
                result.case_id, result.variant, result.status, result.model_turns
            );
            results.push(result);
            if unavailable {
                break 'campaign;
            }
        }
    }
    let same_actual_model = same_model(&results, cases.len().min(3) * 2);
    EvaluationReport {
        schema_version: 3,
        suite_fingerprint: format!("xxh3:{:016x}", xxhash_rust::xxh3::xxh3_64(include_bytes!("task_evaluation_cases.json"))),
        settings: None,
        scope: "synthetic_planning_only; term checks are lexical proxies; no implementation success measured".into(),
        baseline: "full-plan replacement; shared validation and fixed synthetic documents; not a historical release".into(),
        enhanced: "CognitiveRuntime/proactive context plus stateful additive revisions".into(),
        same_actual_model, max_model_turns: cases.len().min(3) * 2 * MAX_TURNS, results,
    }
}

fn same_model(results: &[CaseReport], expected: usize) -> bool {
    let actual: BTreeSet<_> = results
        .iter()
        .flat_map(|r| r.actual_models.iter())
        .collect();
    !results.is_empty()
        && results.len() == expected
        && actual.len() == 1
        && results.iter().all(|r| {
            !r.actual_models.is_empty()
                && !matches!(r.status.as_str(), "provider_error" | "request_timeout")
        })
}

fn tools(revision: &ReviseTaskPlanTool, enhanced: bool) -> Value {
    let mut schemas = json!([
        {"type": "function", "function": {"name": "plan_task", "description": PlanTaskTool.description(), "parameters": PlanTaskTool.parameters_schema()}},
        {"type": "function", "function": {"name": "view_file", "description": "Read an available synthetic workspace file", "parameters": {
            "type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]
        }}},
        {"type": "function", "function": {"name": "ask_user", "description": "Ask only about unresolved material choices; fixture facts are in workspace documents", "parameters": {
            "type": "object", "properties": {"Question": {"type": "string"}}, "required": ["Question"]
        }}}
    ]);
    if enhanced {
        schemas.as_array_mut().unwrap().push(json!({"type":"function","function":{
        "name":revision.name(),"description":revision.description(),"parameters":revision.parameters_schema()
    }}));
    }
    schemas
}

async fn evaluate_case(
    model: &impl EvaluationModel,
    case: &EvalCase,
    enhanced: bool,
) -> CaseReport {
    let started = Instant::now();
    let mut report = CaseReport {
        case_id: case.id.clone(),
        variant: if enhanced { "enhanced" } else { "baseline" }.into(),
        status: "turn_budget_exhausted".into(),
        valid_plan: false,
        covered_term_checks: vec![],
        missing_term_checks: vec![],
        unnecessary_questions: 0,
        context_reads: 0,
        missing_context_reads: 0,
        unknown_tool_calls: 0,
        invalid_plan_calls: 0,
        revision_calls: 0,
        successful_revisions: 0,
        original_requirement_text_retained: None,
        early_final_response: false,
        steering_applied: false,
        model_turns: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        usage_reported: false,
        cost: 0.0,
        duration_ms: 0,
        actual_models: BTreeSet::new(),
        plan: None,
        trace: Vec::new(),
    };
    let root = tempfile::tempdir().expect("synthetic evaluation workspace");
    for (path, content) in &case.files {
        // Built-in fixtures deliberately have no arbitrary paths or real data.
        if std::path::Path::new(path)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            continue;
        }
        let target = root.path().join(path);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(target, content);
    }
    let context = if enhanced {
        super::run_prelude::workspace_context(root.path())
    } else {
        String::new()
    };
    let mut cognition = CognitiveRuntime::with_mode(
        &format!("只做规划：{}", case.request),
        ReasoningMode::Adaptive,
    );
    let revision_tool = ReviseTaskPlanTool(cognition.task_progress());
    let mut initial_plan: Option<TaskContract> = None;
    let mut messages = vec![
        json!({"role": "user", "content": format!("{}\nAvailable workspace files: {}", case.request, case.files.keys().cloned().collect::<Vec<_>>().join(", "))}),
    ];
    for _ in 0..MAX_TURNS {
        report.model_turns += 1;
        let guidance = if enhanced {
            cognition.next_guidance().unwrap_or_default()
        } else {
            String::new()
        };
        let revision_guidance = if enhanced {
            "Once a plan exists, apply additional user requirements with revise_task_plan and only the delta; use the current plan_revision. Retain earlier requirements and do not reread unchanged sources. The steps describe future implementation; only planning is authorized here."
        } else {
            "After additional requirements, submit an updated complete plan with plan_task."
        };
        let system = format!(
            "This is a planning-only evaluation. Inspect relevant available workspace facts and submit a concise plan with plan_task. Use the exact relative file names in the available-file list. Do not implement code or claim an implementation is complete. Use existing conventions, observable success criteria and dependency-ordered subgoals.\n{revision_guidance}\n{guidance}\n{context}"
        );
        let response = tokio::time::timeout(
            Duration::from_secs(60),
            model.turn(messages.clone(), &system, tools(&revision_tool, enhanced)),
        )
        .await;
        let (message, usage) = match response {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                report.status = "provider_error".into();
                break;
            }
            Err(_) => {
                report.status = "request_timeout".into();
                break;
            }
        };
        report.prompt_tokens += usage.prompt_tokens as u64;
        report.completion_tokens += usage.completion_tokens as u64;
        report.usage_reported |= usage.prompt_tokens > 0 || usage.completion_tokens > 0;
        report.cost += usage.cost;
        for attempt in usage.attempt_receipts {
            if attempt.outcome == ProviderAttemptOutcome::Succeeded {
                report
                    .actual_models
                    .insert(format!("{}:{}", attempt.provider, attempt.resolved_model));
            }
        }
        let calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if calls.is_empty() {
            report.early_final_response = true;
            report.status = "final_without_plan".into();
            break;
        }
        if calls.len() > 8 {
            report.status = "tool_batch_limit".into();
            break;
        }
        messages.push(message);
        let mut planned = false;
        for call in calls {
            let name = call["function"]["name"].as_str().unwrap_or("");
            let args = call["function"]["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            let mut result = match name {
                "view_file" => {
                    report.context_reads += 1;
                    args.get("path")
                        .or_else(|| args.get("AbsolutePath"))
                        .and_then(Value::as_str)
                        .and_then(|path| case.files.get(path))
                        .map(|content| json!({"content": content, "source": "synthetic workspace"}))
                        .unwrap_or_else(|| {
                            report.missing_context_reads += 1;
                            json!({"error": "fixture file not found; inspect the available-file list or record this as an unresolved workspace context gap"})
                        })
                }
                "ask_user" => {
                    report.unnecessary_questions += 1;
                    json!({"error": "No external user input is needed for this fixture. Inspect the available workspace documents."})
                }
                "plan_task" => match TaskContract::from_value(args.clone()) {
                    Ok(plan) => {
                        report.plan = Some(plan.clone());
                        report.valid_plan = true;
                        planned = true;
                        json!({"status": "planned", "task_contract": plan})
                    }
                    Err(error) => {
                        report.invalid_plan_calls += 1;
                        json!({"error": error})
                    }
                },
                "revise_task_plan" if enhanced => {
                    report.revision_calls += 1;
                    match revision_tool.execute(args.clone()) {
                        Ok(value) => {
                            report.successful_revisions += 1;
                            report.plan = value
                                .get("task_contract")
                                .cloned()
                                .and_then(|v| TaskContract::from_value(v).ok());
                            report.valid_plan = report.plan.is_some();
                            planned = report.valid_plan;
                            value
                        }
                        Err(error) => {
                            report.invalid_plan_calls += 1;
                            json!({"error":error})
                        }
                    }
                }
                _ => {
                    report.unknown_tool_calls += 1;
                    json!({"error": "Only planning and fixture inspection tools are available"})
                }
            };
            cognition.observe_receipt(&ToolExecutionReceipt {
                call_id: call["id"].as_str().unwrap_or("fixture-call").to_string(),
                name: name.to_string(),
                status: if result.get("error").is_some() {
                    ToolExecutionStatus::Failed
                } else {
                    ToolExecutionStatus::Succeeded
                },
                duration_ms: 0,
                original_result_chars: 0,
                truncated: false,
                stored_result_reference: None,
                risk: classify_tool_risk(name, &args),
                verification: false,
                result: result.clone(),
            });
            if matches!(name, "plan_task" | "revise_task_plan") && result.get("error").is_none() {
                result["plan_revision"] =
                    cognition.task_progress().lock().unwrap().snapshot()["plan_revision"].clone();
            }
            report.trace.push(ActionTrace {
                turn: report.model_turns,
                tool: name.chars().take(64).collect(),
                error: result
                    .get("error")
                    .and_then(Value::as_str)
                    .map(|s| s.chars().take(240).collect()),
            });
            messages.push(
                json!({"role": "tool", "tool_call_id": call["id"], "content": result.to_string()}),
            );
        }
        if planned {
            if let Some(steering) = case.steering.as_ref().filter(|_| !report.steering_applied) {
                report.steering_applied = true;
                // A plan from before the added constraint cannot count as the
                // final answer if the model then ignores the steering message.
                initial_plan = report.plan.take();
                report.valid_plan = false;
                messages.push(json!({"role": "user", "content": steering}));
            } else {
                report.status = "planned".into();
                break;
            }
        }
    }
    report.original_requirement_text_retained = initial_plan
        .as_ref()
        .zip(report.plan.as_ref())
        .map(|(before, after)| {
            before
                .success_criteria
                .iter()
                .all(|s| after.success_criteria.contains(s))
                && before
                    .constraints
                    .iter()
                    .all(|s| after.constraints.contains(s))
        });
    let text = report
        .plan
        .as_ref()
        .map(|p| {
            format!(
                "{} {} {} {}",
                p.interpretation,
                p.success_criteria.join(" "),
                p.constraints.join(" "),
                p.steps
                    .iter()
                    .map(|s| format!("{} {}", s.goal, s.acceptance))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        })
        .unwrap_or_default()
        .to_lowercase();
    for term in &case.expected {
        if term
            .any_of
            .iter()
            .any(|alias| text.contains(&alias.to_lowercase()))
        {
            report.covered_term_checks.push(term.id.clone());
        } else {
            report.missing_term_checks.push(term.id.clone());
        }
    }
    report.duration_ms = started.elapsed().as_millis() as u64;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct Scripted(Mutex<VecDeque<Value>>);
    #[async_trait]
    impl EvaluationModel for Scripted {
        async fn turn(
            &self,
            _: Vec<Value>,
            _: &str,
            _: Value,
        ) -> Result<(Value, TokenUsage), String> {
            Ok((
                self.0
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("scripted response"),
                TokenUsage::default(),
            ))
        }
    }
    fn call(name: &str, args: Value) -> Value {
        json!({"role": "assistant", "content": null, "tool_calls": [{"id": uuid::Uuid::new_v4().to_string(), "type": "function", "function": {"name": name, "arguments": args.to_string()}}]})
    }

    #[tokio::test]
    async fn evaluation_counts_questions_and_missing_coverage_without_inventing_model_identity() {
        let plan = super::super::task_understanding::tests::sample();
        let model = Scripted(Mutex::new(VecDeque::from([
            call("ask_user", json!({"Question": "Which encoding?"})),
            call("plan_task", plan.clone()),
            call("plan_task", plan),
        ])));
        let mut report = evaluate(&model, &cases()[..1]).await;
        assert_eq!(report.results[0].unnecessary_questions, 1);
        assert_eq!(report.results[1].unnecessary_questions, 0);
        assert!(!report.results[0].missing_term_checks.is_empty());
        assert!(!report.same_actual_model);
        assert!(!report.results[0].usage_reported);
        // Poor planning is a measured outcome, not evidence of a model change.
        for result in &mut report.results {
            result.actual_models.insert("fixture:test-model".into());
            result.status = "final_without_plan".into();
        }
        assert!(same_model(&report.results, 2));
        report.results[1]
            .actual_models
            .insert("fixture:different-model".into());
        assert!(!same_model(&report.results, 2));
    }

    #[tokio::test]
    async fn an_old_plan_cannot_count_as_satisfying_later_steering() {
        let plan = super::super::task_understanding::tests::sample();
        let model = Scripted(Mutex::new(VecDeque::from([
            call("plan_task", plan.clone()),
            json!({"role": "assistant", "content": "done"}),
            call("plan_task", plan),
            json!({"role": "assistant", "content": "done"}),
        ])));
        let report = evaluate(&model, &cases()[2..]).await;
        assert!(
            report
                .results
                .iter()
                .all(|r| r.steering_applied && !r.valid_plan && r.early_final_response)
        );
    }

    #[tokio::test]
    async fn missing_context_is_recorded_without_rejecting_an_honest_planning_gap() {
        let plan = super::super::task_understanding::tests::sample();
        let model = Scripted(Mutex::new(VecDeque::from([
            call("view_file", json!({"path": "not-present.rs"})),
            call("plan_task", plan.clone()),
            call("plan_task", plan),
        ])));
        let report = evaluate(&model, &cases()[..1]).await;
        assert!(report.results[0].valid_plan);
        assert_eq!(report.results[0].missing_context_reads, 1);
        assert!(
            report.results[0].trace[0]
                .error
                .as_ref()
                .unwrap()
                .contains("context gap")
        );
        assert!(
            report.results[0]
                .plan
                .as_ref()
                .unwrap()
                .open_questions
                .len()
                > 0
        );
        assert!(report.suite_fingerprint.starts_with("xxh3:"));
    }

    #[tokio::test]
    async fn steering_revision_uses_real_shared_state_and_keeps_original_requirement_text() {
        let initial = super::super::task_understanding::tests::sample();
        let mut replacement = initial.clone();
        replacement["success_criteria"] = json!(["Only the new condition"]);
        let model = Scripted(Mutex::new(VecDeque::from([
            call("plan_task", initial.clone()),
            call("plan_task", replacement),
            call("plan_task", initial),
            call(
                "revise_task_plan",
                json!({
                    "base_revision":1, "append_constraints":["dry-run must preserve parser.rs"]
                }),
            ),
        ])));
        let report = evaluate(&model, &cases()[2..]).await;
        assert_eq!(
            report.results[0].original_requirement_text_retained,
            Some(false)
        );
        let enhanced = &report.results[1];
        assert!(enhanced.valid_plan);
        assert_eq!(enhanced.successful_revisions, 1);
        assert_eq!(enhanced.original_requirement_text_retained, Some(true));
        assert!(
            enhanced
                .plan
                .as_ref()
                .unwrap()
                .constraints
                .iter()
                .any(|s| s.contains("dry-run"))
        );
    }
}
