use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::tool_approval_store::ToolApprovalStore;
use crate::services::brain::tool_loop_guard::ToolLoopGuard;
use crate::services::brain::tool_policy::{
    classify_tool_risk, ToolPolicy, ToolPolicyDecision, ToolPolicyMode, ToolRisk,
};
use crate::services::brain::tool_result_store::ToolResultStore;
use serde_json::{json, Value};
use std::collections::HashMap;
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
        }
    }

    pub fn begin_run(&self) {
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
        }
    }

    pub fn execute(&self, request: ToolExecutionRequest) -> ToolExecutionReceipt {
        let started = Instant::now();
        let (risk, decision) = self.policy.evaluate(&request.name, &request.arguments);
        let repetition = self
            .loop_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record(&request.name, &request.arguments);
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
        let verification = status == ToolExecutionStatus::Succeeded
            && is_verification_request(&request.name, &request.arguments);
        let risk = classify_tool_risk(&request.name, &request.arguments);
        
        let serialized_original = serde_json::to_string(&result).unwrap_or_default();
        let original_result_chars = serialized_original.chars().count();
        let needs_truncation = original_result_chars > self.max_result_chars;
        let mut stored_result_reference = None;

        let (final_result, truncated) = if needs_truncation {
            let stored = self.result_store.persist(&serialized_original).ok();
            stored_result_reference = stored.as_ref().map(|item| item.reference.clone());
            
            let (mut compressed, was_truncated) = crate::services::brain::context_interceptor::intelligent_compress(
                &request, result, self.max_result_chars
            );
            if was_truncated {
                if let Some(obj) = compressed.as_object_mut() {
                    obj.insert("full_result_reference".to_string(), json!(stored_result_reference));
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
    if name != "run_command"
        || arguments
            .get("RunInBackground")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return false;
    }
    let Some(command) = arguments.get("CommandLine").and_then(Value::as_str) else {
        return false;
    };
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || ["&&", "||", ";", "|", ">", "<"]
            .iter()
            .any(|operator| normalized.contains(operator))
    {
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
    use tempfile::tempdir;

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
