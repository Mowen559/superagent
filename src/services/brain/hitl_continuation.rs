use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApprovalRequest {
    pub token: String,
    pub tool_name: String,
    pub tool_args: String,
    pub risk_reason: String,
    pub created_at: i64,
    pub status: ApprovalStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuationOutcome {
    ResumeExecution { tool_name: String, tool_args: String },
    AbortWithError(String),
    StillPending,
    TokenNotFound,
}

#[derive(Clone)]
pub struct HitlContinuationEngine {
    pending_approvals: Arc<RwLock<HashMap<String, PendingApprovalRequest>>>,
}

impl Default for HitlContinuationEngine {
    fn default() -> Self {
        Self {
            pending_approvals: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl HitlContinuationEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 评估工具调用是否需要触发 HITL 敏感断点阻断
    pub fn is_destructive_op(&self, tool_name: &str, args_json: &str) -> Option<String> {
        let name_lower = tool_name.to_lowercase();
        let args_lower = args_json.to_lowercase();

        if name_lower == "run_command" || name_lower == "execute_command" || name_lower == "shell" {
            if args_lower.contains("rm ") || args_lower.contains("drop ") || args_lower.contains("format") || args_lower.contains("git push --force") {
                return Some(format!("Destructive shell command detected in tool '{}'", tool_name));
            }
        }

        if name_lower == "delete_file" || name_lower == "remove_dir" {
            return Some(format!("File removal tool '{}' invoked", tool_name));
        }

        None
    }

    /// 挂起 Tool 执行并创建 Pause and Wait 状态与 Approval Token
    pub fn pause_for_approval(&self, tool_name: &str, tool_args: &str, risk_reason: &str) -> PendingApprovalRequest {
        let token = format!("approval-{}", Uuid::new_v4());
        let request = PendingApprovalRequest {
            token: token.clone(),
            tool_name: tool_name.to_string(),
            tool_args: tool_args.to_string(),
            risk_reason: risk_reason.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            status: ApprovalStatus::Pending,
        };

        let mut lock = self.pending_approvals.write().unwrap();
        lock.insert(token, request.clone());
        request
    }

    /// 用户人类提交决策以恢复 Continuation 流程
    pub fn submit_decision(&self, token: &str, approved: bool, feedback: Option<&str>) -> ContinuationOutcome {
        let mut lock = self.pending_approvals.write().unwrap();
        if let Some(req) = lock.get_mut(token) {
            if approved {
                req.status = ApprovalStatus::Approved;
                ContinuationOutcome::ResumeExecution {
                    tool_name: req.tool_name.clone(),
                    tool_args: req.tool_args.clone(),
                }
            } else {
                let reason = feedback.unwrap_or("User rejected destructive action").to_string();
                req.status = ApprovalStatus::Rejected(reason.clone());
                ContinuationOutcome::AbortWithError(reason)
            }
        } else {
            ContinuationOutcome::TokenNotFound
        }
    }

    pub fn get_approval_status(&self, token: &str) -> ContinuationOutcome {
        let lock = self.pending_approvals.read().unwrap();
        if let Some(req) = lock.get(token) {
            match &req.status {
                ApprovalStatus::Pending => ContinuationOutcome::StillPending,
                ApprovalStatus::Approved => ContinuationOutcome::ResumeExecution {
                    tool_name: req.tool_name.clone(),
                    tool_args: req.tool_args.clone(),
                },
                ApprovalStatus::Rejected(reason) => ContinuationOutcome::AbortWithError(reason.clone()),
            }
        } else {
            ContinuationOutcome::TokenNotFound
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_destructive_op_detection() {
        let engine = HitlContinuationEngine::new();
        assert!(engine.is_destructive_op("run_command", "{\"cmd\": \"rm -rf /\"}").is_some());
        assert!(engine.is_destructive_op("run_command", "{\"cmd\": \"cargo check\"}").is_none());
    }

    #[test]
    fn test_pause_and_continuation_flow() {
        let engine = HitlContinuationEngine::new();
        let req = engine.pause_for_approval("run_command", "{\"cmd\": \"rm -rf /\"}", "Destructive");
        assert!(req.token.starts_with("approval-"));

        assert_eq!(engine.get_approval_status(&req.token), ContinuationOutcome::StillPending);

        let outcome = engine.submit_decision(&req.token, true, None);
        match outcome {
            ContinuationOutcome::ResumeExecution { tool_name, .. } => {
                assert_eq!(tool_name, "run_command");
            }
            _ => panic!("Expected ResumeExecution"),
        }
    }
}
