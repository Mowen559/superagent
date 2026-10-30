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

fn extract_actionable_diagnostic(text: &str) -> Option<String> {
    let lower = text.to_lowercase();

    // 1. 目标文本替换失配（replace_in_file / surgical edit failure）
    if lower.contains("target content not found")
        || lower.contains("target string not found")
        || lower.contains("target not found")
    {
        return Some("Diagnostic insight: The target text to replace was not found in the file. File content or line endings (CRLF/LF) may differ. Use `view_file` to read the exact lines and whitespace before calling `replace_in_file`.".to_string());
    }

    // 2. 爬虫反爬风控、403 或验证码拦截
    if lower.contains("cloudflare")
        || lower.contains("just a moment")
        || lower.contains("sec_server")
        || lower.contains("403 forbidden")
        || lower.contains("access denied")
        || text.contains("风控")
        || text.contains("验证码")
    {
        return Some("Diagnostic insight: Target endpoint triggered anti-bot protection (403/Cloudflare/sec_server). Do NOT blindly repeat static HTTP requests! Pivot to: 1) Probe local Python engine (http://127.0.0.1:18000) for headless browser rendering; 2) Switch to mobile app/mini-program REST API; 3) Use webview sniffer protocol.".to_string());
    }

    // 3. 海阔沙箱 DOM 禁忌
    if lower.contains("window is not defined")
        || lower.contains("document is not defined")
        || lower.contains("fetch is not defined")
    {
        return Some("Diagnostic insight: Hiker sandbox violation! Naked DOM APIs (window/document/fetch) do not exist in the Hiker engine. Use isomorphic functions: `request(url, options)`, `pdfa`, `pdfh`, `pd` instead.".to_string());
    }

    // 4. 编译器/语法解析错误定位（Rust, TypeScript, Python, Node）
    let lines: Vec<&str> = text.lines().collect();
    for line in lines.iter().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("error[")
            || trimmed.starts_with("error:")
            || trimmed.contains("TS2")
            || trimmed.contains("TS1")
            || trimmed.starts_with("SyntaxError")
            || trimmed.starts_with("ReferenceError")
            || trimmed.starts_with("TypeError")
        {
            return Some(format!("Diagnostic insight: Located compiler/runtime issue: '{}'. Use `view_file` to inspect the referenced line and make surgical corrections.", trimmed));
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
        let repetition = {
            let mut guard = self.loop_guard.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if is_verification_request(&req.name, &req.arguments) {
                guard.record_verification(&req.name, &req.arguments, self.workspace_revision.load(Ordering::Relaxed))
            } else {
                guard.record(&req.name, &req.arguments)
            }
        };
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
            && ((matches!(risk, ToolRisk::WorkspaceMutation | ToolRisk::Destructive) && !verification)
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

        let mut enriched_result = final_result;
        if let Some(obj) = enriched_result.as_object_mut() {
            let has_issue = status == ToolExecutionStatus::Failed
                || obj.get("exit_code").and_then(Value::as_i64).unwrap_or(0) != 0
                || obj.contains_key("error");

            if has_issue {
                let diagnostic = obj.get("stderr")
                    .and_then(Value::as_str)
                    .and_then(extract_actionable_diagnostic)
                    .or_else(|| {
                        obj.get("error")
                            .and_then(Value::as_str)
                            .and_then(extract_actionable_diagnostic)
                    })
                    .or_else(|| {
                        obj.get("stdout")
                            .and_then(Value::as_str)
                            .and_then(extract_actionable_diagnostic)
                    })
                    .or_else(|| {
                        obj.get("output")
                            .and_then(Value::as_str)
                            .and_then(extract_actionable_diagnostic)
                    });

                if let Some(diag) = diagnostic {
                    obj.insert("diagnostic_insight".to_string(), json!(diag));
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
    fn alias_quality_checks_do_not_advance_their_own_verification_generation() {
        let gateway = intelligence_gateway();
        for index in 0..3 {
            let receipt = gateway.execute(ToolExecutionRequest {
                call_id: format!("alias-check-{index}"), name: "bash".into(),
                arguments: json!({"cmd": "cargo test"}),
            });
            assert_eq!(receipt.name, "run_command");
            assert_eq!(receipt.status, ToolExecutionStatus::Succeeded);
            assert!(receipt.verification);
        }
        let blocked = gateway.execute(ToolExecutionRequest {
            call_id: "unchanged-check".into(), name: "bash".into(), arguments: json!({"cmd": "cargo test"}),
        });
        assert_eq!(blocked.status, ToolExecutionStatus::RepetitionBlocked);
        let edited = gateway.execute(ToolExecutionRequest {
            call_id: "edit-after-check".into(), name: "replace_in_file".into(), arguments: json!({"path": "source.rs", "search_text": "old", "replace_text": "changed"}),
        });
        assert_eq!(edited.status, ToolExecutionStatus::Succeeded);
        let fresh = gateway.execute(ToolExecutionRequest {
            call_id: "fresh-check".into(), name: "bash".into(), arguments: json!({"cmd": "cargo test"}),
        });
        assert_eq!(fresh.status, ToolExecutionStatus::Succeeded);
        assert!(fresh.verification);
    }

    #[test]
    fn parallel_observation_receipts_cannot_replace_post_change_verification() {
        let gateway = Arc::new(intelligence_gateway());
        let mut runtime = CognitiveRuntime::with_mode("实现并验证配置变更", ReasoningMode::Adaptive);
        let parallel_read = |prefix: &str| {
            let handles: Vec<_> = (0..2).map(|index| {
                let gateway = gateway.clone();
                let call_id = format!("{prefix}-{index}");
                std::thread::spawn(move || gateway.execute(ToolExecutionRequest {
                    call_id, name: "view_file".into(), arguments: json!({"AbsolutePath": format!("config-{index}.rs")}),
                }))
            }).collect();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
        };
        for receipt in parallel_read("inspect") { runtime.observe_receipt(&receipt); }
        let mutation = gateway.execute(ToolExecutionRequest {
            call_id: "actual-change".into(), name: "replace_in_file".into(), arguments: json!({"path":"config.rs", "search_text":"old", "replace_text":"updated"}),
        });
        assert_eq!(mutation.status, ToolExecutionStatus::Succeeded);
        runtime.observe_receipt(&mutation);
        for receipt in parallel_read("read-after-change") { runtime.observe_receipt(&receipt); }
        let draft = "配置修改已经完成，并通过有记录的质量检查验证，保留已有接口和未改动部分。";
        assert!(matches!(runtime.review_completion(draft), CompletionDecision::Revise { .. }));
        let check = gateway.execute(ToolExecutionRequest {
            call_id: "quality-check".into(), name: "run_command".into(), arguments: json!({"CommandLine":"cargo test"}),
        });
        assert!(check.verification, "expected canonical quality receipt: {check:?}");
        runtime.observe_receipt(&check);
        assert_eq!(runtime.review_completion(draft), CompletionDecision::Accept);
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

    #[test]
    fn diagnostic_insight_covers_multiple_failure_patterns() {
        assert!(extract_actionable_diagnostic("error[E0432]: unresolved import `foo`").is_some());
        assert!(extract_actionable_diagnostic("Error: target content not found in file").is_some());
        assert!(extract_actionable_diagnostic("403 Forbidden: Cloudflare anti-bot verification required").is_some());
        assert!(extract_actionable_diagnostic("ReferenceError: window is not defined in sandbox").is_some());
        assert!(extract_actionable_diagnostic("All tests passed successfully").is_none());
    }
}
