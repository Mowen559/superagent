use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    pub max_tool_output_chars: usize,
    pub preserve_recent_messages: usize,
    pub max_total_tokens_estimate: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            max_tool_output_chars: 1200,
            preserve_recent_messages: 10,
            max_total_tokens_estimate: 8000,
        }
    }
}

pub struct SemanticCompactor {
    config: CompactionConfig,
}

impl Default for SemanticCompactor {
    fn default() -> Self {
        Self::new(CompactionConfig::default())
    }
}

impl SemanticCompactor {
    pub fn new(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// KV Trimming: 针对单个 Tool Output 文本进行智能截断与解构
    pub fn trim_tool_output(&self, text: &str) -> String {
        if text.len() <= self.config.max_tool_output_chars {
            return text.to_string();
        }

        let head_size = self.config.max_tool_output_chars / 2 - 50;
        let tail_size = self.config.max_tool_output_chars / 2 - 50;
        let truncated_bytes = text.len() - (head_size + tail_size);

        format!(
            "{}\n\n... [Semantic Compactor: Truncated {} bytes of verbose tool output] ...\n\n{}",
            &text[..head_size],
            truncated_bytes,
            &text[text.len() - tail_size..]
        )
    }

    /// 对消息队列进行上下文语义压缩（保留 system prompt、最新的 N 条消息，对其余历史 tool result 执行截断）
    pub fn compact_messages(&self, messages: Vec<Value>) -> Vec<Value> {
        let total = messages.len();
        if total <= self.config.preserve_recent_messages + 1 {
            return messages;
        }

        let preserve_start_idx = total.saturating_sub(self.config.preserve_recent_messages);

        messages
            .into_iter()
            .enumerate()
            .map(|(idx, mut msg)| {
                // 不保留区域的历史 Tool Result 消息进行语义裁剪
                if idx > 0 && idx < preserve_start_idx {
                    if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                        if role == "tool" || role == "user" {
                            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                                let trimmed = self.trim_tool_output(content);
                                msg["content"] = Value::String(trimmed);
                            }
                        }
                    }
                }
                msg
            })
            .collect()
    }

    /// 将 Working Memory 关键信息转换为固化的 Episodic Memory 记录项
    pub fn extract_episodic_snapshot(&self, recent_actions: &[String]) -> String {
        format!(
            "[Episodic Memory Snapshot]\n- Action Count: {}\n- Highlights:\n{}",
            recent_actions.len(),
            recent_actions
                .iter()
                .map(|a| format!("  * {}", a))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_tool_output() {
        let compactor = SemanticCompactor::default();
        let verbose_text = "A".repeat(5000);
        let trimmed = compactor.trim_tool_output(&verbose_text);
        assert!(trimmed.contains("Truncated"));
        assert!(trimmed.len() < 5000);
    }

    #[test]
    fn test_compact_messages() {
        let compactor = SemanticCompactor::new(CompactionConfig {
            max_tool_output_chars: 100,
            preserve_recent_messages: 2,
            max_total_tokens_estimate: 1000,
        });

        let messages = vec![
            serde_json::json!({"role": "system", "content": "System"}),
            serde_json::json!({"role": "tool", "content": "B".repeat(500)}),
            serde_json::json!({"role": "user", "content": "Recent 1"}),
            serde_json::json!({"role": "assistant", "content": "Recent 2"}),
        ];

        let compacted = compactor.compact_messages(messages);
        assert_eq!(compacted.len(), 4);
        let hist_tool = compacted[1]["content"].as_str().unwrap();
        assert!(hist_tool.contains("Truncated"));
    }
}
