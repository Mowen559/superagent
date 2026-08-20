//! 上下文感知切块 (Contextual Chunking)
//! 参考 ai-agent-book 和 Anthropic 的最新设计，长文本切块时不应丢失全局上下文。
//! 
//! 这个模块提供一种方法：将长文本切块后，强行将该文件的全局语义 (Context)
//! 附加上去，使得在 RAG 检索时，哪怕切块再小，也不会丢失来源信息。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextualChunk {
    pub chunk_index: usize,
    pub global_context: String,
    pub content: String,
}

impl ContextualChunk {
    /// 组合成供大模型阅读的最终字符串
    pub fn to_llm_string(&self) -> String {
        format!("(Context: {})\n{}", self.global_context, self.content)
    }
}

pub struct ContextualChunker {
    pub max_chunk_size: usize,
}

impl ContextualChunker {
    pub fn new(max_chunk_size: usize) -> Self {
        Self { max_chunk_size }
    }

    /// 执行上下文感知切块
    /// `global_context` 可以是文件路径、函数的摘要或是由 LLM 提前总结好的全局描述
    pub fn chunk_text(&self, text: &str, global_context: &str) -> Vec<ContextualChunk> {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut index = 0;

        // 简单按段落或行进行拆分，防止切断句子
        for line in text.lines() {
            if current_chunk.len() + line.len() > self.max_chunk_size && !current_chunk.is_empty() {
                chunks.push(ContextualChunk {
                    chunk_index: index,
                    global_context: global_context.to_string(),
                    content: current_chunk.clone(),
                });
                current_chunk.clear();
                index += 1;
            }
            current_chunk.push_str(line);
            current_chunk.push('\n');
        }

        if !current_chunk.trim().is_empty() {
            chunks.push(ContextualChunk {
                chunk_index: index,
                global_context: global_context.to_string(),
                content: current_chunk,
            });
        }

        chunks
    }
}
