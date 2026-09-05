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
    Audit,
    ReadOnly,
    Approval,
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
            "read-only" | "readonly" | "deny-mutations" => ToolPolicyMode::ReadOnly,
            "approval" | "require-approval" => ToolPolicyMode::Approval,
            _ => ToolPolicyMode::Audit,
        };
        Self { mode }
    }

    pub fn new(mode: ToolPolicyMode) -> Self {
        Self { mode }
    }

    pub fn evaluate(&self, name: &str, arguments: &Value) -> (ToolRisk, ToolPolicyDecision) {
        let risk = classify_tool_risk(name, arguments);
        let decision = match self.mode {
            ToolPolicyMode::Audit => ToolPolicyDecision::Allow,
            ToolPolicyMode::ReadOnly if risk == ToolRisk::ReadOnly => ToolPolicyDecision::Allow,
            ToolPolicyMode::ReadOnly => ToolPolicyDecision::Deny,
            ToolPolicyMode::Approval if risk == ToolRisk::ReadOnly => ToolPolicyDecision::Allow,
            ToolPolicyMode::Approval => ToolPolicyDecision::RequireApproval,
        };
        (risk, decision)
    }
}

pub fn classify_tool_risk(name: &str, arguments: &Value) -> ToolRisk {
    match name {
        "view_file"
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
        "run_command" | "manage_process" | "invoke_subagent" | "intervene_worker"
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
}
