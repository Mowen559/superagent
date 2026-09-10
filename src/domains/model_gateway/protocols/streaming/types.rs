//! 娴佸紡鍝嶅簲杞鎹㈡ā鍧

//!

//! 瀹炵幇 OpenAI SSE -> Anthropic SSE 鏍煎紡杞鎹

use crate::domains::model_gateway::protocols::sse::{strip_sse_field, take_sse_block};

use bytes::Bytes;

use futures::stream::{Stream, StreamExt};

use serde::{Deserialize, Serialize};

use serde_json::{json, Value};

use std::collections::{HashMap, HashSet};


use super::*;

/// OpenAI 娴佸紡鍝嶅簲鏁版嵁缁撴瀯

#[derive(Debug, Deserialize)]

pub(crate) struct OpenAIStreamChunk {
    #[serde(default)]
    pub(crate) id: String,

    #[serde(default)]
    pub(crate) model: String,

    #[serde(default)]
    pub(crate) choices: Vec<StreamChoice>,

    #[serde(default)]
    pub(crate) usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]

pub(crate) struct StreamChoice {
    pub(crate) delta: Delta,

    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]

pub(crate) struct Delta {
    #[serde(default)]
    pub(crate) content: Option<String>,

    // OpenRouter/Kimi/鍏跺畠 浣跨敤 reasoning锛孌eepSeek 浣跨敤 reasoning_content
    #[serde(default, alias = "reasoning_content")]
    pub(crate) reasoning: Option<String>,

    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize, Serialize)]

pub(crate) struct DeltaToolCall {
    pub(crate) index: usize,

    #[serde(default)]
    pub(crate) id: Option<String>,

    #[serde(rename = "type", default)]
    pub(crate) call_type: Option<String>,

    #[serde(default)]
    pub(crate) function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize, Serialize)]

pub(crate) struct DeltaFunction {
    #[serde(default)]
    pub(crate) name: Option<String>,

    #[serde(default)]
    pub(crate) arguments: Option<String>,
}

/// OpenAI 娴佸紡鍝嶅簲?usage 淇℃伅锛堝畬鏁寸増?

#[derive(Debug, Deserialize)]

pub(crate) struct Usage {
    #[serde(default)]
    pub(crate) prompt_tokens: u32,

    #[serde(default)]
    pub(crate) completion_tokens: u32,

    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<PromptTokensDetails>,

    /// Some compatible servers return Anthropic-style cache fields directly

    #[serde(default)]
    pub(crate) cache_read_input_tokens: Option<u32>,

    #[serde(default)]
    pub(crate) cache_creation_input_tokens: Option<u32>,
}

/// Nested token details from OpenAI format

#[derive(Debug, Deserialize)]

pub(crate) struct PromptTokensDetails {
    #[serde(default)]
    pub(crate) cached_tokens: u32,

    #[serde(default)]
    pub(crate) cache_write_tokens: u32,
}

#[derive(Debug, Clone)]

pub(crate) struct ToolBlockState {
    pub(crate) anthropic_index: u32,

    pub(crate) id: String,

    pub(crate) name: String,

    pub(crate) started: bool,

    pub(crate) pending_args: String,

    /// 杩炵画绌虹櫧瀛楃﹁℃暟 ?鐢ㄤ簬妫?Copilot 鏃犻檺鎹㈣ bug

    /// ?function call 鍙傛暟涓鐨勮繛缁绌虹櫧瀛楃﹁揪鍒伴槇鍊兼椂锛屽己鍒剁粓姝㈡祦
    pub(crate) consecutive_whitespace: usize,

    /// 鏄鍚﹀凡鍥犳棤闄愮┖鐧 bug 琚涓?
    pub(crate) aborted: bool,
}

/// 鏃犻檺绌虹櫧 bug 鐨勮繛缁绌虹櫧瀛楃﹂槇?

pub(crate) const INFINITE_WHITESPACE_THRESHOLD: usize = 500;

