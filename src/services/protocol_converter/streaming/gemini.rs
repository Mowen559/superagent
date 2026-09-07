//! 娴佸紡鍝嶅簲杞鎹㈡ā鍧

//!

//! 瀹炵幇 OpenAI SSE -> Anthropic SSE 鏍煎紡杞鎹

use crate::services::protocol_converter::sse::{strip_sse_field, take_sse_block};

use bytes::Bytes;

use futures::stream::{Stream, StreamExt};

use serde::{Deserialize, Serialize};

use serde_json::{json, Value};

use std::collections::{HashMap, HashSet};


use super::*;
use super::types::*;

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

