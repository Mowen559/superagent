//! 娴佸紡鍝嶅簲杞鎹㈡ā鍧

//!

//! 瀹炵幇 OpenAI SSE -> Anthropic SSE 鏍煎紡杞鎹

use crate::services::protocol_converter::sse::{strip_sse_field, take_sse_block};

use bytes::Bytes;

use futures::stream::{Stream, StreamExt};

use serde::{Deserialize, Serialize};

use serde_json::{json, Value};

use std::collections::{HashMap, HashSet};

/// OpenAI 娴佸紡鍝嶅簲鏁版嵁缁撴瀯

#[derive(Debug, Deserialize)]

struct OpenAIStreamChunk {
    #[serde(default)]
    id: String,

    #[serde(default)]
    model: String,

    #[serde(default)]
    choices: Vec<StreamChoice>,

    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]

struct StreamChoice {
    delta: Delta,

    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]

struct Delta {
    #[serde(default)]
    content: Option<String>,

    // OpenRouter/Kimi/鍏跺畠 浣跨敤 reasoning锛孌eepSeek 浣跨敤 reasoning_content
    #[serde(default, alias = "reasoning_content")]
    reasoning: Option<String>,

    #[serde(default)]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize, Serialize)]

struct DeltaToolCall {
    index: usize,

    #[serde(default)]
    id: Option<String>,

    #[serde(rename = "type", default)]
    call_type: Option<String>,

    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize, Serialize)]

struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    arguments: Option<String>,
}

/// OpenAI 娴佸紡鍝嶅簲?usage 淇℃伅锛堝畬鏁寸増?

#[derive(Debug, Deserialize)]

struct Usage {
    #[serde(default)]
    prompt_tokens: u32,

    #[serde(default)]
    completion_tokens: u32,

    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,

    /// Some compatible servers return Anthropic-style cache fields directly

    #[serde(default)]
    cache_read_input_tokens: Option<u32>,

    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
}

/// Nested token details from OpenAI format

#[derive(Debug, Deserialize)]

struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u32,

    #[serde(default)]
    cache_write_tokens: u32,
}

#[derive(Debug, Clone)]

struct ToolBlockState {
    anthropic_index: u32,

    id: String,

    name: String,

    started: bool,

    pending_args: String,

    /// 杩炵画绌虹櫧瀛楃﹁℃暟 ?鐢ㄤ簬妫?Copilot 鏃犻檺鎹㈣ bug

    /// ?function call 鍙傛暟涓鐨勮繛缁绌虹櫧瀛楃﹁揪鍒伴槇鍊兼椂锛屽己鍒剁粓姝㈡祦
    consecutive_whitespace: usize,

    /// 鏄鍚﹀凡鍥犳棤闄愮┖鐧 bug 琚涓?
    aborted: bool,
}

/// 鏃犻檺绌虹櫧 bug 鐨勮繛缁绌虹櫧瀛楃﹂槇?

const INFINITE_WHITESPACE_THRESHOLD: usize = 500;

fn build_anthropic_usage_json(usage: &Usage) -> Value {
    // OpenAI prompt_tokens 鍚缂撳瓨锛孉nthropic input_tokens 涓嶅惈锛岄渶鍑忓幓 cache_read ?cache_creation

    // 锛堜笁妗朵簰鏂ワ紝鎭掔瓑 input + cache_read + cache_creation == prompt_tokens锛?

    let cached = extract_cache_read_tokens(usage).unwrap_or(0);

    let cache_creation = extract_cache_write_tokens(usage).unwrap_or(0);

    let input_tokens = usage
        .prompt_tokens
        .saturating_sub(cached)
        .saturating_sub(cache_creation);

    let mut usage_json = json!({

        "input_tokens": input_tokens,

        "output_tokens": usage.completion_tokens

    });

    if cached > 0 {
        usage_json["cache_read_input_tokens"] = json!(cached);
    }

    if cache_creation > 0 {
        usage_json["cache_creation_input_tokens"] = json!(cache_creation);
    }

    usage_json
}

fn default_anthropic_usage_json() -> Value {
    json!({

        "input_tokens": 0,

        "output_tokens": 0

    })
}

fn build_message_delta_event(stop_reason: Option<String>, usage_json: Option<Value>) -> Value {
    let usage = usage_json
        .filter(|usage| usage.is_object())
        .unwrap_or_else(default_anthropic_usage_json);

    json!({

        "type": "message_delta",

        "delta": {

            "stop_reason": stop_reason,

            "stop_sequence": null

        },

        "usage": usage

    })
}

/// 鍒涘缓 Anthropic SSE ?

pub fn create_anthropic_sse_stream<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {

        let mut buffer = String::new();

        let mut utf8_remainder: Vec<u8> = Vec::new();

        let mut message_id = None;

        let mut current_model = None;

        let mut next_content_index: u32 = 0;

        let mut has_sent_message_start = false;

        // 鏌愪簺涓婃父 provider锛堝 OpenRouter ?kimi-k2.6锛変細?tool_use 鍚庡彂閫佸?

        // ?finish_reason ?SSE chunk銆侫nthropic 鍗忚瑕佹眰姣忎釜娑堟伅娴佸彧鑳芥湁涓?

        // message_delta锛岄噸澶嶄細瀵艰嚧 Claude Code abort 杩炴帴銆傚洜姝ら渶瑕侊細

        // 1) has_emitted_message_delta: 鍘婚噸锛屽彧澶勭悊绗涓?finish_reason

        // 2) pending_message_delta: 缂撳瓨寤惰繜?[DONE] 鍙戦侊紝纭淇 usage 瀹屾暣

        let mut has_emitted_message_delta = false;

        let mut pending_message_delta: Option<(Option<String>, Option<Value>)> = None;

        let mut has_sent_message_stop = false;

        let mut stream_ended_with_error = false;

        let mut latest_usage: Option<Value> = None;

        let mut current_non_tool_block_type: Option<&'static str> = None;

        let mut current_non_tool_block_index: Option<u32> = None;

        let mut tool_blocks_by_index: HashMap<usize, ToolBlockState> = HashMap::new();

        let mut open_tool_block_indices: HashSet<u32> = HashSet::new();



        tokio::pin!(stream);



        while let Some(chunk) = stream.next().await {

            match chunk {

                Ok(bytes) => {

                    crate::services::protocol_converter::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);



                    while let Some(line) = take_sse_block(&mut buffer) {

                        if line.trim().is_empty() {

                            continue;

                        }



                        for l in line.lines() {

                            if let Some(data) = strip_sse_field(l, "data") {

                                if data.trim() == "[DONE]" {

                                    tracing::debug!("[Claude/OpenRouter] <<< OpenAI SSE: [DONE]");



                                    // 娴佹ｅ父缁撴潫锛屽彂鍑虹紦瀛?message_delta锛堝惈瀹屾暣 usage锛?

                                    if let Some((stop_reason, usage_json)) = pending_message_delta.take() {

                                        let event = build_message_delta_event(stop_reason, usage_json);

                                        let sse_data = format!("event: message_delta\ndata: {}\n\n",

                                            serde_json::to_string(&event).unwrap_or_default());

                                        tracing::debug!("[Claude/OpenRouter] >>> Anthropic SSE: message_delta (from pending)");

                                        yield Ok(Bytes::from(sse_data));

                                    }



                                    let event = json!({"type": "message_stop"});

                                    let sse_data = format!("event: message_stop\ndata: {}\n\n",

                                        serde_json::to_string(&event).unwrap_or_default());

                                    tracing::debug!("[Claude/OpenRouter] >>> Anthropic SSE: message_stop");

                                    yield Ok(Bytes::from(sse_data));

                                    has_sent_message_stop = true;

                                    continue;

                                }



                                if let Ok(chunk) = serde_json::from_str::<OpenAIStreamChunk>(data) {

                                    tracing::debug!("[Claude/OpenRouter] <<< SSE chunk received");



                                    if message_id.is_none() && !chunk.id.is_empty() {

                                        message_id = Some(chunk.id.clone());

                                    }

                                    if current_model.is_none() && !chunk.model.is_empty() {

                                        current_model = Some(chunk.model.clone());

                                    }



                                    let chunk_usage_json =

                                        chunk.usage.as_ref().map(build_anthropic_usage_json);

                                    if let Some(usage_json) = &chunk_usage_json {

                                        latest_usage = Some(usage_json.clone());

                                        if let Some((_, pending_usage)) = pending_message_delta.as_mut() {

                                            *pending_usage = Some(usage_json.clone());

                                        }

                                    }



                                    if let Some(choice) = chunk.choices.first() {

                                        if !has_sent_message_start {

                                            // Build usage with cache tokens if available from first chunk

                                            let mut start_usage = json!({

                                                "input_tokens": 0,

                                                "output_tokens": 0

                                            });

                                            if let Some(u) = &chunk.usage {

                                                let cached = extract_cache_read_tokens(u).unwrap_or(0);

                                                let cache_creation =

                                                    extract_cache_write_tokens(u).unwrap_or(0);

                                                let input = u

                                                    .prompt_tokens

                                                    .saturating_sub(cached)

                                                    .saturating_sub(cache_creation);

                                                start_usage["input_tokens"] = json!(input);

                                                if cached > 0 {

                                                    start_usage["cache_read_input_tokens"] = json!(cached);

                                                }

                                                if cache_creation > 0 {

                                                    start_usage["cache_creation_input_tokens"] =

                                                        json!(cache_creation);

                                                }

                                            }



                                            let event = json!({

                                                "type": "message_start",

                                                "message": {

                                                    "id": message_id.clone().unwrap_or_default(),

                                                    "type": "message",

                                                    "role": "assistant",

                                                    "model": current_model.clone().unwrap_or_default(),

                                                    "usage": start_usage

                                                }

                                            });

                                            let sse_data = format!("event: message_start\ndata: {}\n\n",

                                                serde_json::to_string(&event).unwrap_or_default());

                                            yield Ok(Bytes::from(sse_data));

                                            has_sent_message_start = true;

                                        }



                                        // 澶勭悊 reasoning锛坱hinking?

                                        if let Some(reasoning) = &choice.delta.reasoning {

                                            if current_non_tool_block_type != Some("thinking") {

                                                if let Some(index) = current_non_tool_block_index.take() {

                                                    let event = json!({

                                                        "type": "content_block_stop",

                                                        "index": index

                                                    });

                                                    let sse_data = format!("event: content_block_stop\ndata: {}\n\n",

                                                        serde_json::to_string(&event).unwrap_or_default());

                                                    yield Ok(Bytes::from(sse_data));

                                                }

                                                let index = next_content_index;

                                                next_content_index += 1;

                                                let event = json!({

                                                    "type": "content_block_start",

                                                    "index": index,

                                                    "content_block": {

                                                        "type": "thinking",

                                                        "thinking": ""

                                                    }

                                                });

                                                let sse_data = format!("event: content_block_start\ndata: {}\n\n",

                                                    serde_json::to_string(&event).unwrap_or_default());

                                                yield Ok(Bytes::from(sse_data));

                                                current_non_tool_block_type = Some("thinking");

                                                current_non_tool_block_index = Some(index);

                                            }



                                            if let Some(index) = current_non_tool_block_index {

                                                let event = json!({

                                                    "type": "content_block_delta",

                                                    "index": index,

                                                    "delta": {

                                                        "type": "thinking_delta",

                                                        "thinking": reasoning

                                                    }

                                                });

                                                let sse_data = format!("event: content_block_delta\ndata: {}\n\n",

                                                    serde_json::to_string(&event).unwrap_or_default());

                                                yield Ok(Bytes::from(sse_data));

                                            }

                                        }



                                        // 澶勭悊鏂囨湰鍐呭

                                        if let Some(content) = &choice.delta.content {

                                            if !content.is_empty() {

                                                if current_non_tool_block_type != Some("text") {

                                                    if let Some(index) = current_non_tool_block_index.take() {

                                                        let event = json!({

                                                            "type": "content_block_stop",

                                                            "index": index

                                                        });

                                                        let sse_data = format!("event: content_block_stop\ndata: {}\n\n",

                                                            serde_json::to_string(&event).unwrap_or_default());

                                                        yield Ok(Bytes::from(sse_data));

                                                    }



                                                    let index = next_content_index;

                                                    next_content_index += 1;

                                                    let event = json!({

                                                        "type": "content_block_start",

                                                        "index": index,

                                                        "content_block": {

                                                            "type": "text",

                                                            "text": ""

                                                        }

                                                    });

                                                    let sse_data = format!("event: content_block_start\ndata: {}\n\n",

                                                        serde_json::to_string(&event).unwrap_or_default());

                                                    yield Ok(Bytes::from(sse_data));

                                                    current_non_tool_block_type = Some("text");

                                                    current_non_tool_block_index = Some(index);

                                                }



                                                if let Some(index) = current_non_tool_block_index {

                                                    let event = json!({

                                                        "type": "content_block_delta",

                                                        "index": index,

                                                        "delta": {

                                                            "type": "text_delta",

                                                            "text": content

                                                        }

                                                    });

                                                    let sse_data = format!("event: content_block_delta\ndata: {}\n\n",

                                                        serde_json::to_string(&event).unwrap_or_default());

                                                    yield Ok(Bytes::from(sse_data));

                                                }

                                            }

                                        }



                                        // 澶勭悊宸ュ叿璋冪敤

                                        if let Some(tool_calls) = &choice.delta.tool_calls {

                                            if !tool_calls.is_empty() {

                                                if let Some(index) = current_non_tool_block_index.take() {

                                                    let event = json!({

                                                        "type": "content_block_stop",

                                                        "index": index

                                                    });

                                                    let sse_data = format!("event: content_block_stop\ndata: {}\n\n",

                                                        serde_json::to_string(&event).unwrap_or_default());

                                                    yield Ok(Bytes::from(sse_data));

                                                }

                                                current_non_tool_block_type = None;



                                                for tool_call in tool_calls {

                                                    let (

                                                        anthropic_index,

                                                        id,

                                                        name,

                                                        should_start,

                                                        pending_after_start,

                                                        immediate_delta,

                                                    ) = {

                                                        let state = tool_blocks_by_index

                                                            .entry(tool_call.index)

                                                            .or_insert_with(|| {

                                                                let index = next_content_index;

                                                                next_content_index += 1;

                                                                ToolBlockState {

                                                                    anthropic_index: index,

                                                                    id: String::new(),

                                                                    name: String::new(),

                                                                    started: false,

                                                                    pending_args: String::new(),

                                                                    consecutive_whitespace: 0,

                                                                    aborted: false,

                                                                }

                                                            });



                                                        // 濡傛灉?tool call 宸茶涓姝锛堟棤闄愮┖?bug锛夛紝璺宠繃鍚庣画澶勭悊

                                                        if state.aborted {

                                                            continue;

                                                        }



                                                        if let Some(id) = &tool_call.id {

                                                            state.id = id.clone();

                                                        }

                                                        if let Some(function) = &tool_call.function {

                                                            if let Some(name) = &function.name {

                                                                state.name = name.clone();

                                                            }

                                                        }



                                                        let should_start =

                                                            !state.started

                                                                && !state.id.is_empty()

                                                                && !state.name.is_empty();

                                                        if should_start {

                                                            state.started = true;

                                                        }

                                                        let pending_after_start = if should_start

                                                            && !state.pending_args.is_empty()

                                                        {

                                                            Some(std::mem::take(&mut state.pending_args))

                                                        } else {

                                                            None

                                                        };

                                                        let args_delta = tool_call

                                                            .function

                                                            .as_ref()

                                                            .and_then(|f| f.arguments.clone());

                                                        let immediate_delta = if let Some(args) = args_delta {

                                                            // 鏃犻檺绌虹櫧 bug 妫娴嬶細璺熻釜杩炵画绌虹櫧瀛楃

                                                            for ch in args.chars() {

                                                                if ch.is_whitespace() {

                                                                    state.consecutive_whitespace += 1;

                                                                } else {

                                                                    state.consecutive_whitespace = 0;

                                                                }

                                                            }

                                                            if state.consecutive_whitespace >= INFINITE_WHITESPACE_THRESHOLD {

                                                                tracing::warn!(

                                    "[Copilot] bug (tool: {}), abort tool call",

                                                                    state.name

                                                                );

                                                                state.aborted = true;

                                                                None

                                                            } else if state.started {

                                                                Some(args)

                                                            } else {

                                                                state.pending_args.push_str(&args);

                                                                None

                                                            }

                                                        } else {

                                                            None

                                                        };

                                                        (

                                                            state.anthropic_index,

                                                            state.id.clone(),

                                                            state.name.clone(),

                                                            should_start,

                                                            pending_after_start,

                                                            immediate_delta,

                                                        )

                                                    };



                                                    if should_start {

                                                        let event = json!({

                                                            "type": "content_block_start",

                                                            "index": anthropic_index,

                                                            "content_block": {

                                                                "type": "tool_use",

                                                                "id": id,

                                                                "name": name

                                                            }

                                                        });

                                                        let sse_data = format!("event: content_block_start\ndata: {}\n\n",

                                                            serde_json::to_string(&event).unwrap_or_default());

                                                        yield Ok(Bytes::from(sse_data));

                                                        open_tool_block_indices.insert(anthropic_index);

                                                    }



                                                    if let Some(args) = pending_after_start {

                                                        let event = json!({

                                                            "type": "content_block_delta",

                                                            "index": anthropic_index,

                                                            "delta": {

                                                                "type": "input_json_delta",

                                                                "partial_json": args

                                                            }

                                                        });

                                                        let sse_data = format!("event: content_block_delta\ndata: {}\n\n",

                                                            serde_json::to_string(&event).unwrap_or_default());

                                                        yield Ok(Bytes::from(sse_data));

                                                    }



                                                    if let Some(args) = immediate_delta {

                                                        let event = json!({

                                                            "type": "content_block_delta",

                                                            "index": anthropic_index,

                                                            "delta": {

                                                                "type": "input_json_delta",

                                                                "partial_json": args

                                                            }

                                                        });

                                                        let sse_data = format!("event: content_block_delta\ndata: {}\n\n",

                                                            serde_json::to_string(&event).unwrap_or_default());

                                                        yield Ok(Bytes::from(sse_data));

                                                    }

                                                }

                                            }

                                        }



                                        // 澶勭悊 finish_reason?

                                        // 娉ㄦ剰锛歄penRouter 鏌愪簺 provider 浼氬彂閫佸氫釜甯 finish_reason ?chunk

                                        // 锛堢涓?usage ?null锛屽悗缁鎵嶈ˉ鍏锛夈傛ゅ勫彧鍋氱紦瀛橈紝涓嶇珛鍗冲彂閫侊紝

                                        // 绛夊埌 [DONE] 鎴栨祦鏈灏惧啀缁熶竴鍙戝嚭锛岀‘?usage 瀹屾暣涓斿彧鍙戜竴娆?

                                        if let Some(finish_reason) = &choice.finish_reason {

                                            let stop_reason = map_stop_reason(Some(finish_reason));

                                            let usage_json =

                                                chunk_usage_json.clone().or_else(|| latest_usage.clone());



                                            if has_emitted_message_delta {

                                                // 鏇存柊缂撳瓨?message_delta usage锛堝傛灉鏈夋洿瀹屾暣鐨 usage?

                                                if let (Some((_, usage)), Some(uj)) = (&mut pending_message_delta, usage_json) {

                                                    *usage = Some(uj);

                                                }

                                                continue;

                                            }

                                            has_emitted_message_delta = true;



                                            if let Some(index) = current_non_tool_block_index.take() {

                                                let event = json!({

                                                    "type": "content_block_stop",

                                                    "index": index

                                                });

                                                let sse_data = format!("event: content_block_stop\ndata: {}\n\n",

                                                    serde_json::to_string(&event).unwrap_or_default());

                                                yield Ok(Bytes::from(sse_data));

                                            }

                                            current_non_tool_block_type = None;



                                            // Late start for blocks that accumulated args before id/name arrived.

                                            let mut late_tool_starts: Vec<(u32, String, String, String)> =

                                                Vec::new();

                                            for (tool_idx, state) in tool_blocks_by_index.iter_mut() {

                                                if state.started {

                                                    continue;

                                                }

                                                let has_payload = !state.pending_args.is_empty()

                                                    || !state.id.is_empty()

                                                    || !state.name.is_empty();

                                                if !has_payload {

                                                    continue;

                                                }

                                                let fallback_id = if state.id.is_empty() {

                                                    format!("tool_call_{tool_idx}")

                                                } else {

                                                    state.id.clone()

                                                };

                                                let fallback_name = if state.name.is_empty() {

                                                    "unknown_tool".to_string()

                                                } else {

                                                    state.name.clone()

                                                };

                                                state.started = true;

                                                let pending = std::mem::take(&mut state.pending_args);

                                                late_tool_starts.push((

                                                    state.anthropic_index,

                                                    fallback_id,

                                                    fallback_name,

                                                    pending,

                                                ));

                                            }

                                            late_tool_starts.sort_unstable_by_key(|(index, _, _, _)| *index);

                                            for (index, id, name, pending) in late_tool_starts {

                                                let event = json!({

                                                    "type": "content_block_start",

                                                    "index": index,

                                                    "content_block": {

                                                        "type": "tool_use",

                                                        "id": id,

                                                        "name": name

                                                    }

                                                });

                                                let sse_data = format!("event: content_block_start\ndata: {}\n\n",

                                                    serde_json::to_string(&event).unwrap_or_default());

                                                yield Ok(Bytes::from(sse_data));

                                                open_tool_block_indices.insert(index);

                                                if !pending.is_empty() {

                                                    let delta_event = json!({

                                                        "type": "content_block_delta",

                                                        "index": index,

                                                        "delta": {

                                                            "type": "input_json_delta",

                                                            "partial_json": pending

                                                        }

                                                    });

                                                    let delta_sse = format!("event: content_block_delta\ndata: {}\n\n",

                                                        serde_json::to_string(&delta_event).unwrap_or_default());

                                                    yield Ok(Bytes::from(delta_sse));

                                                }

                                            }



                                            if !open_tool_block_indices.is_empty() {

                                                let mut tool_indices: Vec<u32> =

                                                    open_tool_block_indices.iter().copied().collect();

                                                tool_indices.sort_unstable();

                                                for index in tool_indices {

                                                    let event = json!({

                                                        "type": "content_block_stop",

                                                        "index": index

                                                    });

                                                    let sse_data = format!("event: content_block_stop\ndata: {}\n\n",

                                                        serde_json::to_string(&event).unwrap_or_default());

                                                    yield Ok(Bytes::from(sse_data));

                                                }

                                                open_tool_block_indices.clear();

                                            }



                                            // 缂撳瓨 message_delta锛岀瓑?[DONE] 鏃跺彂閫侊紙浠ヤ究鏀堕泦瀹屾暣?usage?

                                            pending_message_delta = Some((stop_reason, usage_json));

                                        }

                                    }

                                }

                            }

                        }

                    }

                }

                Err(e) => {

                    tracing::error!("Stream error: {e}");

                    stream_ended_with_error = true;

                    let error_event = json!({

                        "type": "error",

                        "error": {

                            "type": "stream_error",

                            "message": format!("Stream error: {e}")

                        }

                    });

                    let sse_data = format!("event: error\ndata: {}\n\n",

                        serde_json::to_string(&error_event).unwrap_or_default());

                    yield Ok(Bytes::from(sse_data));

                    break;

                }

            }

        }



        // 娴佽嚜鐒剁粨鏉熶絾鏈鏀?[DONE] 鏃讹紝纭淇濆彂閫佺紦瀛樼殑 message_delta ?message_stop?

        // 鑻ヤ笂娓稿凡鏄惧紡鎶ラ敊锛屽垯鍙淇?error 浜嬩欢锛岄伩鍏嶆妸澶辫触浼瑁呮垚鎴愬姛瀹屾?

        if !stream_ended_with_error {

            let emitted_pending_message_delta = if let Some((stop_reason, usage_json)) =

                pending_message_delta.take()

            {

                let event = build_message_delta_event(stop_reason, usage_json);

                let sse_data = format!("event: message_delta\ndata: {}\n\n",

                    serde_json::to_string(&event).unwrap_or_default());

                tracing::debug!("[Claude/OpenRouter] >>> Anthropic SSE: message_delta (at stream end)");

                yield Ok(Bytes::from(sse_data));

                true

            } else {

                false

            };



            if emitted_pending_message_delta && !has_sent_message_stop {

                let event = json!({"type": "message_stop"});

                let sse_data = format!("event: message_stop\ndata: {}\n\n",

                    serde_json::to_string(&event).unwrap_or_default());

                tracing::debug!("[Claude/OpenRouter] >>> Anthropic SSE: message_stop (at stream end)");

                yield Ok(Bytes::from(sse_data));

            }

        }

    }
}

/// Extract cache_read tokens from Usage, checking both direct field and nested details

fn extract_cache_read_tokens(usage: &Usage) -> Option<u32> {
    // Direct field takes priority (compatible servers)

    if let Some(v) = usage.cache_read_input_tokens {
        return Some(v);
    }

    // OpenAI standard: prompt_tokens_details.cached_tokens

    usage
        .prompt_tokens_details
        .as_ref()
        .map(|d| d.cached_tokens)
        .filter(|&v| v > 0)
}

/// Extract cache-write tokens from direct compatibility fields or OpenAI details.

fn extract_cache_write_tokens(usage: &Usage) -> Option<u32> {
    if let Some(value) = usage.cache_creation_input_tokens {
        return Some(value);
    }

    usage
        .prompt_tokens_details
        .as_ref()
        .map(|details| details.cache_write_tokens)
        .filter(|value| *value > 0)
}

/// 鏄犲皠鍋滄㈠師鍥

fn map_stop_reason(finish_reason: Option<&str>) -> Option<String> {
    finish_reason.map(|r| {
        match r {
            "tool_calls" | "function_call" => "tool_use",

            "stop" => "end_turn",

            "length" => "max_tokens",

            "content_filter" => "end_turn",

            other => {
                tracing::warn!("[Claude/OpenRouter] Unknown finish_reason in streaming: {other}");

                "end_turn"
            }
        }
        .to_string()
    })
}

pub fn create_gemini_sse_stream<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    crate::services::protocol_converter::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    while let Some(line) = take_sse_block(&mut buffer) {
                        if line.trim().is_empty() {
                            continue;
                        }

                        for l in line.lines() {
                            if let Some(data) = strip_sse_field(l, "data") {
                                if data.trim() == "[DONE]" {
                                    // Gemini doesn't use [DONE], but we can just forward empty or skip
                                    continue;
                                }

                                if let Ok(chunk_data) = serde_json::from_str::<Value>(data) {
                                    let content = chunk_data.pointer("/choices/0/delta/content").and_then(|c| c.as_str()).unwrap_or("");
                                    let finish_reason = chunk_data.pointer("/choices/0/finish_reason").and_then(|f| f.as_str());

                                    if !content.is_empty() || finish_reason.is_some() {
                                        let gemini_resp = json!({
                                            "candidates": [
                                                {
                                                    "content": {
                                                        "parts": [
                                                            {
                                                                "text": content
                                                            }
                                                        ],
                                                        "role": "model"
                                                    },
                                                    "finishReason": finish_reason.unwrap_or(""),
                                                    "index": 0,
                                                    "safetyRatings": []
                                                }
                                            ]
                                        });
                                        let sse_data = format!("data: {}\n\n", serde_json::to_string(&gemini_resp).unwrap_or_default());
                                        yield Ok(Bytes::from(sse_data));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    use futures::stream;

    use futures::StreamExt;

    use serde_json::Value;

    use std::collections::HashMap;

    async fn collect_anthropic_events(input: &str) -> Vec<Value> {
        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);

        let converted = create_anthropic_sse_stream(upstream);

        let chunks: Vec<_> = converted.collect().await;

        let merged = chunks
            .into_iter()
            .map(|chunk| String::from_utf8_lossy(chunk.unwrap().as_ref()).to_string())
            .collect::<String>();

        merged
            .split("\n\n")
            .filter_map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| strip_sse_field(line, "data"))?;

                serde_json::from_str::<Value>(data).ok()
            })
            .collect()
    }

    fn event_type(event: &Value) -> Option<&str> {
        event.get("type").and_then(|v| v.as_str())
    }

    #[test]

    fn test_map_stop_reason_legacy_and_filtered_values() {
        assert_eq!(
            map_stop_reason(Some("function_call")),
            Some("tool_use".to_string())
        );

        assert_eq!(
            map_stop_reason(Some("content_filter")),
            Some("end_turn".to_string())
        );
    }

    #[tokio::test]

    async fn test_streaming_tool_calls_routed_by_index() {
        let input = concat!(

            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"type\":\"function\",\"function\":{\"name\":\"first_tool\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"second_tool\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"{\\\"b\\\":2}\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"a\\\":1}\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":4}}\n\n",

            "data: [DONE]\n\n"

        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);

        let converted = create_anthropic_sse_stream(upstream);

        let chunks: Vec<_> = converted.collect().await;

        let merged = chunks
            .into_iter()
            .map(|chunk| String::from_utf8_lossy(chunk.unwrap().as_ref()).to_string())
            .collect::<String>();

        let events: Vec<Value> = merged
            .split("\n\n")
            .filter_map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| strip_sse_field(line, "data"))?;

                serde_json::from_str::<Value>(data).ok()
            })
            .collect();

        let mut tool_index_by_call: HashMap<String, u64> = HashMap::new();

        for event in &events {
            if event.get("type").and_then(|v| v.as_str()) == Some("content_block_start")
                && event
                    .pointer("/content_block/type")
                    .and_then(|v| v.as_str())
                    == Some("tool_use")
            {
                if let (Some(call_id), Some(index)) = (
                    event.pointer("/content_block/id").and_then(|v| v.as_str()),
                    event.get("index").and_then(|v| v.as_u64()),
                ) {
                    tool_index_by_call.insert(call_id.to_string(), index);
                }
            }
        }

        assert_eq!(tool_index_by_call.len(), 2);

        assert_ne!(
            tool_index_by_call.get("call_0"),
            tool_index_by_call.get("call_1")
        );

        let deltas: Vec<(u64, String)> = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(|v| v.as_str()) == Some("content_block_delta")
                    && event.pointer("/delta/type").and_then(|v| v.as_str())
                        == Some("input_json_delta")
            })
            .filter_map(|event| {
                let index = event.get("index").and_then(|v| v.as_u64())?;

                let partial_json = event
                    .pointer("/delta/partial_json")
                    .and_then(|v| v.as_str())?
                    .to_string();

                Some((index, partial_json))
            })
            .collect();

        assert_eq!(deltas.len(), 2);

        let second_idx = deltas
            .iter()
            .find_map(|(index, payload)| (payload == "{\"b\":2}").then_some(*index))
            .unwrap();

        let first_idx = deltas
            .iter()
            .find_map(|(index, payload)| (payload == "{\"a\":1}").then_some(*index))
            .unwrap();

        assert_eq!(second_idx, *tool_index_by_call.get("call_1").unwrap());

        assert_eq!(first_idx, *tool_index_by_call.get("call_0").unwrap());

        assert!(events.iter().any(|event| {
            event.get("type").and_then(|v| v.as_str()) == Some("message_delta")
                && event.pointer("/delta/stop_reason").and_then(|v| v.as_str()) == Some("tool_use")
        }));
    }

    #[tokio::test]

    async fn test_streaming_delays_tool_start_until_id_and_name_ready() {
        let input = concat!(

            "data: {\"id\":\"chatcmpl_2\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"a\\\":\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_2\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"type\":\"function\",\"function\":{\"name\":\"first_tool\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_2\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_2\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":6,\"completion_tokens\":2}}\n\n",

            "data: [DONE]\n\n"

        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);

        let converted = create_anthropic_sse_stream(upstream);

        let chunks: Vec<_> = converted.collect().await;

        let merged = chunks
            .into_iter()
            .map(|chunk| String::from_utf8_lossy(chunk.unwrap().as_ref()).to_string())
            .collect::<String>();

        let events: Vec<Value> = merged
            .split("\n\n")
            .filter_map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| strip_sse_field(line, "data"))?;

                serde_json::from_str::<Value>(data).ok()
            })
            .collect();

        let starts: Vec<&Value> = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(|v| v.as_str()) == Some("content_block_start")
                    && event
                        .pointer("/content_block/type")
                        .and_then(|v| v.as_str())
                        == Some("tool_use")
            })
            .collect();

        assert_eq!(starts.len(), 1);

        assert_eq!(
            starts[0]
                .pointer("/content_block/id")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "call_0"
        );

        assert_eq!(
            starts[0]
                .pointer("/content_block/name")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "first_tool"
        );

        let deltas: Vec<&str> = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(|v| v.as_str()) == Some("content_block_delta")
                    && event.pointer("/delta/type").and_then(|v| v.as_str())
                        == Some("input_json_delta")
            })
            .filter_map(|event| {
                event
                    .pointer("/delta/partial_json")
                    .and_then(|v| v.as_str())
            })
            .collect();

        assert!(deltas.contains(&"{\"a\":"));

        assert!(deltas.contains(&"1}"));
    }

    #[tokio::test]

    async fn test_streaming_chinese_split_across_chunks_no_replacement_chars() {
        // "你好" split across two TCP chunks inside a streaming text delta.

        // Before the fix, from_utf8_lossy would produce U+FFFD for each half.

        let full = concat!(

            "data: {\"id\":\"chatcmpl_3\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n",

            "data: {\"id\":\"chatcmpl_3\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",

            "data: [DONE]\n\n"

        );

        let bytes = full.as_bytes();

        // Find "? in the byte stream and split inside it

        let ni_start = bytes.windows(3).position(|w| w == "你".as_bytes()).unwrap();

        let split_point = ni_start + 1; // split after first byte of "?

        let chunk1 = Bytes::from(bytes[..split_point].to_vec());

        let chunk2 = Bytes::from(bytes[split_point..].to_vec());

        let upstream = stream::iter(vec![
            Ok::<_, std::io::Error>(chunk1),
            Ok::<_, std::io::Error>(chunk2),
        ]);

        let converted = create_anthropic_sse_stream(upstream);

        let chunks: Vec<_> = converted.collect().await;

        let merged = chunks
            .into_iter()
            .map(|chunk| String::from_utf8_lossy(chunk.unwrap().as_ref()).to_string())
            .collect::<String>();

        // Must contain the original Chinese characters, not replacement chars

        assert!(
            merged.contains("你好"),
            "expected '你好' in output, got replacement chars (U+FFFD)"
        );

        assert!(
            !merged.contains('\u{FFFD}'),
            "output must not contain U+FFFD replacement characters"
        );
    }

    #[tokio::test]

    async fn test_duplicate_finish_reason_emits_only_one_message_delta() {
        // Simulates OpenRouter behavior where two chunks carry finish_reason:

        // first with null usage, second with populated usage.

        let input = concat!(

            "data: {\"id\":\"chatcmpl_dup\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",

            "data: {\"id\":\"chatcmpl_dup\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",

            "data: [DONE]\n\n"

        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);

        let converted = create_anthropic_sse_stream(upstream);

        let chunks: Vec<_> = converted.collect().await;

        let merged = chunks
            .into_iter()
            .map(|chunk| String::from_utf8_lossy(chunk.unwrap().as_ref()).to_string())
            .collect::<String>();

        let events: Vec<Value> = merged
            .split("\n\n")
            .filter_map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| strip_sse_field(line, "data"))?;

                serde_json::from_str::<Value>(data).ok()
            })
            .collect();

        let message_deltas: Vec<&Value> = events
            .iter()
            .filter(|e| e.get("type").and_then(|v| v.as_str()) == Some("message_delta"))
            .collect();

        assert_eq!(
            message_deltas.len(),
            1,
            "duplicate finish_reason chunks must produce exactly one message_delta, got {}: {:?}",
            message_deltas.len(),
            message_deltas
        );

        assert_eq!(message_deltas[0]["usage"]["input_tokens"], 10);

        assert_eq!(message_deltas[0]["usage"]["output_tokens"], 5);

        let message_stops = events
            .iter()
            .filter(|e| e.get("type").and_then(|v| v.as_str()) == Some("message_stop"))
            .count();

        assert_eq!(message_stops, 1, "message_stop must only be emitted once");
    }

    #[tokio::test]

    async fn test_usage_only_chunk_after_finish_reason_updates_message_delta_usage() {
        let input = concat!(

            "data: {\"id\":\"chatcmpl_split\",\"model\":\"glm-5.1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"tool-0924\",\"type\":\"function\",\"function\":{\"name\":\"Bash\",\"arguments\":\"{\\\"command\\\":\\\"pwd\\\"}\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_split\",\"model\":\"glm-5.1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",

            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":13312,\"completion_tokens\":79,\"prompt_tokens_details\":{\"cached_tokens\":100}}}\n\n",

            "data: [DONE]\n\n"

        );

        let events = collect_anthropic_events(input).await;

        let message_deltas: Vec<&Value> = events
            .iter()
            .filter(|event| event_type(event) == Some("message_delta"))
            .collect();

        let message_stops = events
            .iter()
            .filter(|event| event_type(event) == Some("message_stop"))
            .count();

        assert_eq!(message_deltas.len(), 1);

        assert_eq!(message_stops, 1);

        let message_delta = message_deltas[0];

        assert_eq!(
            message_delta
                .pointer("/delta/stop_reason")
                .and_then(|v| v.as_str()),
            Some("tool_use")
        );

        assert_eq!(
            message_delta
                .pointer("/usage/input_tokens")
                .and_then(|v| v.as_u64()),
            Some(13212)
        );

        assert_eq!(
            message_delta
                .pointer("/usage/output_tokens")
                .and_then(|v| v.as_u64()),
            Some(79)
        );

        assert_eq!(
            message_delta
                .pointer("/usage/cache_read_input_tokens")
                .and_then(|v| v.as_u64()),
            Some(100)
        );
    }

    #[tokio::test]

    async fn test_usage_chunk_subtracts_cache_read_and_creation_from_input() {
        // prompt_tokens(1000) ?cache_read(600) ?cache_creation(300)锛涜浆 Anthropic ?

        // input 搴斾负 fresh锛屽畧鎭掞細input(100) + cache_read(600) + cache_creation(300) == prompt(1000)?

        let input = concat!(

            "data: {\"id\":\"chatcmpl_cc\",\"model\":\"glm-5.1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"tool-1\",\"type\":\"function\",\"function\":{\"name\":\"Bash\",\"arguments\":\"{\\\"command\\\":\\\"pwd\\\"}\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_cc\",\"model\":\"glm-5.1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",

            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1000,\"completion_tokens\":50,\"prompt_tokens_details\":{\"cached_tokens\":600,\"cache_write_tokens\":300}}}\n\n",

            "data: [DONE]\n\n"

        );

        let events = collect_anthropic_events(input).await;

        let message_delta = events
            .iter()
            .find(|event| event_type(event) == Some("message_delta"))
            .expect("should emit message_delta with usage");

        // fresh input = 1000 - 600 - 300 = 100

        assert_eq!(
            message_delta
                .pointer("/usage/input_tokens")
                .and_then(|v| v.as_u64()),
            Some(100)
        );

        assert_eq!(
            message_delta
                .pointer("/usage/cache_read_input_tokens")
                .and_then(|v| v.as_u64()),
            Some(600)
        );

        assert_eq!(
            message_delta
                .pointer("/usage/cache_creation_input_tokens")
                .and_then(|v| v.as_u64()),
            Some(300)
        );
    }

    #[tokio::test]

    async fn test_usage_chunk_clamps_input_to_zero_when_cache_exceeds_prompt() {
        // prompt(100) < cache_read(80)+cache_creation(50)=130锛歴aturating 閽冲埌 0锛岄槻涓嬫孩?

        // 閽夋々锛氶樆姝㈡湭鏉ユ妸 saturating_sub 璇鏀规垚鏅閫氬噺?debug panic / release wrap)?

        let input = concat!(

            "data: {\"id\":\"chatcmpl_uf\",\"model\":\"glm-5.1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"tool-1\",\"type\":\"function\",\"function\":{\"name\":\"Bash\",\"arguments\":\"{\\\"command\\\":\\\"pwd\\\"}\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_uf\",\"model\":\"glm-5.1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",

            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":50,\"prompt_tokens_details\":{\"cached_tokens\":80},\"cache_creation_input_tokens\":50}}\n\n",

            "data: [DONE]\n\n"

        );

        let events = collect_anthropic_events(input).await;

        let message_delta = events
            .iter()
            .find(|event| event_type(event) == Some("message_delta"))
            .expect("should emit message_delta with usage");

        assert_eq!(
            message_delta
                .pointer("/usage/input_tokens")
                .and_then(|v| v.as_u64()),
            Some(0)
        );

        assert_eq!(
            message_delta
                .pointer("/usage/cache_read_input_tokens")
                .and_then(|v| v.as_u64()),
            Some(80)
        );

        assert_eq!(
            message_delta
                .pointer("/usage/cache_creation_input_tokens")
                .and_then(|v| v.as_u64()),
            Some(50)
        );
    }

    #[tokio::test]

    async fn test_message_delta_includes_zero_usage_when_stream_has_no_usage() {
        let input = concat!(

            "data: {\"id\":\"chatcmpl_no_usage\",\"model\":\"gpt-5.5\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"type\":\"function\",\"function\":{\"name\":\"get_time\",\"arguments\":\"{}\"}}]}}]}\n\n",

            "data: {\"id\":\"chatcmpl_no_usage\",\"model\":\"gpt-5.5\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",

            "data: [DONE]\n\n"

        );

        let events = collect_anthropic_events(input).await;

        let message_deltas: Vec<&Value> = events
            .iter()
            .filter(|event| event_type(event) == Some("message_delta"))
            .collect();

        assert_eq!(message_deltas.len(), 1);

        let message_delta = message_deltas[0];

        assert_eq!(
            message_delta
                .pointer("/delta/stop_reason")
                .and_then(|v| v.as_str()),
            Some("tool_use")
        );

        assert_eq!(
            message_delta
                .pointer("/usage/input_tokens")
                .and_then(|v| v.as_u64()),
            Some(0)
        );

        assert_eq!(
            message_delta
                .pointer("/usage/output_tokens")
                .and_then(|v| v.as_u64()),
            Some(0)
        );
    }

    #[tokio::test]

    async fn test_streaming_finalizes_after_finish_when_done_is_missing() {
        let input = concat!(

            "data: {\"id\":\"chatcmpl_no_done\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",

            "data: {\"id\":\"chatcmpl_no_done\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n"

        );

        let events = collect_anthropic_events(input).await;

        assert!(events.iter().any(|event| {
            event_type(event) == Some("message_delta")
                && event.pointer("/delta/stop_reason").and_then(|v| v.as_str()) == Some("end_turn")
        }));

        assert_eq!(
            events.last().and_then(|event| event_type(event)),
            Some("message_stop")
        );
    }

    #[tokio::test]

    async fn test_stream_end_without_finish_reason_does_not_emit_success_terminal_events() {
        let input = "data: {\"id\":\"chatcmpl_truncated\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n";

        let events = collect_anthropic_events(input).await;

        assert!(!events
            .iter()
            .any(|event| event_type(event) == Some("message_delta")));

        assert!(!events
            .iter()
            .any(|event| event_type(event) == Some("message_stop")));
    }

    #[tokio::test]

    async fn test_stream_error_does_not_emit_success_terminal_events() {
        let upstream = stream::iter(vec![Err::<Bytes, _>(std::io::Error::other(
            "upstream disconnected",
        ))]);

        let converted = create_anthropic_sse_stream(upstream);

        let chunks: Vec<_> = converted.collect().await;

        let merged = chunks
            .into_iter()
            .map(|chunk| String::from_utf8_lossy(chunk.unwrap().as_ref()).to_string())
            .collect::<String>();

        let events: Vec<Value> = merged
            .split("\n\n")
            .filter_map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| strip_sse_field(line, "data"))?;

                serde_json::from_str::<Value>(data).ok()
            })
            .collect();

        assert!(events
            .iter()
            .any(|e| e.get("type").and_then(|v| v.as_str()) == Some("error")));

        assert!(!events
            .iter()
            .any(|e| e.get("type").and_then(|v| v.as_str()) == Some("message_delta")));

        assert!(!events
            .iter()
            .any(|e| e.get("type").and_then(|v| v.as_str()) == Some("message_stop")));
    }
}
