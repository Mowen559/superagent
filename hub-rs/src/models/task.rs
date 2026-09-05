use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    #[serde(rename = "timeout")]
    TimedOut,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timeout",
        }
    }

    pub fn from_agent_status(status: &str) -> Self {
        match status {
            "completed" => Self::Completed,
            "cancelled" | "canceled" | "aborted" | "interrupted" => Self::Cancelled,
            "timeout" | "timedout" | "timed_out" => Self::TimedOut,
            _ => Self::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_terminal_statuses_do_not_collapse_timeout_into_failure() {
        assert_eq!(
            TaskStatus::from_agent_status("completed"),
            TaskStatus::Completed
        );
        assert_eq!(
            TaskStatus::from_agent_status("cancelled"),
            TaskStatus::Cancelled
        );
        assert_eq!(
            TaskStatus::from_agent_status("timeout"),
            TaskStatus::TimedOut
        );
        assert_eq!(TaskStatus::from_agent_status("unknown"), TaskStatus::Failed);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(rename = "type")]
    pub task_type: String,
    pub project_path: String,
    pub status: TaskStatus,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,

    // -- Development Run Extensions --
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_baseline: Option<String>, // e.g., git commit hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_set: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<serde_json::Value>, // lint/test results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<String>, // pending, approved, rejected
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<serde_json::Value>,
}
