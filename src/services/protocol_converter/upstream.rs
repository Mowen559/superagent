use bytes::Bytes;
use futures::stream::Stream;
use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::config::llm_settings::LLMSettings;

pub trait UpstreamAdapter: Send + Sync {
    /// Convert standard (OpenAI-compatible) request to the upstream format, and return the target URL, headers, and body.
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error>;

    /// Convert upstream response to standard (OpenAI-compatible) response
    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error>;

    /// Convert upstream SSE stream to standard (OpenAI-compatible) SSE stream
    fn response_stream_to_standard(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;
}

pub struct OpenAiUpstreamAdapter;

impl UpstreamAdapter for OpenAiUpstreamAdapter {
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error> {
        let base = settings.base_url.trim_end_matches("/v1");
        let target_url = format!("{}/v1/chat/completions", base);
        
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse().unwrap());
        headers.insert(
            "Authorization",
            format!("Bearer {}", settings.api_key).parse().unwrap(),
        );

        Ok((target_url, headers, standard_body))
    }

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        // Already standard format
        Ok(body)
    }

    fn response_stream_to_standard(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        // Just map errors for passthrough
        Box::pin(futures::stream::StreamExt::map(stream, |res| {
            res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        }))
    }
}

pub struct CopilotUpstreamAdapter;

impl UpstreamAdapter for CopilotUpstreamAdapter {
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error> {
        let base = settings.base_url.trim_end_matches("/v1");
        let target_url = format!("{}/chat/completions", base); // Copilot endpoint is /chat/completions
        
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse().unwrap());
        headers.insert(
            "Authorization",
            format!("Bearer {}", settings.api_key).parse().unwrap(),
        );
        // Inject required Copilot headers
        headers.insert("Editor-Version", "vscode/1.85.0".parse().unwrap());
        headers.insert("Editor-Plugin-Version", "copilot-chat/0.11.1".parse().unwrap());
        headers.insert("OpenAI-Organization", "github-copilot".parse().unwrap());
        headers.insert("OpenAI-Intent", "conversation-panel".parse().unwrap());

        Ok((target_url, headers, standard_body))
    }

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        // Already standard format
        Ok(body)
    }

    fn response_stream_to_standard(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        // Just map errors for passthrough
        Box::pin(futures::stream::StreamExt::map(stream, |res| {
            res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        }))
    }
}
