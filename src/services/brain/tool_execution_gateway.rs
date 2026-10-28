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


/// 将大模型常见的行业通用工具名称映射为 SuperAgent 内部标准命名
pub fn canonicalize_tool_name(name: &str) -> &'static str {
    match name.trim().to_ascii_lowercase().as_str() {
        "read_file" | "cat" | "view" | "read" | "view_file_content" | "read_file_content" | "get_file_content" => "view_file",
        "write_file" | "create_file" | "write" | "write_to_file" | "save_file" | "new_file" => "write_to_file",
        "edit_file" | "replace_in_file" | "patch_file" | "replace_text" | "edit" | "patch" => "replace_in_file",
        "multi_replace" | "multi_replace_file_content" | "batch_replace" => "multi_replace_file_content",
        "bash" | "sh" | "terminal" | "shell" | "exec" | "execute_command" | "run_cmd" | "run_command" => "run_command",
        "grep" | "search" | "grep_search" | "search_files" | "search_in_files" => "grep_search",
        "find" | "find_by_name" | "find_file" | "find_files" | "glob" | "locate" => "find_by_name",
        "ls" | "dir" | "list_dir" | "list_directory" | "list_files" => "list_dir",
        "todo" | "manage_task" | "task" | "tasks" | "task_list" | "plan" => "manage_task",
        "subagent" | "invoke_subagent" | "call_subagent" | "agent" => "invoke_subagent",
        "outline" | "view_outline" | "file_outline" => "view_outline",
        "ask" | "ask_user" | "question" => "ask_user",
        _ => "",
    }
}

/// 针对核心工具将大模型可能传递的不同风格入参归一化为工具预期字段
pub fn normalize_tool_arguments(canonical_name: &str, arguments: &Value) -> Value {
    let mut args = arguments.clone();
    let Some(obj) = args.as_object_mut() else {
        return args;
    };

    match canonical_name {
        "view_file" => {
            if !obj.contains_key("AbsolutePath") {
                let candidate = obj.get("path")
                    .or_else(|| obj.get("file_path"))
                    .or_else(|| obj.get("filePath"))
                    .or_else(|| obj.get("file"))
                    .or_else(|| obj.get("TargetFile"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("AbsolutePath".to_string(), c);
                }
            }
        }
        "write_to_file" => {
            if !obj.contains_key("TargetFile") {
                let candidate = obj.get("path")
                    .or_else(|| obj.get("file_path"))
                    .or_else(|| obj.get("filePath"))
                    .or_else(|| obj.get("file"))
                    .or_else(|| obj.get("AbsolutePath"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("TargetFile".to_string(), c);
                }
            }
            if !obj.contains_key("CodeContent") {
                let candidate = obj.get("content")
                    .or_else(|| obj.get("text"))
                    .or_else(|| obj.get("code"))
                    .or_else(|| obj.get("body"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("CodeContent".to_string(), c);
                }
            }
        }
        "replace_in_file" => {
            if !obj.contains_key("path") {
                let candidate = obj.get("TargetFile")
                    .or_else(|| obj.get("file_path"))
                    .or_else(|| obj.get("filePath"))
                    .or_else(|| obj.get("file"))
                    .or_else(|| obj.get("AbsolutePath"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("path".to_string(), c);
                }
            }
            if !obj.contains_key("search_text") {
                let candidate = obj.get("old_text")
                    .or_else(|| obj.get("target"))
                    .or_else(|| obj.get("find"))
                    .or_else(|| obj.get("search"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("search_text".to_string(), c);
                }
            }
            if !obj.contains_key("replace_text") {
                let candidate = obj.get("new_text")
                    .or_else(|| obj.get("replacement"))
                    .or_else(|| obj.get("replace"))
                    .or_else(|| obj.get("content"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("replace_text".to_string(), c);
                }
            }
        }
        "run_command" => {
            if !obj.contains_key("CommandLine") {
                let candidate = obj.get("command")
                    .or_else(|| obj.get("command_line"))
                    .or_else(|| obj.get("cmd"))
                    .or_else(|| obj.get("script"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("CommandLine".to_string(), c);
                }
            }
        }
        "manage_task" => {
            if !obj.contains_key("Action") {
                let candidate = obj.get("action")
                    .or_else(|| obj.get("operation"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("Action".to_string(), c);
                }
            }
            if !obj.contains_key("Content") {
                let candidate = obj.get("content")
                    .or_else(|| obj.get("text"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("Content".to_string(), c);
                }
            }
        }
        "find_by_name" => {
            if !obj.contains_key("pattern") {
                let candidate = obj.get("name")
                    .or_else(|| obj.get("query"))
                    .or_else(|| obj.get("file_pattern"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("pattern".to_string(), c);
                }
            }
        }
        "grep_search" => {
            if !obj.contains_key("query") {
                let candidate = obj.get("pattern")
                    .or_else(|| obj.get("search_term"))
                    .or_else(|| obj.get("text"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("query".to_string(), c);
                }
            }
        }
        "list_dir" => {
            if !obj.contains_key("path") {
                let candidate = obj.get("dir")
                    .or_else(|| obj.get("directory"))
                    .or_else(|| obj.get("folder"))
                    .cloned();
                if let Some(c) = candidate {
                    obj.insert("path".to_string(), c);
                }
            }
        }
        _ => {}
    }

    args
}

fn find_fuzzy_tool_match(input: &str, registered_tools: &[String]) -> Option<String> {
    let lower = input.trim().to_ascii_lowercase();
    for tool in registered_tools {
        if tool.contains(&lower) || lower.contains(tool) {
            return Some(tool.clone());
        }
    }
    None
}

fn extract_actionable_diagnostic(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().collect();
    for line in lines.iter().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("error[") || trimmed.starts_with("error:") || trimmed.contains("TS") || trimmed.starts_with("SyntaxError") {
            return Some(format!("Diagnostic hint: Found error '{}'. Use view_file and replace_in_file to surgically correct the referenced location.", trimmed));
        }
    }
    None
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

        // 透明别名映射与参数智能归一化
        let canonical = canonicalize_tool_name(&request.name);
        let effective_name = if !canonical.is_empty() {
            canonical.to_string()
        } else {
            request.name.clone()
        };
        let effective_args = normalize_tool_arguments(&effective_name, &request.arguments);

        let req = ToolExecutionRequest {
            call_id: request.call_id.clone(),
            name: effective_name,
            arguments: effective_args,
        };

        let (risk, decision) = self.policy.evaluate(&req.name, &req.arguments);
        let repetition = self
            .loop_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record(&req.name, &req.arguments);
        if let Err(block) = repetition {
            return self.receipt(
                req,
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
                req,
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
        let in_sandbox = req
            .arguments
            .get("TargetFile")
            .or_else(|| req.arguments.get("path"))
            .or_else(|| req.arguments.get("file_path"))
            .and_then(Value::as_str)
            .map(|p| {
                crate::domains::version_control::diff_transaction::is_path_in_sandbox(
                    std::path::Path::new(p),
                )
            })
            .unwrap_or(false);

        if decision == ToolPolicyDecision::RequireApproval && !in_sandbox {
            match self
                .approval_store
                .consume_matching(&req.name, &req.arguments)
            {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return match self
                        .approval_store
                        .request(&req.name, &req.arguments, risk)
                    {
                        Ok(approval) => self.receipt(
                            req,
                            ToolExecutionStatus::ApprovalRequired,
                            started,
                            json!({
                                "error": "tool approval required",
                                "kind": "approval_required",
                                "approval_id": approval.id,
                                "tool": approval.tool_name,
                                "risk": approval.risk,
                                "expires_at": approval.expires_at,
                                "diff_payload": approval.diff_payload,
                                "recovery_hint": "A tool approval was requested. For autonomous execution, you can proceed with other non-destructive analysis tools or request user clearance.",
                                "control_plane": {
                                    "approve": format!("POST /api/tool-approvals/{}/approve", approval.id),
                                    "approve_session": format!("POST /api/tool-approvals/{}/approve-session", approval.id),
                                    "deny": format!("POST /api/tool-approvals/{}/deny", approval.id),
                                    "required_header": "X-Hub-Approval: approve|approve-session|deny",
                                }
                            }),
                        ),
                        Err(error) => self.receipt(
                            req,
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
                        req,
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
            .get(&req.name)
            .cloned();

        let Some(tool) = tool else {
            let available_names: Vec<String> = self
                .registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .keys()
                .cloned()
                .collect();
            let fuzzy = find_fuzzy_tool_match(&request.name, &available_names);
            let core_summary = "view_file (read_file), write_to_file (write_file), replace_in_file (edit_file), run_command (bash), grep_search, find_by_name, list_dir, manage_task";
            let recovery_hint = if let Some(suggested) = fuzzy {
                format!("Tool '{}' not found. Did you mean '{}'? Supported primary tools: {}", request.name, suggested, core_summary)
            } else {
                format!("Tool '{}' not found. Supported primary tools: {}", request.name, core_summary)
            };

            return self.receipt(
                req,
                ToolExecutionStatus::NotFound,
                started,
                json!({
                    "error": format!("tool not found: '{}'", request.name),
                    "kind": "not_found",
                    "recovery_hint": recovery_hint
                }),
            );
        };

        let execution = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tool.execute(req.arguments.clone())
        }));

        match execution {
            Ok(Ok(result)) => {
                self.receipt(req, ToolExecutionStatus::Succeeded, started, result)
            }
            Ok(Err(error)) => self.receipt(
                req,
                ToolExecutionStatus::Failed,
                started,
                json!({ "error": error, "kind": "execution_failed" }),
            ),
            Err(_) => self.receipt(
                req,
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

        let mut enriched_result = final_result;
        if let Some(obj) = enriched_result.as_object_mut() {
            if status == ToolExecutionStatus::Failed || obj.get("exit_code").and_then(Value::as_i64).unwrap_or(0) != 0 {
                if let Some(stderr) = obj.get("stderr").and_then(Value::as_str) {
                    if let Some(diag) = extract_actionable_diagnostic(stderr) {
                        obj.insert("diagnostic_insight".to_string(), json!(diag));
                    }
                }
            }
        }

        ToolExecutionReceipt {
            call_id: request.call_id,
            name: request.name,
            status,
            duration_ms: started.elapsed().as_millis() as u64,
            result: enriched_result,
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
    #[test]
    fn transparently_resolves_common_aliases_and_normalizes_args() {
        let tools = registry(FixtureTool { outcome: "ok" });
        // 在 registry 里注册 view_file 和 write_to_file 模拟核心工具
        {
            let mut guard = tools.lock().unwrap();
            guard.insert("view_file".to_string(), Arc::new(FixtureTool { outcome: "ok" }));
            guard.insert("write_to_file".to_string(), Arc::new(FixtureTool { outcome: "ok" }));
        }
        let gateway = ToolExecutionGateway::new(tools);

        // 1. read_file 别名透明解析为 view_file，且参数 path 自动映射为 AbsolutePath
        let receipt_read = gateway.execute(ToolExecutionRequest {
            call_id: "c1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "/foo/bar.txt" }),
        });
        assert_eq!(receipt_read.status, ToolExecutionStatus::Succeeded);
        assert_eq!(receipt_read.name, "view_file");

        // 2. write_file 别名透明解析为 write_to_file，且参数 content 映射为 CodeContent
        let receipt_write = gateway.execute(ToolExecutionRequest {
            call_id: "c2".to_string(),
            name: "write_file".to_string(),
            arguments: json!({ "file": "/foo/bar.txt", "content": "hello" }),
        });
        assert_eq!(receipt_write.status, ToolExecutionStatus::Succeeded);
        assert_eq!(receipt_write.name, "write_to_file");
    }

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
