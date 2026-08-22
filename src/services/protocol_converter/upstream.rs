use bytes::Bytes;
use futures::stream::Stream;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

use crate::config::llm_settings::LLMSettings;

use super::{
    anthropic_upstream::AnthropicUpstreamAdapter,
    gemini_internal_upstream::GeminiInternalUpstreamAdapter,
};

pub fn adapter_for_provider(provider: &str) -> Box<dyn UpstreamAdapter> {
    match provider.to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => Box::new(AnthropicUpstreamAdapter),
        "gemini_native" => Box::new(GeminiInternalUpstreamAdapter),
        "github_copilot" => Box::new(CopilotUpstreamAdapter),
        _ => Box::new(OpenAiUpstreamAdapter),
    }
}

pub trait UpstreamAdapter: Send + Sync {
    /// Adapt one Canonical Model Request to a provider-specific request.
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error>;

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error>;

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
        let mut headers = bearer_headers(&settings.api_key)?;
        if settings.api_key.starts_with("ya29.") {
            if let Some(project_id) = settings.project_id.as_deref() {
                headers.insert("x-goog-user-project", HeaderValue::from_str(project_id)?);
            }
        }
        Ok((openai_chat_url(settings), headers, standard_body))
    }

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        Ok(body)
    }

    fn response_stream_to_standard(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        stream
    }
}

pub struct CopilotUpstreamAdapter;

impl UpstreamAdapter for CopilotUpstreamAdapter {
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error> {
        let base = settings
            .base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1");
        let mut headers = bearer_headers(&settings.api_key)?;
        headers.insert("Editor-Version", HeaderValue::from_static("vscode/1.85.0"));
        headers.insert(
            "Editor-Plugin-Version",
            HeaderValue::from_static("copilot-chat/0.11.1"),
        );
        headers.insert(
            "OpenAI-Organization",
            HeaderValue::from_static("github-copilot"),
        );
        headers.insert(
            "OpenAI-Intent",
            HeaderValue::from_static("conversation-panel"),
        );
        Ok((format!("{base}/chat/completions"), headers, standard_body))
    }

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        Ok(body)
    }

    fn response_stream_to_standard(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        stream
    }
}

fn bearer_headers(api_key: &str) -> Result<HeaderMap, anyhow::Error> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))?,
    );
    Ok(headers)
}

fn openai_chat_url(settings: &LLMSettings) -> String {
    let base = settings.base_url.trim_end_matches('/');
    let provider = settings.provider.to_ascii_lowercase();
    let is_google_openai = provider.contains("gemini")
        || provider.contains("google")
        || base.contains("generativelanguage.googleapis.com");

    if is_google_openai {
        if base.ends_with("/v1beta/openai") {
            format!("{base}/chat/completions")
        } else if base.ends_with("/v1beta") {
            format!("{base}/openai/chat/completions")
        } else if base.contains("/v1beta/") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1beta/openai/chat/completions")
        }
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_adapter_preserves_google_compatibility_paths() {
        let settings = LLMSettings {
            provider: "gemini".to_string(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            api_key: "key".to_string(),
            ..LLMSettings::default()
        };
        assert_eq!(
            openai_chat_url(&settings),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );

        let settings = LLMSettings {
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
            ..settings
        };
        assert_eq!(
            openai_chat_url(&settings),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
    }
}
