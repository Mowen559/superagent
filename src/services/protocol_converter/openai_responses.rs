use super::ProtocolConverter;
use serde_json::Value;

pub struct OpenAiResponsesConverter;

impl ProtocolConverter for OpenAiResponsesConverter {
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        // Here we map from `/v1/responses` (Codex specific features) to standard `/v1/chat/completions`.
        // For now, act as passthrough until more specific mapping is required.
        Ok(body.clone())
    }

    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error> {
        Ok(body)
    }

    fn standard_to_response_stream(
        &self,
        stream: std::pin::Pin<Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>> {
        use futures::stream::StreamExt;
        Box::pin(stream.map(|res| res.map_err(|e| std::io::Error::other(e))))
    }
}
