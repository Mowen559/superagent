
pub fn create_completions_sse_stream<E: std::error::Error + Send + 'static>(
    stream: impl futures::stream::Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
) -> impl futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        use futures::StreamExt;
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        tokio::pin!(stream);

        while let Some(chunk_res) = stream.next().await {
            match chunk_res {
                Ok(bytes) => {
                    crate::services::protocol_converter::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    while let Some(line) = crate::services::protocol_converter::sse::take_sse_block(&mut buffer) {
                        if line.trim().is_empty() {
                            continue;
                        }

                        for l in line.lines() {
                            if let Some(data) = crate::services::protocol_converter::sse::strip_sse_field(l, "data") {
                                if data.trim() == "[DONE]" {
                                    let out = format!("data: [DONE]\n\n");
                                    yield Ok(bytes::Bytes::from(out));
                                    continue;
                                }

                                if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(data) {
                                    if let Some(choices) = val.get_mut("choices").and_then(|c| c.as_array_mut()) {
                                        for choice in choices.iter_mut() {
                                            if let Some(delta) = choice.get("delta") {
                                                if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                                    choice["text"] = serde_json::json!(content);
                                                } else {
                                                    choice["text"] = serde_json::json!("");
                                                }
                                                if let Some(obj) = choice.as_object_mut() {
                                                    obj.remove("delta");
                                                }
                                            }
                                        }
                                    }
                                    if let Some(obj) = val.as_object_mut() {
                                        obj.insert("object".to_string(), serde_json::json!("text_completion"));
                                    }
                                    
                                    let out = format!("data: {}\n\n", val.to_string());
                                    yield Ok(bytes::Bytes::from(out));
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
