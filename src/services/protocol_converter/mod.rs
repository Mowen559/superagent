pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod openai_responses;
pub mod sse;
pub mod streaming;
pub mod upstream;

use bytes::Bytes;
use futures::stream::Stream;
use serde_json::Value;

pub trait ProtocolConverter: Send + Sync {
    /// Convert an incoming request into a standard (OpenAI-compatible) chat completions request.
    fn request_to_standard(&self, body: Value) -> Result<Value, anyhow::Error>;

    /// Convert a standard (OpenAI-compatible) response back into the incoming protocol's response format.
    fn standard_to_response(&self, body: Value) -> Result<Value, anyhow::Error>;

    /// Convert a standard (OpenAI-compatible) SSE stream back into the incoming protocol's stream format.
    fn standard_to_response_stream(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;
}
