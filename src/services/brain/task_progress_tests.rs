use super::task_plan_revision::PlanRevision;
use super::task_progress::{ProgressUpdate, TaskProgress};
use super::task_understanding::TaskContract;
use super::tool_execution_gateway::{ToolExecutionReceipt, ToolExecutionStatus};
use super::tool_policy::ToolRisk;
use serde_json::json;

fn plan() -> TaskContract {
    TaskContract::from_value(json!({
        "interpretation": "Inspect, implement and verify the requested change",
        "success_criteria": ["The original requirements are covered"],
        "steps": [
            {"id": "inspect", "goal": "Find the actual contract", "acceptance": "Relevant source read", "evidence_kind": "observation"},
            {"id": "change", "goal": "Apply the requested change", "acceptance": "Source updated", "depends_on": ["inspect"], "evidence_kind": "mutation"},
            {"id": "verify", "goal": "Check the change", "acceptance": "Checks passed", "depends_on": ["change"], "evidence_kind": "verification"}
        ],
        "open_questions": [{"question": "What does the workspace require?", "source": "workspace"}]
    })).unwrap()
}

fn receipt(id: &str, name: &str, risk: ToolRisk, verification: bool) -> ToolExecutionReceipt {
    ToolExecutionReceipt {
        call_id: id.into(),
        name: name.into(),
        status: ToolExecutionStatus::Succeeded,
        duration_ms: 0,
        result: json!({}),
        original_result_chars: 0,
        truncated: false,
        stored_result_reference: None,
        risk,
        verification,
    }
}

fn complete(
    runtime: &mut TaskProgress,
    step: &str,
    ids: &[&str],
) -> Result<serde_json::Value, String> {
    runtime.update(ProgressUpdate::CompleteStep {
        step_id: step.into(),
        receipt_ids: ids.iter().map(|s| s.to_string()).collect(),
    })
}

#[test]
fn subgoals_require_real_receipts_of_the_right_kind_and_order() {
    let mut runtime = TaskProgress::default();
    runtime.apply_plan(plan());
    assert!(complete(&mut runtime, "inspect", &["invented"]).is_err());
    runtime.observe(&receipt("read", "view_file", ToolRisk::ReadOnly, false));
    runtime.observe(&receipt(
        "write",
        "replace_in_file",
        ToolRisk::WorkspaceMutation,
        false,
    ));
    assert!(complete(&mut runtime, "change", &["write"]).is_err());
    complete(&mut runtime, "inspect", &["read"]).unwrap();
    assert!(complete(&mut runtime, "change", &["read"]).is_err());
    complete(&mut runtime, "change", &["write"]).unwrap();
    assert!(complete(&mut runtime, "verify", &["read"]).is_err());
    runtime.observe(&receipt(
        "test",
        "run_command",
        ToolRisk::ProcessControl,
        true,
    ));
    complete(&mut runtime, "verify", &["test"]).unwrap();
    assert!(runtime.completion_gap().unwrap().contains("question"));
    runtime
        .update(ProgressUpdate::ResolveQuestion {
            question_index: 0,
            receipt_ids: vec!["read".into()],
        })
        .unwrap();
    assert!(runtime.completion_gap().is_none());
    runtime.observe(&receipt(
        "write-again",
        "replace_in_file",
        ToolRisk::WorkspaceMutation,
        false,
    ));
    assert!(runtime.completion_gap().unwrap().contains("verify"));
    assert!(complete(&mut runtime, "verify", &["test"]).is_err());
}

#[test]
fn failed_tools_plans_todos_and_background_processes_are_not_step_evidence() {
    let mut runtime = TaskProgress::default();
    runtime.apply_plan(plan());
    for (id, name, risk) in [
        ("plan", "plan_task", ToolRisk::ReadOnly),
        ("todo", "manage_task", ToolRisk::WorkspaceMutation),
        ("progress", "update_task_progress", ToolRisk::ReadOnly),
        ("background", "run_command", ToolRisk::ProcessControl),
    ] {
        runtime.observe(&receipt(id, name, risk, false));
        assert!(complete(&mut runtime, "inspect", &[id]).is_err());
    }
    let mut failed = receipt("failed", "view_file", ToolRisk::ReadOnly, false);
    failed.status = ToolExecutionStatus::Failed;
    runtime.observe(&failed);
    assert!(complete(&mut runtime, "inspect", &["failed"]).is_err());
}

#[test]
fn foreground_process_facts_are_citable_observations_but_not_mutation_or_verification() {
    let mut runtime = TaskProgress::default();
    runtime.apply_plan(plan());
    let mut observed = receipt("actual-shell-id", "run_command", ToolRisk::ProcessControl, false);
    observed.result = json!({"exit_code":0,"stdout":"actual directory listing"});
    runtime.observe(&observed);
    complete(&mut runtime, "inspect", &["actual-shell-id"]).unwrap();
    assert!(complete(&mut runtime, "change", &["actual-shell-id"]).is_err());
    assert!(!observed.verification);
    assert!(runtime.snapshot().to_string().contains("actual-shell-id"));

    use super::cognitive_runtime::{CognitiveRuntime, CompletionDecision, ReasoningMode};
    let mut investigation = CognitiveRuntime::with_mode("分析本地安装应用的技术栈", ReasoningMode::Adaptive);
    investigation.observe_receipt(&observed);
    assert_eq!(investigation.review_completion("已通过真实的命令执行结果查验安装目录及其技术栈，结论基于实际输出；本轮没有改动安装文件，也不把目录信息当成业务播放验证。"), CompletionDecision::Accept);

    for (id, status, result) in [
        ("failed-shell", ToolExecutionStatus::Failed, json!({"exit_code":1})),
        ("background-shell", ToolExecutionStatus::Succeeded, json!({"process_id":123})),
    ] {
        let mut bad = receipt(id, "run_command", ToolRisk::ProcessControl, false);
        bad.status = status; bad.result = result;
        runtime.observe(&bad);
        assert!(complete(&mut runtime, "inspect", &[id]).is_err());
    }
}

#[test]
fn context_gap_resolution_checks_source_and_real_user_responses() {
    let mut runtime = TaskProgress::default();
    let mut contract = plan();
    contract.open_questions = serde_json::from_value(json!([
        {"question": "Which local API?", "source": "workspace"},
        {"question": "What does the external specification require?", "source": "external"},
        {"question": "Which incompatible behavior does the user prefer?", "source": "user"}
    ]))
    .unwrap();
    runtime.apply_plan(contract);
    runtime.observe(&receipt("local", "view_file", ToolRisk::ReadOnly, false));
    runtime.observe(&receipt("web", "web_fetch", ToolRisk::Network, false));
    runtime.observe(&receipt(
        "empty-answer",
        "ask_user",
        ToolRisk::ReadOnly,
        false,
    ));
    assert!(
        runtime
            .update(ProgressUpdate::ResolveQuestion {
                question_index: 0,
                receipt_ids: vec!["web".into()]
            })
            .is_err()
    );
    assert!(
        runtime
            .update(ProgressUpdate::ResolveQuestion {
                question_index: 1,
                receipt_ids: vec!["local".into()]
            })
            .is_err()
    );
    assert!(
        runtime
            .update(ProgressUpdate::ResolveQuestion {
                question_index: 2,
                receipt_ids: vec!["empty-answer".into()]
            })
            .is_err()
    );
    let mut answer = receipt("answer", "ask_user", ToolRisk::ReadOnly, false);
    answer.result = json!({"requestId": "request", "response": "Preserve the old API"});
    runtime.observe(&answer);
    for (i, id) in ["local", "web", "answer"].into_iter().enumerate() {
        runtime
            .update(ProgressUpdate::ResolveQuestion {
                question_index: i,
                receipt_ids: vec![id.into()],
            })
            .unwrap();
    }
    assert!(
        runtime.snapshot()["questions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|q| q["resolved"] == true)
    );
}

#[test]
fn revised_plans_clear_bindings_without_losing_observed_facts() {
    let mut runtime = TaskProgress::default();
    runtime.apply_plan(plan());
    runtime.observe(&receipt("read", "view_file", ToolRisk::ReadOnly, false));
    complete(&mut runtime, "inspect", &["read"]).unwrap();
    runtime.apply_plan(plan());
    assert!(runtime.completion_gap().unwrap().contains("inspect"));
    complete(&mut runtime, "inspect", &["read"]).unwrap();
    let mut fresh = TaskProgress::default();
    fresh.apply_plan(plan());
    assert!(complete(&mut fresh, "inspect", &["read"]).is_err());
}

#[test]
fn evidence_cache_is_bounded_and_preserves_cited_receipts() {
    let mut runtime = TaskProgress::default();
    runtime.apply_plan(plan());
    runtime.observe(&receipt("pinned", "view_file", ToolRisk::ReadOnly, false));
    complete(&mut runtime, "inspect", &["pinned"]).unwrap();
    for i in 0..300 {
        runtime.observe(&receipt(
            &format!("read-{i}"),
            "view_file",
            ToolRisk::ReadOnly,
            false,
        ));
    }
    complete(&mut runtime, "inspect", &["pinned"]).unwrap();
    assert!(
        runtime.snapshot()["recent_evidence"]
            .as_array()
            .unwrap()
            .len()
            <= 12
    );
}

fn verified() -> TaskProgress {
    let mut runtime = TaskProgress::default();
    runtime.apply_plan(plan());
    runtime.observe(&receipt("read", "view_file", ToolRisk::ReadOnly, false));
    complete(&mut runtime, "inspect", &["read"]).unwrap();
    runtime.observe(&receipt(
        "write",
        "replace_in_file",
        ToolRisk::WorkspaceMutation,
        false,
    ));
    complete(&mut runtime, "change", &["write"]).unwrap();
    runtime.observe(&receipt(
        "test",
        "run_command",
        ToolRisk::ProcessControl,
        true,
    ));
    complete(&mut runtime, "verify", &["test"]).unwrap();
    runtime
        .update(ProgressUpdate::ResolveQuestion {
            question_index: 0,
            receipt_ids: vec!["read".into()],
        })
        .unwrap();
    assert!(runtime.completion_gap().is_none());
    runtime
}

#[test]
fn rollback_preserves_requirements_but_rejects_all_old_receipts() {
    let mut runtime = verified();
    let original = runtime.render().unwrap().split("[Subgoal Evidence]").next().unwrap().to_string();
    runtime.invalidate_after_rollback();
    assert!(runtime.render().unwrap().starts_with(&original));
    assert!(complete(&mut runtime, "inspect", &["read"]).is_err());
    assert!(complete(&mut runtime, "change", &["write"]).is_err());
    assert!(complete(&mut runtime, "verify", &["test"]).is_err());
    runtime.observe(&receipt("new-read", "view_file", ToolRisk::ReadOnly, false));
    assert!(complete(&mut runtime, "inspect", &["new-read"]).is_ok());
}

#[test]
fn additive_revision_preserves_requirements_and_unaffected_progress_but_rechecks_new_constraints() {
    let mut runtime = verified();
    let result = runtime
        .revise(
            PlanRevision::from_value(json!({
                "base_revision":1, "append_constraints":["dry-run must not write data"],
                "append_success_criteria":["dry-run behavior is tested"]
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(result["plan_revision"], 2);
    assert_eq!(result["retained_steps"], json!(["inspect", "change"]));
    assert_eq!(result["invalidated_steps"], json!(["verify"]));
    assert_eq!(
        result["task_contract"]["success_criteria"][0],
        "The original requirements are covered"
    );
    assert_eq!(runtime.snapshot()["questions"][0]["resolved"], true);
    assert!(complete(&mut runtime, "verify", &["test"]).is_err());
    runtime.observe(&receipt(
        "fresh-test",
        "run_command",
        ToolRisk::ProcessControl,
        true,
    ));
    complete(&mut runtime, "verify", &["fresh-test"]).unwrap();
    assert!(runtime.completion_gap().is_none());
}

#[test]
fn revised_dependency_graph_keeps_inspection_but_requires_fresh_changed_work() {
    let mut runtime = verified();
    let result = runtime.revise(PlanRevision::from_value(json!({
        "base_revision":1,
        "upsert_steps":[
            {"id":"preview", "goal":"Inspect the added dry-run requirement", "acceptance":"New observation", "evidence_kind":"observation", "depends_on":["inspect"]},
            {"id":"change", "goal":"Implement both original behavior and dry-run", "acceptance":"Updated implementation", "evidence_kind":"mutation", "depends_on":["preview"]}
        ]
    })).unwrap()).unwrap();
    let ids: Vec<_> = result["task_contract"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["inspect", "preview", "change", "verify"]);
    assert_eq!(result["retained_steps"], json!(["inspect"]));
    assert!(complete(&mut runtime, "preview", &["read"]).is_err());
    runtime.observe(&receipt("new-read", "view_file", ToolRisk::ReadOnly, false));
    complete(&mut runtime, "preview", &["new-read"]).unwrap();
    assert!(complete(&mut runtime, "change", &["write"]).is_err());
    runtime.observe(&receipt(
        "new-write",
        "replace_in_file",
        ToolRisk::WorkspaceMutation,
        false,
    ));
    complete(&mut runtime, "change", &["new-write"]).unwrap();
    assert!(runtime.completion_gap().unwrap().contains("verify"));
}

#[test]
fn stale_cyclic_over_budget_and_weakened_revisions_leave_state_unchanged() {
    let mut runtime = verified();
    for value in [
        json!({"base_revision":0,"append_constraints":["stale"]}),
        json!({"base_revision":1,"upsert_steps":[{"id":"inspect","goal":"cycle","acceptance":"source","evidence_kind":"observation","depends_on":["verify"]}]}),
        json!({"base_revision":1,"append_constraints":["知".repeat(241)]}),
        json!({"base_revision":1,"upsert_steps":[{"id":"change","goal":"weaken","acceptance":"just read","evidence_kind":"observation"}]}),
    ] {
        let before = format!("{runtime:?}");
        assert!(
            runtime
                .revise(PlanRevision::from_value(value).unwrap())
                .is_err()
        );
        assert_eq!(format!("{runtime:?}"), before);
        assert!(runtime.completion_gap().is_none());
    }
}

#[test]
fn duplicate_delta_is_a_noop_and_full_replacement_invalidates_old_checks() {
    let mut runtime = verified();
    let result = runtime.revise(PlanRevision::from_value(json!({
        "base_revision":1, "append_success_criteria":["The original requirements are covered"]
    })).unwrap()).unwrap();
    assert_eq!(result["status"], "unchanged");
    assert_eq!(result["plan_revision"], 1);
    assert!(runtime.completion_gap().is_none());
    runtime.apply_plan(plan());
    complete(&mut runtime, "inspect", &["read"]).unwrap();
    complete(&mut runtime, "change", &["write"]).unwrap();
    assert!(complete(&mut runtime, "verify", &["test"]).is_err());
}

#[test]
fn partial_step_updates_preserve_omitted_fields_and_prerequisites() {
    let mut runtime = verified();
    let result = runtime.revise(PlanRevision::from_value(json!({
        "base_revision":1, "upsert_steps":[{"id":"change", "goal":"Also implement dry-run", "depends_on":[]}]
    })).unwrap()).unwrap();
    let step = result["task_contract"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "change")
        .unwrap();
    assert_eq!(step["evidence_kind"], "mutation");
    assert_eq!(step["acceptance"], "Source updated");
    assert_eq!(step["depends_on"], json!(["inspect"]));
    assert_eq!(result["retained_steps"], json!(["inspect"]));
}
