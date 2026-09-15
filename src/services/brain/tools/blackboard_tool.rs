use std::sync::Arc;
use serde_json::{json, Value};

use crate::services::brain::agent_blackboard::AgentBlackboard;
use crate::services::brain::executor::ToolExecutor;

pub struct BlackboardTool {
    blackboard: Arc<AgentBlackboard>,
    agent_identity: String,
}

impl BlackboardTool {
    pub fn new(blackboard: Arc<AgentBlackboard>, agent_identity: impl Into<String>) -> Self {
        Self {
            blackboard,
            agent_identity: agent_identity.into(),
        }
    }
}

impl ToolExecutor for BlackboardTool {
    fn name(&self) -> &'static str {
        "blackboard_tool"
    }

    fn description(&self) -> &'static str {
        "Shared multi-agent blackboard protocol for cross-agent collaboration, state synchronization, and collective intelligence. Use action='post' to broadcast facts, hypotheses, blockers, or handoffs; action='get' to read a specific entry; action='query_tag' to retrieve entries by tag; and action='list' to inspect all active blackboard entries."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform on the blackboard: 'post' (publish entry), 'get' (retrieve entry by key), 'query_tag' (filter by tag), or 'list' (list all entries).",
                    "enum": ["post", "get", "query_tag", "list"]
                },
                "key": {
                    "type": "string",
                    "description": "The unique key for the entry (e.g. 'auth:token_format', 'hypothesis:db_timeout', 'worker:file_locks'). Required for 'post' and 'get'."
                },
                "value": {
                    "type": "string",
                    "description": "The payload or discovery statement to share with other agents. Required for 'post'."
                },
                "tag": {
                    "type": "string",
                    "description": "Semantic category: 'fact' (confirmed workspace reality), 'hypothesis' (unverified theory), 'blocker' (active impediment), 'handoff' (subagent deliverables). Required for 'post' and 'query_tag'."
                }
            },
            "required": ["action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action_opt = args.get("action").and_then(|v| v.as_str());
        let action = action_opt.unwrap_or("list");

        match action {
            "post" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "key is required for action 'post'".to_string())?;
                let value = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "value is required for action 'post'".to_string())?;
                let tag = args
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("general");

                self.blackboard.post(key, value, &self.agent_identity, tag);

                Ok(json!({
                    "status": "posted",
                    "key": key,
                    "publisher": self.agent_identity,
                    "tag": tag,
                    "message": "Entry published to shared blackboard. All cooperating agents can now query this state."
                }))
            }
            "get" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "key is required for action 'get'".to_string())?;

                if let Some(entry) = self.blackboard.get(key) {
                    Ok(json!({
                        "found": true,
                        "key": entry.key,
                        "value": entry.value,
                        "publisher": entry.publisher,
                        "tag": entry.tag,
                        "timestamp": entry.timestamp
                    }))
                } else {
                    Ok(json!({
                        "found": false,
                        "key": key,
                        "message": "Key not found on blackboard."
                    }))
                }
            }
            "query_tag" => {
                let tag = args
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "tag is required for action 'query_tag'".to_string())?;

                let entries = self.blackboard.query_by_tag(tag);
                Ok(json!({
                    "tag": tag,
                    "count": entries.len(),
                    "entries": entries
                }))
            }
            "list" => {
                let history = self.blackboard.get_all_history();
                Ok(json!({
                    "total_entries": history.len(),
                    "entries": history
                }))
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blackboard_tool_full_flow() {
        let board = Arc::new(AgentBlackboard::new());
        let tool_planner = BlackboardTool::new(board.clone(), "PlannerAgent");
        let tool_worker = BlackboardTool::new(board.clone(), "WorkerAgent");

        // 1. Post fact from Planner
        let post_res = tool_planner.execute(json!({
            "action": "post",
            "key": "arch:db_type",
            "value": "MySQL 8.0 with MyBatis-Plus (JPA strictly prohibited)",
            "tag": "fact"
        })).unwrap();
        assert_eq!(post_res["status"], "posted");

        // 2. Post blocker from Worker
        let post_blocker = tool_worker.execute(json!({
            "action": "post",
            "key": "blocker:missing_dep",
            "value": "Redisson dependency missing in pom.xml",
            "tag": "blocker"
        })).unwrap();
        assert_eq!(post_blocker["status"], "posted");

        // 3. Get specific key
        let get_res = tool_worker.execute(json!({
            "action": "get",
            "key": "arch:db_type"
        })).unwrap();
        assert_eq!(get_res["found"], true);
        assert_eq!(get_res["publisher"], "PlannerAgent");

        // 4. Query by tag
        let query_res = tool_planner.execute(json!({
            "action": "query_tag",
            "tag": "blocker"
        })).unwrap();
        assert_eq!(query_res["count"], 1);

        // 5. List all
        let list_res = tool_planner.execute(json!({ "action": "list" })).unwrap();
        assert_eq!(list_res["total_entries"], 2);
    }
}
