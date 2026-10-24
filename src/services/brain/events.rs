use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Created,
    Running,
    NeedsInput,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

/// 智能体状态机事件：所有大模型的输入输出、用户的交互全部溯源为强类型事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum SessionEvent {
    RunStarted {
        run_id: String,
        at: String,
    },
    RunFinished {
        run_id: String,
        outcome: RunOutcome,
        detail: Option<String>,
        at: String,
    },
    LifecycleChanged {
        from: SessionStatus,
        to: SessionStatus,
        reason: Option<String>,
        at: String,
    },
    UserInput(String),
    /// 用户输入附带图片（多模态）：`images` 为本地可读路径或 data URL。
    UserInputWithImages { text: String, images: Vec<String> },
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
    SteeringInjected {
        lease_id: String,
        item_ids: Vec<String>,
        content: String,
    },
}

impl SessionEvent {
    /// 投影 (Project)：将强类型领域事件转换为 OpenAI 兼容的 Message 格式
    pub fn to_llm_message(&self) -> Value {
        match self {
            Self::RunStarted { run_id, .. } => serde_json::json!({
                "role": "system",
                "content": format!("[Run started: {run_id}]")
            }),
            Self::RunFinished {
                run_id, outcome, ..
            } => serde_json::json!({
                "role": "system",
                "content": format!("[Run finished: {run_id} - {:?}]", outcome)
            }),
            Self::LifecycleChanged { to, reason, .. } => serde_json::json!({
                "role": "system",
                "content": format!("[Session lifecycle: {:?}{}]", to, reason.as_ref().map(|value| format!(" - {value}")).unwrap_or_default())
            }),
            Self::UserInput(text) => serde_json::json!({ "role": "user", "content": text }),
            // 多模态投影：OpenAI 兼容 content 数组（text + image_url data URL）。
            // 图片在入账时已由 worker 统一转为 data URL，投影阶段零 IO。
            Self::UserInputWithImages { text, images } => {
                let mut parts = vec![serde_json::json!({ "type": "text", "text": text })];
                for image in images {
                    parts.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": image }
                    }));
                }
                serde_json::json!({ "role": "user", "content": parts })
            }
            Self::ModelThought(text) => serde_json::json!({ "role": "assistant", "content": text }),
            Self::ContextCompacted {
                summary,
                original_event_count,
            } => {
                serde_json::json!({
                    "role": "user",
                    "content": format!("[SYSTEM: {} older events were compacted into this summary]: {}", original_event_count, summary)
                })
            }
            Self::SystemWarning(text) => {
                serde_json::json!({ "role": "user", "content": format!("[System Guard] {}", text) })
            }
            Self::SupervisorIntervention(text) => {
                serde_json::json!({ "role": "user", "content": format!("[SUPERVISOR HIGH-PRIORITY INTERVENTION]: {}", text) })
            }
            Self::SteeringInjected { content, .. } => {
                serde_json::json!({ "role": "user", "content": content })
            }
            Self::ToolCall {
                id,
                name,
                arguments,
            } => {
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
            }
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

use crate::domains::model_gateway::providers::LlmProvider;
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Async hook for before_prompt
pub trait BeforePromptHook: Send + Sync {
    fn on_before_prompt<'a>(
        &'a self,
        events: &'a mut Vec<SessionEvent>,
        llm: &'a LlmProvider,
    ) -> BoxFuture<'a, ()>;
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
