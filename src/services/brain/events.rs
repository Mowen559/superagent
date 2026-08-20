use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 智能体状态机事件：所有大模型的输入输出、用户的交互全部溯源为强类型事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum SessionEvent {
    UserInput(String),
    ModelThought(String),
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        id: String,
        result: Value,
    },
    SystemWarning(String),
    SupervisorIntervention(String),
}

impl SessionEvent {
    /// 投影 (Project)：将强类型领域事件转换为 OpenAI 兼容的 Message 格式
    pub fn to_llm_message(&self) -> Value {
        match self {
            Self::UserInput(text) => serde_json::json!({ "role": "user", "content": text }),
            Self::ModelThought(text) => serde_json::json!({ "role": "assistant", "content": text }),
            Self::SystemWarning(text) => serde_json::json!({ "role": "user", "content": format!("[System Guard] {}", text) }),
            Self::SupervisorIntervention(text) => serde_json::json!({ "role": "user", "content": format!("[SUPERVISOR HIGH-PRIORITY INTERVENTION]: {}", text) }),
            Self::ToolCall { id, name, arguments } => {
                serde_json::json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments.to_string()
                        }
                    }]
                })
            },
            Self::ToolResult { id, result } => {
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": result.to_string()
                })
            }
        }
    }
}
