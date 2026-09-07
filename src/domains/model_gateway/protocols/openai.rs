use super::ProtocolConverter;
use serde_json::Value;

pub struct OpenAiConverter;

impl ProtocolConverter for OpenAiConverter {
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        // The standard internal format is OpenAI Chat Completions format,
        // so we can largely just pass this through.
        Ok(body)
    }

    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error> {
        // Passthrough for OpenAI standard format
        Ok(body)
    }

    fn standard_to_response_stream(
        &self,
        stream: std::pin::Pin<
            Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
        >,
    ) -> std::pin::Pin<
        Box<dyn futures::stream::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
    > {
        use futures::stream::StreamExt;
        Box::pin(stream.map(|res| res.map_err(|e| std::io::Error::other(e))))
    }
}
