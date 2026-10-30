use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::tool_approval_store::ToolApprovalStore;
use crate::services::brain::tool_loop_guard::ToolLoopGuard;
use crate::services::brain::tool_policy::{
    ToolPolicy, ToolPolicyDecision, ToolPolicyMode, ToolRisk, classify_tool_risk,
};
use crate::services::brain::tool_result_store::ToolResultStore;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const DEFAULT_MAX_RESULT_CHARS: usize = 16_000;

pub type ToolRegistry = Arc<Mutex<HashMap<String, Arc<dyn ToolExecutor>>>>;

#[derive(Debug, Clone)]
pub struct ToolExecutionRequest {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionStatus {
    Succeeded,
    Failed,
    Panicked,
    NotFound,
    PolicyDenied,
    ApprovalRequired,
    RepetitionBlocked,
}

#[derive(Debug, Clone)]
pub struct ToolExecutionReceipt {
    pub call_id: String,
    pub name: String,
    pub status: ToolExecutionStatus,
    pub duration_ms: u64,
    pub result: Value,
    pub original_result_chars: usize,
    pub truncated: bool,
    pub stored_result_reference: Option<String>,
    pub risk: ToolRisk,
    /// True only for a successful, foreground, allow-listed quality command.
    pub verification: bool,
}

pub struct ToolExecutionGateway {
    registry: ToolRegistry,
    max_result_chars: usize,
    result_store: ToolResultStore,
    policy: ToolPolicy,
    approval_store: ToolApprovalStore,
    loop_guard: Mutex<ToolLoopGuard>,
    workspace_revision: AtomicU64,
}

impl ToolExecutionGateway {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            max_result_chars: DEFAULT_MAX_RESULT_CHARS,
            result_store: ToolResultStore::default_location(),
            policy: ToolPolicy::from_env(),
            approval_store: ToolApprovalStore::default_location(),
            loop_guard: Mutex::new(ToolLoopGuard::default()),
            workspace_revision: AtomicU64::new(0),
        }
    }

    pub fn begin_run(&self) {
        self.workspace_revision.store(0, Ordering::Relaxed);
        self.loop_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset();
    }

    #[cfg(test)]
    fn with_max_result_chars(
        registry: ToolRegistry,
        max_result_chars: usize,
        result_store: ToolResultStore,
    ) -> Self {
        Self {
            registry,
            max_result_chars,
            result_store,
            policy: ToolPolicy::new(ToolPolicyMode::Audit),
            approval_store: ToolApprovalStore::default_location(),
            loop_guard: Mutex::new(ToolLoopGuard::default()),
            workspace_revision: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    fn with_policy(registry: ToolRegistry, policy: ToolPolicy) -> Self {
        Self {
            registry,
            max_result_chars: DEFAULT_MAX_RESULT_CHARS,
            result_store: ToolResultStore::default_location(),
            policy,
            approval_store: ToolApprovalStore::default_location(),
            loop_guard: Mutex::new(ToolLoopGuard::default()),
            workspace_revision: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    fn with_policy_and_approval_store(
        registry: ToolRegistry,
        policy: ToolPolicy,
        approval_store: ToolApprovalStore,
    ) -> Self {
        Self {
            registry,
            max_result_chars: DEFAULT_MAX_RESULT_CHARS,
            result_store: ToolResultStore::default_location(),
            policy,
            approval_store,
            loop_guard: Mutex::new(ToolLoopGuard::default()),
            workspace_revision: AtomicU64::new(0),
        }
    }

    pub fn execute(&self, request: ToolExecutionRequest) -> ToolExecutionReceipt {
        let started = Instant::now();
        let (risk, decision) = self.policy.evaluate(&request.name, &request.arguments);
        let repetition = {
            let mut guard = self
                .loop_guard
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if is_verification_request(&request.name, &request.arguments) {
                guard.record_verification(
                    &request.name,
                    &request.arguments,
                    self.workspace_revision.load(Ordering::Relaxed),
                )
            } else {
                guard.record(&request.name, &request.arguments)
            }
        };
        if let Err(block) = repetition {
            return self.receipt(
                request,
                ToolExecutionStatus::RepetitionBlocked,
                started,
                json!({
                    "error": "repeated tool call blocked by loop guard",
                    "kind": "repetition_blocked",
                    "fingerprint": block.fingerprint,
                    "consecutive_count": block.consecutive_count,
                    "window_count": block.window_count,
                    "recovery_hint": "Change the arguments, inspect prior results, or ask the user for direction."
                }),
            );
        }
        if decision == ToolPolicyDecision::Deny {
            return self.receipt(
                request,
                ToolExecutionStatus::PolicyDenied,
                started,
                json!({
                    "error": "tool execution denied by policy",
                    "kind": "policy_denied",
                    "risk": format!("{:?}", risk),
                    "policy_hint": "Set HUB_TOOL_POLICY_MODE=audit or use an approved control-plane flow."
                }),
            );
        }
        if decision == ToolPolicyDecision::RequireApproval {
            match self
                .approval_store
                .consume_matching(&request.name, &request.arguments)
            {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return match self
                        .approval_store
                        .request(&request.name, &request.arguments, risk)
                    {
                        Ok(approval) => self.receipt(
                            request,
                            ToolExecutionStatus::ApprovalRequired,
                            started,
                            json!({
                                "error": "tool approval required",
                                "kind": "approval_required",
                                "approval_id": approval.id,
                                "tool": approval.tool_name,
                                "risk": approval.risk,
                                "expires_at": approval.expires_at,
                                "control_plane": {
                                    "approve": format!("POST /api/tool-approvals/{}/approve", approval.id),
                                    "deny": format!("POST /api/tool-approvals/{}/deny", approval.id),
                                    "required_header": "X-Hub-Approval: approve|deny",
                                }
                            }),
                        ),
                        Err(error) => self.receipt(
                            request,
                            ToolExecutionStatus::Failed,
                            started,
                            json!({
                                "error": format!("approval store unavailable: {error}"),
                                "kind": "approval_store_error"
                            }),
                        ),
                    };
                }
                Err(error) => {
                    return self.receipt(
                        request,
                        ToolExecutionStatus::Failed,
                        started,
                        json!({
                            "error": format!("approval store unavailable: {error}"),
                            "kind": "approval_store_error"
                        }),
                    );
                }
            }
        }
        let tool = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&request.name)
            .cloned();

        let Some(tool) = tool else {
            return self.receipt(
                request,
                ToolExecutionStatus::NotFound,
                started,
                json!({ "error": "tool not found", "kind": "not_found" }),
            );
        };

        let execution = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tool.execute(request.arguments.clone())
        }));

        match execution {
            Ok(Ok(result)) => {
                self.receipt(request, ToolExecutionStatus::Succeeded, started, result)
            }
            Ok(Err(error)) => self.receipt(
                request,
                ToolExecutionStatus::Failed,
                started,
                json!({ "error": error, "kind": "execution_failed" }),
            ),
            Err(_) => self.receipt(
                request,
                ToolExecutionStatus::Panicked,
                started,
                json!({ "error": "tool panicked", "kind": "panic" }),
            ),
        }
    }

    fn receipt(
        &self,
        request: ToolExecutionRequest,
        status: ToolExecutionStatus,
        started: Instant,
        result: Value,
    ) -> ToolExecutionReceipt {
        // Ok(Value) means the adapter returned a result, not that its command
        // passed. Normalize execution facts before compression or cognition.
        let status = if status == ToolExecutionStatus::Succeeded
            && matches!(request.name.as_str(), "run_command" | "verify_task_tool")
            && (result
                .get("exit_code")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0)
                || result.get("success").and_then(Value::as_bool) == Some(false)
                || result.get("timed_out").and_then(Value::as_bool) == Some(true)
                || result.get("error").is_some_and(|error| !error.is_null())
                || matches!(
                    result.get("status").and_then(Value::as_str),
                    Some("failed" | "timed_out" | "cancelled" | "interactive_prompt_detected")
                )) {
            ToolExecutionStatus::Failed
        } else {
            status
        };
        let verification = status == ToolExecutionStatus::Succeeded
            && result.get("exit_code").and_then(Value::as_i64) == Some(0)
            && is_verification_request(&request.name, &request.arguments);
        let risk = classify_tool_risk(&request.name, &request.arguments);
        if status == ToolExecutionStatus::Succeeded
            && (matches!(risk, ToolRisk::WorkspaceMutation | ToolRisk::Destructive)
                || (request.name == "revise_task_plan"
                    && result.get("requires_verification").and_then(Value::as_bool) == Some(true)))
        {
            self.workspace_revision.fetch_add(1, Ordering::Relaxed);
        }

        let serialized_original = serde_json::to_string(&result).unwrap_or_default();
        let original_result_chars = serialized_original.chars().count();
        let needs_truncation = original_result_chars > self.max_result_chars;
        let mut stored_result_reference = None;

        let (final_result, truncated) = if needs_truncation {
            let stored = self.result_store.persist(&serialized_original).ok();
            stored_result_reference = stored.as_ref().map(|item| item.reference.clone());

            let (mut compressed, was_truncated) =
                crate::services::brain::context_interceptor::intelligent_compress(
                    &request,
                    result,
                    self.max_result_chars,
                );
            if was_truncated {
                if let Some(obj) = compressed.as_object_mut() {
                    obj.insert(
                        "full_result_reference".to_string(),
                        json!(stored_result_reference),
                    );
                }
            }
            (compressed, was_truncated)
        } else {
            (result, false)
        };

        ToolExecutionReceipt {
            call_id: request.call_id,
            name: request.name,
            status,
            duration_ms: started.elapsed().as_millis() as u64,
            result: final_result,
            original_result_chars,
            truncated,
            stored_result_reference,
            risk,
            verification,
        }
    }
}

fn is_verification_request(name: &str, arguments: &Value) -> bool {
    let command_key = match name {
        "run_command" => "CommandLine",
        "verify_task_tool" => "command",
        _ => return false,
    };
    if arguments
        .get("RunInBackground")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    let Some(command) = arguments.get(command_key).and_then(Value::as_str) else {
        return false;
    };
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || ["&&", "||", ";", "|", ">", "<", "\n", "\r", "`", "$"]
            .iter()
            .any(|operator| normalized.contains(operator))
    {
        return false;
    }
    if normalized.split_whitespace().any(|arg| {
        matches!(
            arg,
            "--help" | "-h" | "--version" | "--list" | "--collect-only"
        )
    }) {
        return false;
    }
    const QUALITY_COMMANDS: &[&str] = &[
        "cargo test",
        "cargo check",
        "cargo clippy",
        "cargo fmt --check",
        "npm test",
        "npm run test",
        "npm run lint",
        "npm run build",
        "npm run typecheck",
        "pnpm test",
        "pnpm run test",
        "pnpm lint",
        "pnpm build",
        "yarn test",
        "yarn lint",
        "yarn build",
        "pytest",
        "python -m pytest",
        "python3 -m pytest",
        "go test",
        "mvn test",
        "mvn verify",
        "./mvnw test",
        "./mvnw verify",
        "gradle test",
        "gradle check",
        "./gradlew test",
        "./gradlew check",
        "dotnet test",
        "ruff check",
        "eslint",
        "tsc --noemit",
        "npx tsc --noemit",
    ];
    QUALITY_COMMANDS.iter().any(|prefix| {
        normalized == *prefix
            || normalized
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with(char::is_whitespace))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::brain::cognitive_runtime::{
        CognitiveRuntime, CompletionDecision, ReasoningMode,
    };
    use tempfile::tempdir;

    struct ReturnedTool {
        name: &'static str,
        result: Value,
    }

    impl ToolExecutor for ReturnedTool {
        fn name(&self) -> &'static str {
            self.name
        }
        fn description(&self) -> &'static str {
            "deterministic adapter fixture"
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn execute(&self, _: Value) -> Result<Value, String> {
            Ok(self.result.clone())
        }
    }

    fn intelligence_gateway() -> ToolExecutionGateway {
        let tools: ToolRegistry = Arc::new(Mutex::new(HashMap::new()));
        tools.lock().unwrap().insert(
            "plan_task".into(),
            Arc::new(crate::services::brain::tools::plan_task::PlanTaskTool),
        );
        for (name, result) in [
            ("view_file", json!({"content": "fn main() {}"})),
            ("replace_in_file", json!({"replaced": true})),
            (
                "run_command",
                json!({"status": "completed", "exit_code": 0}),
            ),
            (
                "verify_task_tool",
                json!({"status": "passed", "exit_code": 0}),
            ),
        ] {
            tools
                .lock()
                .unwrap()
                .insert(name.to_string(), Arc::new(ReturnedTool { name, result }));
        }
        ToolExecutionGateway::with_policy(tools, ToolPolicy::new(ToolPolicyMode::Audit))
    }

    fn observe(
        gateway: &ToolExecutionGateway,
        runtime: &mut CognitiveRuntime,
        name: &str,
        arguments: Value,
    ) -> ToolExecutionReceipt {
        let receipt = gateway.execute(ToolExecutionRequest {
            call_id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            arguments,
        });
        runtime.observe_receipt(&receipt);
        receipt
    }

    #[test]
    fn structured_command_failures_are_not_successful_evidence() {
        let gateway = intelligence_gateway();
        for name in ["run_command", "verify_task_tool"] {
            for result in [
                json!({"status": "completed", "exit_code": 101}),
                json!({"status": "failed", "exit_code": 1}),
                json!({"timed_out": true}),
                json!({"success": false}),
                json!({"status": "interactive_prompt_detected"}),
            ] {
                let receipt = gateway.receipt(
                    ToolExecutionRequest {
                        call_id: "failed-check".into(),
                        name: name.into(),
                        arguments: if name == "run_command" {
                            json!({"CommandLine": "cargo test"})
                        } else {
                            json!({"command": "cargo test"})
                        },
                    },
                    ToolExecutionStatus::Succeeded,
                    Instant::now(),
                    result.clone(),
                );
                assert_eq!(
                    receipt.status,
                    ToolExecutionStatus::Failed,
                    "{name}: {result}"
                );
                assert!(!receipt.verification);
                assert_eq!(receipt.result, result);
            }
        }
    }

    #[test]
    fn only_completed_quality_commands_form_verification_evidence() {
        let gateway = intelligence_gateway();
        for (command, result, expected) in [
            ("cargo test", json!({"exit_code": 0}), true),
            ("pytest -v", json!({"exit_code": 0}), true),
            ("cargo test", json!({"status": "spawned"}), false),
            ("cargo test", json!({"exit_code": null}), false),
            ("echo success", json!({"exit_code": 0}), false),
            ("cargo test --help", json!({"exit_code": 0}), false),
            ("cargo test -- --list", json!({"exit_code": 0}), false),
            ("cargo test\necho success", json!({"exit_code": 0}), false),
            ("cargo test || echo success", json!({"exit_code": 0}), false),
        ] {
            let mut req = request("run_command");
            req.arguments = json!({"CommandLine": command});
            let receipt =
                gateway.receipt(req, ToolExecutionStatus::Succeeded, Instant::now(), result);
            assert_eq!(receipt.verification, expected, "{command}");
        }
    }

    #[test]
    fn real_gateway_receipts_drive_inspect_edit_verify_and_reverification() {
        let gateway = intelligence_gateway();
        let mut runtime =
            CognitiveRuntime::with_mode("修复 src/main.rs 的错误", ReasoningMode::Adaptive);
        observe(
            &gateway,
            &mut runtime,
            "view_file",
            json!({"path": "src/main.rs"}),
        );
        observe(
            &gateway,
            &mut runtime,
            "replace_in_file",
            json!({"old": "first", "new": "second"}),
        );
        observe(
            &gateway,
            &mut runtime,
            "run_command",
            json!({"CommandLine": "echo done"}),
        );
        let draft = "已修改相关代码，并核对任务目标和既有接口，下面说明验证结果及剩余限制。";
        assert!(matches!(
            runtime.review_completion(draft),
            CompletionDecision::Revise { .. }
        ));
        let check = observe(
            &gateway,
            &mut runtime,
            "verify_task_tool",
            json!({"command": "cargo test"}),
        );
        assert!(check.verification);
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
        observe(
            &gateway,
            &mut runtime,
            "replace_in_file",
            json!({"old": "second", "new": "third"}),
        );
        assert!(matches!(
            runtime.review_completion(draft),
            CompletionDecision::Blocked { .. }
        ));
        observe(
            &gateway,
            &mut runtime,
            "run_command",
            json!({"CommandLine": "cargo test"}),
        );
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
    }

    #[test]
    fn quality_checks_can_repeat_after_repairs_but_unchanged_loops_are_bounded() {
        let gateway = intelligence_gateway();
        let mut runtime = CognitiveRuntime::with_mode("修复错误", ReasoningMode::Adaptive);
        for revision in 0..7 {
            observe(
                &gateway,
                &mut runtime,
                "replace_in_file",
                json!({"revision": revision}),
            );
            let receipt = observe(
                &gateway,
                &mut runtime,
                "run_command",
                json!({"CommandLine": "cargo test"}),
            );
            assert!(receipt.verification, "repair {revision}");
        }
        for _ in 0..2 {
            assert!(
                observe(
                    &gateway,
                    &mut runtime,
                    "run_command",
                    json!({"CommandLine": "cargo test"})
                )
                .verification
            );
        }
        assert_eq!(
            observe(
                &gateway,
                &mut runtime,
                "run_command",
                json!({"CommandLine": "cargo test"})
            )
            .status,
            ToolExecutionStatus::RepetitionBlocked
        );
    }

    #[test]
    fn verification_only_objective_completes_without_writing_files() {
        let gateway = intelligence_gateway();
        let mut runtime = CognitiveRuntime::with_mode("运行测试", ReasoningMode::Adaptive);
        observe(
            &gateway,
            &mut runtime,
            "verify_task_tool",
            json!({"command": "cargo test"}),
        );
        assert_eq!(
            runtime.review_completion(
                "验证已结束，指定的质量检查返回成功退出码，检查范围和结果将在这里说明。"
            ),
            CompletionDecision::Accept
        );
        assert!(runtime.evolution_report(true).plan_closed);
    }

    #[test]
    fn decomposition_is_retained_per_run_but_never_counts_as_execution_evidence() {
        let gateway = intelligence_gateway();
        let mut runtime =
            CognitiveRuntime::with_mode("进化 superagent 的需求理解能力", ReasoningMode::Adaptive);
        let value = crate::services::brain::task_understanding::tests::sample();
        let receipt = observe(&gateway, &mut runtime, "plan_task", value.clone());
        assert_eq!(receipt.status, ToolExecutionStatus::Succeeded);
        assert!(!receipt.verification);
        for _ in 0..2 {
            let guidance = runtime.next_guidance().unwrap();
            assert!(guidance.contains("No invented framework rules"));
            assert!(guidance.contains("after: inspect"));
            assert!(guidance.contains("Which module owns intent routing?"));
        }
        assert!(matches!(
            runtime.review_completion("已修改全部实现"),
            CompletionDecision::Revise { .. }
        ));
        assert!(matches!(
            runtime.review_completion("已修改全部实现"),
            CompletionDecision::Blocked { .. }
        ));
        assert!(!runtime.evolution_report(false).plan_closed);
        let mut replacement = value;
        replacement["interpretation"] = json!("Narrow the scope after reading workspace evidence");
        observe(&gateway, &mut runtime, "plan_task", replacement);
        let guidance = runtime.next_guidance().unwrap();
        assert!(guidance.contains("Narrow the scope"));
        assert!(!guidance.contains("Interpretation: Improve task understanding"));
        let mut fresh_run = CognitiveRuntime::with_mode("你好", ReasoningMode::Adaptive);
        assert!(
            !fresh_run
                .next_guidance()
                .unwrap()
                .contains("Which module owns intent routing?")
        );
    }

    #[test]
    fn fast_mode_cannot_bypass_missing_execution_evidence() {
        let mut runtime = CognitiveRuntime::with_mode("修复错误", ReasoningMode::Fast);
        assert!(matches!(
            runtime.review_completion("已修改代码并完成验证"),
            CompletionDecision::Blocked { .. }
        ));
    }

    #[test]
    fn progress_tool_and_completion_gate_share_the_same_run_evidence() {
        let gateway = intelligence_gateway();
        let mut runtime =
            CognitiveRuntime::with_mode("修复 src/main.rs 的错误", ReasoningMode::Adaptive);
        gateway.registry.lock().unwrap().insert(
            "update_task_progress".into(),
            Arc::new(
                crate::services::brain::tools::task_progress::TaskProgressTool(
                    runtime.task_progress(),
                ),
            ),
        );
        let plan = json!({
            "interpretation": "Fix and verify the bug", "success_criteria": ["The behavior is verified"],
            "steps": [
                {"id": "inspect", "goal": "Read the source", "acceptance": "Read receipt", "evidence_kind": "observation"},
                {"id": "fix", "goal": "Fix the bug", "acceptance": "Mutation receipt", "depends_on": ["inspect"], "evidence_kind": "mutation"},
                {"id": "check", "goal": "Verify the fix", "acceptance": "Quality receipt", "depends_on": ["fix"], "evidence_kind": "verification"}
            ]
        });
        observe(&gateway, &mut runtime, "plan_task", plan);
        let read = observe(
            &gateway,
            &mut runtime,
            "view_file",
            json!({"path": "src/main.rs"}),
        );
        let write = observe(
            &gateway,
            &mut runtime,
            "replace_in_file",
            json!({"old": "broken", "new": "fixed"}),
        );
        let check = observe(
            &gateway,
            &mut runtime,
            "run_command",
            json!({"CommandLine": "cargo test"}),
        );
        let draft = "已修改相关实现，并通过针对性测试确认预期行为，下面说明修改范围和验证结果。";
        assert!(matches!(
            runtime.review_completion(draft),
            CompletionDecision::Revise { .. }
        ));
        assert!(!runtime.evolution_report(true).plan_closed);
        for (step, id) in [
            ("inspect", read.call_id),
            ("fix", write.call_id),
            ("check", check.call_id),
        ] {
            let receipt = observe(
                &gateway,
                &mut runtime,
                "update_task_progress",
                json!({
                    "action": "complete_step", "step_id": step, "receipt_ids": [id]
                }),
            );
            assert_eq!(
                receipt.status,
                ToolExecutionStatus::Succeeded,
                "{}",
                receipt.result
            );
        }
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
        assert!(runtime.evolution_report(true).plan_closed);
        observe(
            &gateway,
            &mut runtime,
            "replace_in_file",
            json!({"old": "fixed", "new": "changed again"}),
        );
        assert!(matches!(
            runtime.review_completion(draft),
            CompletionDecision::Blocked { .. }
        ));
    }

    #[test]
    fn proposal_only_runs_do_not_require_executing_future_implementation_steps() {
        let gateway = intelligence_gateway();
        let mut runtime = CognitiveRuntime::with_mode(
            "只做规划：阅读当前 README.md，提出重构方案，不修改源码",
            ReasoningMode::Adaptive,
        );
        observe(
            &gateway,
            &mut runtime,
            "view_file",
            json!({"path": "README.md"}),
        );
        observe(
            &gateway,
            &mut runtime,
            "plan_task",
            crate::services::brain::task_understanding::tests::sample(),
        );
        assert!(
            runtime
                .task_progress()
                .lock()
                .unwrap()
                .completion_gap()
                .is_some()
        );
        assert_eq!(runtime.review_completion("这是基于当前文档的改造提案，列出未来实施步骤、依赖关系、验收标准和需要继续调查的信息。本轮仅提供方案。"), CompletionDecision::Accept);
        assert!(runtime.evolution_report(true).plan_closed);
        assert!(
            runtime
                .next_guidance()
                .unwrap()
                .contains("Proposed future implementation steps are not authorized")
        );
        let mut generic =
            CognitiveRuntime::with_mode("只给方案：设计一个日程助手", ReasoningMode::Adaptive);
        assert_eq!(
            generic.review_completion(
                "先界定目标和数据来源，再设计提醒流程与验收条件；以下步骤是未来工作的建议。"
            ),
            CompletionDecision::Accept
        );
        let ordinary_execution =
            CognitiveRuntime::with_mode("修复包含“只做规划”字样的解析器", ReasoningMode::Adaptive);
        assert_eq!(
            ordinary_execution.task_kind(),
            crate::services::brain::run_plan::TaskKind::Execution
        );
    }

    #[test]
    fn revision_tool_preserves_progress_and_reopens_the_real_verification_gate() {
        let gateway = intelligence_gateway();
        let mut runtime = CognitiveRuntime::with_mode("修复 src/main.rs", ReasoningMode::Adaptive);
        gateway.registry.lock().unwrap().insert(
            "revise_task_plan".into(),
            Arc::new(
                crate::services::brain::tools::revise_task_plan::ReviseTaskPlanTool(
                    runtime.task_progress(),
                ),
            ),
        );
        gateway.registry.lock().unwrap().insert(
            "update_task_progress".into(),
            Arc::new(
                crate::services::brain::tools::task_progress::TaskProgressTool(
                    runtime.task_progress(),
                ),
            ),
        );
        observe(
            &gateway,
            &mut runtime,
            "plan_task",
            json!({
                "interpretation":"Fix the bug", "success_criteria":["Behavior fixed"],
                "steps":[{"id":"inspect","goal":"Read source","acceptance":"Read receipt","evidence_kind":"observation"}]
            }),
        );
        let read = observe(
            &gateway,
            &mut runtime,
            "view_file",
            json!({"path":"src/main.rs"}),
        );
        observe(
            &gateway,
            &mut runtime,
            "replace_in_file",
            json!({"old":"broken","new":"fixed"}),
        );
        for _ in 0..3 {
            assert!(
                observe(
                    &gateway,
                    &mut runtime,
                    "run_command",
                    json!({"CommandLine":"cargo test"})
                )
                .verification
            );
        }
        observe(
            &gateway,
            &mut runtime,
            "update_task_progress",
            json!({"action":"complete_step","step_id":"inspect","receipt_ids":[read.call_id]}),
        );
        let draft = "已修改实现，并通过针对性检查确认行为符合预期，下面说明变更范围及验证结果。";
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
        let patch = observe(
            &gateway,
            &mut runtime,
            "revise_task_plan",
            json!({"base_revision":1,"append_constraints":["Preserve parser.rs"]}),
        );
        assert_eq!(patch.status, ToolExecutionStatus::Succeeded);
        assert_eq!(patch.result["retained_steps"], json!(["inspect"]));
        // Even a plan lacking an explicit verification subgoal must re-open
        // the Run's canonical Verify phase when requirements change.
        assert!(matches!(
            runtime.review_completion(draft),
            CompletionDecision::Revise { .. }
        ));
        observe(
            &gateway,
            &mut runtime,
            "run_command",
            json!({"CommandLine":"cargo test"}),
        );
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
    }

    struct FixtureTool {
        outcome: &'static str,
    }

    impl ToolExecutor for FixtureTool {
        fn name(&self) -> &'static str {
            "fixture"
        }

        fn description(&self) -> &'static str {
            "test fixture"
        }

        fn parameters_schema(&self) -> Value {
            json!({ "type": "object" })
        }

        fn execute(&self, _args: Value) -> Result<Value, String> {
            match self.outcome {
                "ok" => Ok(json!({ "value": 42 })),
                "large" => Ok(json!({ "output": "x".repeat(2_000) })),
                "panic" => panic!("fixture panic"),
                _ => Err("fixture failed".to_string()),
            }
        }
    }

    fn registry(tool: FixtureTool) -> ToolRegistry {
        let mut tools: HashMap<String, Arc<dyn ToolExecutor>> = HashMap::new();
        tools.insert(tool.name().to_string(), Arc::new(tool));
        Arc::new(Mutex::new(tools))
    }

    fn request(name: &str) -> ToolExecutionRequest {
        ToolExecutionRequest {
            call_id: "call-1".to_string(),
            name: name.to_string(),
            arguments: json!({}),
        }
    }

    #[test]
    fn success_preserves_result_and_reinserts_tool() {
        let tools = registry(FixtureTool { outcome: "ok" });
        let gateway = ToolExecutionGateway::new(tools.clone());

        let receipt = gateway.execute(request("fixture"));

        assert_eq!(receipt.status, ToolExecutionStatus::Succeeded);
        assert_eq!(receipt.result, json!({ "value": 42 }));
        assert!(tools.lock().unwrap().contains_key("fixture"));
    }

    #[test]
    fn failure_and_panic_are_classified_and_tool_is_reinserted() {
        for (outcome, expected) in [
            ("error", ToolExecutionStatus::Failed),
            ("panic", ToolExecutionStatus::Panicked),
        ] {
            let tools = registry(FixtureTool { outcome });
            let receipt = ToolExecutionGateway::new(tools.clone()).execute(request("fixture"));
            assert_eq!(receipt.status, expected);
            assert!(receipt.result.get("error").is_some());
            assert!(tools.lock().unwrap().contains_key("fixture"));
        }
    }

    #[test]
    fn missing_tool_has_a_stable_error_shape() {
        let tools: ToolRegistry = Arc::new(Mutex::new(HashMap::new()));
        let receipt = ToolExecutionGateway::new(tools).execute(request("missing"));

        assert_eq!(receipt.status, ToolExecutionStatus::NotFound);
        assert_eq!(receipt.result["kind"], "not_found");
    }

    #[test]
    fn oversized_result_is_bounded_with_metadata() {
        let tools = registry(FixtureTool { outcome: "large" });
        let dir = tempdir().unwrap();
        let store = ToolResultStore::new(dir.path().to_path_buf());
        let receipt = ToolExecutionGateway::with_max_result_chars(tools, 120, store.clone())
            .execute(request("fixture"));

        assert!(receipt.truncated);
        assert!(receipt.original_result_chars > 120);
        assert_eq!(receipt.result["truncated"], true);
        let reference = receipt.stored_result_reference.unwrap();
        let restored = store.read_slice(&reference, 0, 10_000).unwrap();
        assert_eq!(restored.total_chars, receipt.original_result_chars);
    }

    #[test]
    fn read_only_policy_denies_before_tool_is_removed_or_executed() {
        let tools = registry(FixtureTool { outcome: "ok" });
        let gateway = ToolExecutionGateway::with_policy(
            tools.clone(),
            ToolPolicy::new(ToolPolicyMode::ReadOnly),
        );
        let receipt = gateway.execute(request("fixture"));

        assert_eq!(receipt.status, ToolExecutionStatus::PolicyDenied);
        assert_eq!(receipt.risk, ToolRisk::Unclassified);
        assert!(tools.lock().unwrap().contains_key("fixture"));
    }

    #[test]
    fn approval_is_bound_to_request_and_consumed_once() {
        let tools = registry(FixtureTool { outcome: "ok" });
        let directory = tempdir().unwrap();
        let approval_store = ToolApprovalStore::new(directory.path().to_path_buf());
        let gateway = ToolExecutionGateway::with_policy_and_approval_store(
            tools.clone(),
            ToolPolicy::new(ToolPolicyMode::Approval),
            approval_store.clone(),
        );

        let first = gateway.execute(request("fixture"));
        assert_eq!(first.status, ToolExecutionStatus::ApprovalRequired);
        let approval_id = first.result["approval_id"].as_str().unwrap();
        approval_store.approve(approval_id).unwrap();

        let second = gateway.execute(request("fixture"));
        assert_eq!(second.status, ToolExecutionStatus::Succeeded);

        let third = gateway.execute(request("fixture"));
        assert_eq!(third.status, ToolExecutionStatus::ApprovalRequired);
        assert_ne!(third.result["approval_id"], approval_id);
        assert!(tools.lock().unwrap().contains_key("fixture"));
    }

    #[test]
    fn repeated_exact_calls_are_blocked_before_tool_removal() {
        let tools = registry(FixtureTool { outcome: "ok" });
        let gateway = ToolExecutionGateway::new(tools.clone());
        for _ in 0..3 {
            assert_eq!(
                gateway.execute(request("fixture")).status,
                ToolExecutionStatus::Succeeded
            );
        }
        let blocked = gateway.execute(request("fixture"));
        assert_eq!(blocked.status, ToolExecutionStatus::RepetitionBlocked);
        assert_eq!(blocked.result["kind"], "repetition_blocked");
        assert!(tools.lock().unwrap().contains_key("fixture"));
    }
}
