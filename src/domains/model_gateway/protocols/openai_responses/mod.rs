use std::{io, pin::Pin};
use bytes::Bytes;
use futures::stream::Stream;
use serde_json::Value;

use super::ProtocolConverter;

mod request;
mod response;
mod stream;

pub struct OpenAiResponsesConverter;

impl ProtocolConverter for OpenAiResponsesConverter {
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        request::responses_request_to_canonical(body)
    }

    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error> {
        response::canonical_response_to_responses(body)
    }

    fn standard_to_response_stream(
        &self,
        stream: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>> {
        Box::pin(stream::canonical_stream_to_responses(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::domains::model_gateway::protocols::ProtocolConverter;
    use futures::StreamExt;

    #[test]
    fn request_maps_messages_function_history_and_flat_tools() {
        let canonical = super::OpenAiResponsesConverter
            .request_to_standard(json!({
                "model": "gpt-test",
                "instructions": "Be precise",
                "input": [
                    {"role": "user", "content": [{"type": "input_text", "text": "Read a.rs"}]},
                    {"type": "function_call", "call_id": "call-1", "name": "read_file", "arguments": "{\"path\":\"a.rs\"}"},
                    {"type": "function_call_output", "call_id": "call-1", "output": "contents"}
                ],
                "tools": [{"type": "function", "name": "read_file", "description": "Read", "parameters": {"type": "object"}, "strict": true}],
                "max_output_tokens": 100,
                "stream": true
            }))
            .unwrap();

        assert_eq!(canonical["messages"][0]["role"], "system");
        assert_eq!(canonical["messages"][2]["tool_calls"][0]["id"], "call-1");
        assert_eq!(canonical["messages"][3]["role"], "tool");
        assert_eq!(canonical["tools"][0]["function"]["name"], "read_file");
        assert_eq!(canonical["max_tokens"], 100);
        assert_eq!(canonical["stream_options"]["include_usage"], true);
    }

    #[test]
    fn request_rejects_provider_specific_built_in_tools() {
        let error = super::OpenAiResponsesConverter
            .request_to_standard(json!({
                "model": "gpt-test",
                "input": "search",
                "tools": [{"type": "web_search"}]
            }))
            .unwrap_err();
        assert!(error.to_string().contains("no provider-independent"));

        let error = super::OpenAiResponsesConverter
            .request_to_standard(json!({
                "model": "gpt-test",
                "previous_response_id": "resp-prior",
                "input": "continue"
            }))
            .unwrap_err();
        assert!(error.to_string().contains("stateful Responses store"));
    }

    #[test]
    fn response_maps_text_tools_usage_and_incomplete_status() {
        let response = super::OpenAiResponsesConverter
            .standard_to_response(json!({
                "id": "chatcmpl-1", "created": 10, "model": "gpt-test",
                "choices": [{"message": {"role": "assistant", "content": "Checking", "tool_calls": [{
                    "id": "call-1", "type": "function", "function": {"name": "read_file", "arguments": "{}"}
                }]}, "finish_reason": "length"}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}
            }))
            .unwrap();

        assert_eq!(response["object"], "response");
        assert_eq!(response["status"], "incomplete");
        assert_eq!(response["output"][0]["type"], "message");
        assert_eq!(response["output"][1]["type"], "function_call");
        assert_eq!(response["usage"]["total_tokens"], 15);
    }

    #[tokio::test]
    async fn stream_emits_responses_text_lifecycle_and_terminal_usage() {
        let raw = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"created\":10,\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"delta\":{\"content\":\"你\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\n",
            "data: [DONE]\n\n"
        );
        let split = raw.find("你").unwrap() + 1;
        let input = futures::stream::iter(vec![
            Ok(Bytes::copy_from_slice(&raw.as_bytes()[..split])),
            Ok(Bytes::copy_from_slice(&raw.as_bytes()[split..])),
        ]);
        let rendered = super::OpenAiResponsesConverter
            .standard_to_response_stream(Box::pin(input))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();

        assert!(rendered.contains("response.created"));
        assert!(rendered.contains("response.output_text.delta"));
        assert!(rendered.contains("你"));
        assert!(rendered.contains("response.completed"));
        assert!(rendered.contains("\"total_tokens\":5"));
    }

    #[tokio::test]
    async fn stream_maps_function_call_argument_deltas() {
        let raw = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let input = futures::stream::iter(vec![Ok(Bytes::from_static(raw.as_bytes()))]);
        let rendered = super::OpenAiResponsesConverter
            .standard_to_response_stream(Box::pin(input))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();

        assert!(rendered.contains("response.function_call_arguments.delta"));
        assert!(rendered.contains("response.function_call_arguments.done"));
        assert!(rendered.contains("read_file"));
        assert!(rendered.contains("response.completed"));
    }

    #[tokio::test]
    async fn stream_without_finish_reason_emits_failed_not_completed() {
        let raw = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n"
        );
        let input = futures::stream::iter(vec![Ok(Bytes::from_static(raw.as_bytes()))]);
        let rendered = super::OpenAiResponsesConverter
            .standard_to_response_stream(Box::pin(input))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();

        assert!(rendered.contains("response.failed"));
        assert!(!rendered.contains("event: response.completed"));
    }
}
