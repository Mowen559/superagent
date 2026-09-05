use super::ProtocolConverter;
use serde_json::{json, Value};

pub struct OpenAiCompletionsConverter;

impl ProtocolConverter for OpenAiCompletionsConverter {
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        let mut result = body.clone();

        // Map "prompt" to "messages"
        if let Some(prompt) = body.get("prompt") {
            let text = if let Some(s) = prompt.as_str() {
                s.to_string()
            } else if let Some(arr) = prompt.as_array() {
                let texts: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                texts.join("\n")
            } else {
                "".to_string()
            };

            result["messages"] = json!([
                {
                    "role": "user",
                    "content": text
                }
            ]);

            // Remove "prompt" to avoid confusing chat/completions upstream
            if let Some(obj) = result.as_object_mut() {
                obj.remove("prompt");
            }
        }

        Ok(result)
    }

    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error> {
        let mut result = body.clone();

        // Map "message" -> "text"
        if let Some(choices) = result.get_mut("choices").and_then(|c| c.as_array_mut()) {
            for choice in choices.iter_mut() {
                if let Some(message) = choice.get("message") {
                    if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                        choice["text"] = json!(content);
                    }
                    if let Some(obj) = choice.as_object_mut() {
                        obj.remove("message");
                    }
                }
            }
        }

        // Change object type
        if let Some(obj) = result.as_object_mut() {
            obj.insert("object".to_string(), json!("text_completion"));
        }

        Ok(result)
    }

    fn standard_to_response_stream(
        &self,
        stream: std::pin::Pin<
            Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
        >,
    ) -> std::pin::Pin<
        Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
    > {
        Box::pin(
            crate::services::protocol_converter::streaming::create_completions_sse_stream(stream),
        )
    }
}
