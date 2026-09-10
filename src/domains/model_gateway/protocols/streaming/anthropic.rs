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
use super::types::*;

pub(crate) fn build_anthropic_usage_json(usage: &Usage) -> Value {
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

pub(crate) fn default_anthropic_usage_json() -> Value {
    json!({

        "input_tokens": 0,

        "output_tokens": 0

    })
}

pub(crate) fn build_message_delta_event(stop_reason: Option<String>, usage_json: Option<Value>) -> Value {
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

                    crate::domains::model_gateway::protocols::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);



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

pub(crate) fn extract_cache_read_tokens(usage: &Usage) -> Option<u32> {
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

pub(crate) fn extract_cache_write_tokens(usage: &Usage) -> Option<u32> {
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

pub(crate) fn map_stop_reason(finish_reason: Option<&str>) -> Option<String> {
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

