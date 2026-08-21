use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 智能体状态机事件：所有大模型的输入输出、用户的交互全部溯源为强类型事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum SessionEvent {
    UserInput(String),
    ModelThought(String),
    ContextCompacted {
        summary: String,
        original_event_count: usize,
    },
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
            Self::ContextCompacted { summary, original_event_count } => {
                serde_json::json!({
                    "role": "user",
                    "content": format!("[SYSTEM: {} older events were compacted into this summary]: {}", original_event_count, summary)
                })
            },
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

use std::pin::Pin;
use std::future::Future;
use crate::services::llm_provider::LlmProvider;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Async hook for before_prompt
pub trait BeforePromptHook: Send + Sync {
    fn on_before_prompt<'a>(&'a self, events: &'a mut Vec<SessionEvent>, llm: &'a LlmProvider) -> BoxFuture<'a, ()>;
}

pub struct EventBus {
    pub before_prompt_hooks: Vec<Box<dyn BeforePromptHook>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            before_prompt_hooks: Vec::new(),
        }
    }

    pub fn register_before_prompt(&mut self, hook: Box<dyn BeforePromptHook>) {
        self.before_prompt_hooks.push(hook);
    }

    pub async fn emit_before_prompt(&self, events: &mut Vec<SessionEvent>, llm: &LlmProvider) {
        for hook in &self.before_prompt_hooks {
            hook.on_before_prompt(events, llm).await;
        }
    }
}
