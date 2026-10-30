use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    ReadOnly,
    Network,
    WorkspaceMutation,
    ProcessControl,
    Destructive,
    Unclassified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicyMode {
    /// Fully autonomous: every tool call is allowed (ZCode "yolo").
    Audit,
    /// Plan/research only: read-only tools pass, everything else is denied.
    ReadOnly,
    /// Confirm before changes: read-only passes, all mutations/approvals gate.
    Approval,
    /// Auto-apply edits: file mutations pass; network/process/destructive gate.
    AutoEdit,
    /// Guarded autonomy: everything passes except destructive operations gate.
    Guarded,
}

/// Web-facing collaboration modes (mirrors ZCode's permission modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    Plan,
    Build,
    Edit,
    Yolo,
    Guarded,
}

impl WorkMode {
    /// All modes accepted from the HTTP surface, in cycle order.
    pub const ALL: [&'static str; 5] = ["plan", "build", "edit", "yolo", "guarded"];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "plan" | "read-only" | "readonly" => Some(WorkMode::Plan),
            "build" | "approval" | "require-approval" => Some(WorkMode::Build),
            "edit" | "auto-edit" | "accept-edits" => Some(WorkMode::Edit),
            "yolo" | "audit" | "full-auto" | "bypass" => Some(WorkMode::Yolo),
            "guarded" => Some(WorkMode::Guarded),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            WorkMode::Plan => "plan",
            WorkMode::Build => "build",
            WorkMode::Edit => "edit",
            WorkMode::Yolo => "yolo",
            WorkMode::Guarded => "guarded",
        }
    }

    pub fn policy_mode(&self) -> ToolPolicyMode {
        match self {
            WorkMode::Plan => ToolPolicyMode::ReadOnly,
            WorkMode::Build => ToolPolicyMode::Approval,
            WorkMode::Edit => ToolPolicyMode::AutoEdit,
            WorkMode::Yolo => ToolPolicyMode::Audit,
            WorkMode::Guarded => ToolPolicyMode::Guarded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicyDecision {
    Allow,
    Deny,
    RequireApproval,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolPolicy {
    mode: ToolPolicyMode,
}

impl ToolPolicy {
    pub fn from_env() -> Self {
        let mode = match std::env::var("HUB_TOOL_POLICY_MODE")
            .unwrap_or_else(|_| "audit".to_string())
            .to_lowercase()
            .as_str()
        {
            "read-only" | "readonly" | "deny-mutations" | "plan" => ToolPolicyMode::ReadOnly,
            "approval" | "require-approval" | "build" => ToolPolicyMode::Approval,
            "auto-edit" | "accept-edits" | "edit" => ToolPolicyMode::AutoEdit,
            "guarded" => ToolPolicyMode::Guarded,
            _ => ToolPolicyMode::Audit,
        };
        Self { mode }
    }

    pub fn new(mode: ToolPolicyMode) -> Self {
        Self { mode }
    }

    /// Override the active mode at runtime (e.g. per-run work mode from HTTP).
    pub fn set_mode(&mut self, mode: ToolPolicyMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> ToolPolicyMode {
        self.mode
    }

    pub fn evaluate(&self, name: &str, arguments: &Value) -> (ToolRisk, ToolPolicyDecision) {
        let risk = classify_tool_risk(name, arguments);
        let decision = match self.mode {
            ToolPolicyMode::Audit => ToolPolicyDecision::Allow,
            ToolPolicyMode::ReadOnly if risk == ToolRisk::ReadOnly => ToolPolicyDecision::Allow,
            ToolPolicyMode::ReadOnly => ToolPolicyDecision::Deny,
            ToolPolicyMode::Approval if risk == ToolRisk::ReadOnly => ToolPolicyDecision::Allow,
            ToolPolicyMode::Approval => ToolPolicyDecision::RequireApproval,
            ToolPolicyMode::AutoEdit if risk == ToolRisk::ReadOnly => ToolPolicyDecision::Allow,
            ToolPolicyMode::AutoEdit if risk == ToolRisk::WorkspaceMutation => {
                ToolPolicyDecision::Allow
            }
            ToolPolicyMode::AutoEdit => ToolPolicyDecision::RequireApproval,
            ToolPolicyMode::Guarded if risk == ToolRisk::Destructive => {
                ToolPolicyDecision::RequireApproval
            }
            ToolPolicyMode::Guarded => ToolPolicyDecision::Allow,
        };
        (risk, decision)
    }
}

pub fn classify_tool_risk(name: &str, arguments: &Value) -> ToolRisk {
    match name {
        "view_file"
        | "plan_task"
        | "update_task_progress"
        | "revise_task_plan"
        | "list_dir"
        | "grep_search"
        | "find_by_name"
        | "search_graph"
        | "knowledge_search_tool"
        | "super_memory"
        | "lsp"
        | "read_tool_result"
        | "check_worker_status" => ToolRisk::ReadOnly,
        "knowledge_corpus_tool" if corpus_operation(arguments) == "list" => ToolRisk::ReadOnly,
        "knowledge_corpus_tool" if corpus_operation(arguments) == "remove" => ToolRisk::Destructive,
        "knowledge_corpus_tool" => ToolRisk::WorkspaceMutation,
        "web_fetch" | "headless_browser" | "mcp_client" | "harness_acp" | "hermes_workflow"
        | "openclaw_rtk" => ToolRisk::Network,
        "write_to_file"
        | "replace_in_file"
        | "multi_replace_file_content"
        | "knowledge_ingest_tool"
        | "manage_task" => ToolRisk::WorkspaceMutation,
        "run_command" if is_read_only_command(arguments) => ToolRisk::ReadOnly,
        "run_command" if is_safe_sandbox_command(arguments) => ToolRisk::WorkspaceMutation,
        "run_command" | "verify_task_tool" | "manage_process" | "invoke_subagent" | "intervene_worker"
        | "reload_plugins" | "request_reload" | "skill_tool" | "native_harness" => {
            ToolRisk::ProcessControl
        }
        "shadow_git" if destructive_action(arguments) => ToolRisk::Destructive,
        "shadow_git" => ToolRisk::WorkspaceMutation,
        "ask_user" => ToolRisk::ReadOnly,
        _ => ToolRisk::Unclassified,
    }
}

fn destructive_action(arguments: &Value) -> bool {
    arguments
        .get("action")
        .or_else(|| arguments.get("Action"))
        .and_then(Value::as_str)
        .map(|action| {
            matches!(
                action.to_ascii_lowercase().as_str(),
                "restore" | "delete" | "reset"
            )
        })
        .unwrap_or(false)
}

fn corpus_operation(arguments: &Value) -> String {
    arguments
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    #[test]
    fn allows_safe_dev_commands_without_approval_in_auto_edit_mode() {
        let policy = ToolPolicy::new(ToolPolicyMode::AutoEdit);
        assert_eq!(
            policy.evaluate("run_command", &json!({ "command_line": "cargo test" })).1,
            ToolPolicyDecision::Allow
        );
        assert_eq!(
            policy.evaluate("run_command", &json!({ "CommandLine": "npm test" })).1,
            ToolPolicyDecision::Allow
        );
        assert_eq!(
            policy.evaluate("run_command", &json!({ "command": "git status" })).1,
            ToolPolicyDecision::Allow
        );
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_argument_sensitive_destructive_actions() {
        assert_eq!(
            classify_tool_risk("shadow_git", &json!({ "action": "status" })),
            ToolRisk::WorkspaceMutation
        );
        assert_eq!(
            classify_tool_risk("shadow_git", &json!({ "action": "restore" })),
            ToolRisk::Destructive
        );
        assert_eq!(
            classify_tool_risk("knowledge_corpus_tool", &json!({ "operation": "list" })),
            ToolRisk::ReadOnly
        );
        assert_eq!(
            classify_tool_risk("knowledge_corpus_tool", &json!({ "operation": "rebuild" })),
            ToolRisk::WorkspaceMutation
        );
        assert_eq!(
            classify_tool_risk("knowledge_corpus_tool", &json!({ "operation": "remove" })),
            ToolRisk::Destructive
        );
    }

    #[test]
    fn read_only_mode_denies_mutation_network_process_and_unknown_tools() {
        let policy = ToolPolicy::new(ToolPolicyMode::ReadOnly);
        assert_eq!(
            policy.evaluate("view_file", &json!({})).1,
            ToolPolicyDecision::Allow
        );
        for name in [
            "write_to_file",
            "run_command",
            "web_fetch",
            "unknown_plugin",
        ] {
            assert_eq!(
                policy.evaluate(name, &json!({})).1,
                ToolPolicyDecision::Deny
            );
        }
    }

    #[test]
    fn approval_mode_only_allows_read_only_tools_without_approval() {
        let policy = ToolPolicy::new(ToolPolicyMode::Approval);
        assert_eq!(
            policy.evaluate("view_file", &json!({})).1,
            ToolPolicyDecision::Allow
        );
        for name in [
            "write_to_file",
            "run_command",
            "web_fetch",
            "unknown_plugin",
        ] {
            assert_eq!(
                policy.evaluate(name, &json!({})).1,
                ToolPolicyDecision::RequireApproval
            );
        }
    }

    #[test]
    fn auto_edit_mode_allows_workspace_mutations_but_gates_processes() {
        let policy = ToolPolicy::new(ToolPolicyMode::AutoEdit);
        assert_eq!(
            policy.evaluate("replace_in_file", &json!({})).1,
            ToolPolicyDecision::Allow
        );
        assert_eq!(
            policy.evaluate("write_to_file", &json!({})).1,
            ToolPolicyDecision::Allow
        );
        for name in ["run_command", "web_fetch", "unknown_plugin"] {
            assert_eq!(
                policy.evaluate(name, &json!({})).1,
                ToolPolicyDecision::RequireApproval
            );
        }
    }

    #[test]
    fn guarded_mode_only_gates_destructive_actions() {
        let policy = ToolPolicy::new(ToolPolicyMode::Guarded);
        for name in [
            "replace_in_file",
            "run_command",
            "web_fetch",
            "unknown_plugin",
        ] {
            assert_eq!(
                policy.evaluate(name, &json!({})).1,
                ToolPolicyDecision::Allow
            );
        }
        assert_eq!(
            policy
                .evaluate("shadow_git", &json!({ "action": "restore" }))
                .1,
            ToolPolicyDecision::RequireApproval
        );
        assert_eq!(
            policy
                .evaluate("knowledge_corpus_tool", &json!({ "operation": "remove" }))
                .1,
            ToolPolicyDecision::RequireApproval
        );
    }

    #[test]
    fn work_modes_map_to_expected_policy_semantics() {
        use super::WorkMode;

        assert_eq!(
            WorkMode::parse("plan").unwrap().policy_mode(),
            ToolPolicyMode::ReadOnly
        );
        assert_eq!(
            WorkMode::parse("Build").unwrap().policy_mode(),
            ToolPolicyMode::Approval
        );
        assert_eq!(
            WorkMode::parse("accept-edits").unwrap().policy_mode(),
            ToolPolicyMode::AutoEdit
        );
        assert_eq!(
            WorkMode::parse("yolo").unwrap().policy_mode(),
            ToolPolicyMode::Audit
        );
        assert_eq!(
            WorkMode::parse("guarded").unwrap().policy_mode(),
            ToolPolicyMode::Guarded
        );
        assert!(WorkMode::parse("chaos").is_none());

        // plan 模式下写文件必须被拒绝，edit 模式下放行。
        let plan = ToolPolicy::new(WorkMode::Plan.policy_mode());
        assert_eq!(
            plan.evaluate("write_to_file", &json!({})).1,
            ToolPolicyDecision::Deny
        );
        let edit = ToolPolicy::new(WorkMode::Edit.policy_mode());
        assert_eq!(
            edit.evaluate("write_to_file", &json!({})).1,
            ToolPolicyDecision::Allow
        );
    }
}

pub fn is_read_only_command(arguments: &Value) -> bool {
    let cmd = arguments
        .get("command_line")
        .or_else(|| arguments.get("CommandLine"))
        .or_else(|| arguments.get("command"))
        .or_else(|| arguments.get("cmd"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let cmd_lower = cmd.trim().to_lowercase();
    if cmd_lower.is_empty() {
        return false;
    }
    // 纯状态查看与无副作用查询命令
    cmd_lower.starts_with("git status")
        || cmd_lower.starts_with("git diff")
        || cmd_lower.starts_with("git log")
        || cmd_lower.starts_with("git show")
        || cmd_lower.starts_with("git branch")
        || cmd_lower.starts_with("cargo check")
        || cmd_lower.starts_with("cargo clippy")
        || cmd_lower.starts_with("cargo metadata")
        || cmd_lower.starts_with("npm list")
        || cmd_lower.starts_with("pnpm list")
        || cmd_lower.starts_with("dir")
        || cmd_lower.starts_with("ls")
        || cmd_lower.starts_with("pwd")
        || cmd_lower.starts_with("echo ")
        || cmd_lower.starts_with("cat ")
        || cmd_lower.starts_with("type ")
        || cmd_lower.starts_with("where ")
        || cmd_lower.starts_with("which ")
        || cmd_lower.starts_with("get-childitem")
        || cmd_lower.starts_with("select-string")
}

pub fn is_safe_sandbox_command(arguments: &Value) -> bool {
    let cmd = arguments
        .get("command_line")
        .or_else(|| arguments.get("CommandLine"))
        .or_else(|| arguments.get("command"))
        .or_else(|| arguments.get("cmd"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let cmd_lower = cmd.trim().to_lowercase();
    if cmd_lower.is_empty() {
        return false;
    }

    // 拒绝破坏性系统级操作
    let destructive = [
        "rm -rf /", "rmdir /s /q c:", "del /f /s /q c:", "format ",
        "drop database", "mkfs", "dd if="
    ];
    if destructive.iter().any(|t| cmd_lower.contains(t)) {
        return false;
    }

    // 常用安全构建、验证、测试与工程脚本
    cmd_lower.starts_with("cargo test")
        || cmd_lower.starts_with("cargo build")
        || cmd_lower.starts_with("cargo run")
        || cmd_lower.starts_with("cargo fmt")
        || cmd_lower.starts_with("npm test")
        || cmd_lower.starts_with("npm run")
        || cmd_lower.starts_with("pnpm test")
        || cmd_lower.starts_with("pnpm run")
        || cmd_lower.starts_with("yarn test")
        || cmd_lower.starts_with("yarn run")
        || cmd_lower.starts_with("pytest")
        || cmd_lower.starts_with("python -m")
        || cmd_lower.starts_with("python test")
        || cmd_lower.starts_with("node ")
        || cmd_lower.starts_with("vitest")
        || cmd_lower.starts_with("jest")
        || cmd_lower.starts_with("tsc")
        || cmd_lower.starts_with("git ")
        || cmd_lower.contains("validate-rule")
        || cmd_lower.contains("test-rule")
        || cmd_lower.contains("export-command")
        || cmd_lower.contains("quick-scaffold")
        || cmd_lower.contains("patch-rule")
}
