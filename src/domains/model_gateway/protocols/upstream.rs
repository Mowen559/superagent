use bytes::Bytes;
use futures::stream::Stream;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::config::llm_settings::LLMSettings;

use super::{
    anthropic_upstream::AnthropicUpstreamAdapter,
    gemini_internal_upstream::GeminiInternalUpstreamAdapter,
};

pub fn adapter_for_provider(settings: &LLMSettings) -> Box<dyn UpstreamAdapter> {
    if settings.base_url.contains("/backend-api/") {
        return Box::new(ChatGptCodexUpstreamAdapter {
            model: settings.model.clone(),
        });
    }
    let clean_provider = settings.provider.to_ascii_lowercase().replace([' ', '-'], "_");
    match clean_provider.as_str() {
        "anthropic" | "claude" | "claude_official" | "claude_desktop_official" => {
            Box::new(AnthropicUpstreamAdapter)
        }
        "gemini_native" | "google_gemini_native" => Box::new(GeminiInternalUpstreamAdapter),
        "github_copilot" | "copilot" => Box::new(CopilotUpstreamAdapter),
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
        let body = standard_body;
        Ok((openai_chat_url(settings), headers, body))
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

pub struct ChatGptCodexUpstreamAdapter {
    pub model: String,
}

impl UpstreamAdapter for ChatGptCodexUpstreamAdapter {
    fn prepare_request(
        &self,
        settings: &LLMSettings,
        mut standard_body: Value,
    ) -> Result<(String, HeaderMap, Value), anyhow::Error> {
        let mut headers = bearer_headers(&settings.api_key)?;
        headers.insert("Accept", HeaderValue::from_static("text/event-stream"));
        headers.insert("Oai-Language", HeaderValue::from_static("en-US"));
        headers.insert(
            "Oai-Device-Id",
            HeaderValue::from_static("9f8b4d8d-2900-4b89-a292-8dbefd031923"),
        );
        headers.insert(
            "User-Agent",
            HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"),
        );

        let mut body = json!({
            "store": false,
            "stream": true,
        });

        if let Some(model) = standard_body.get("model") {
            body["model"] = model.clone();
        }

        if let Some(messages) = standard_body.get("messages") {
            body["input"] = messages.clone();
        } else if let Some(prompt) = standard_body.get("prompt") {
            body["input"] = json!([{"role": "user", "content": prompt}]);
        } else {
            body["input"] = json!([]);
        }

        let mut url = openai_chat_url(settings);
        if url.contains("/backend-api/codex/chat/completions") {
            url = url.replace(
                "/backend-api/codex/chat/completions",
                "/backend-api/codex/responses",
            );
        } else if url.ends_with("/backend-api/codex") {
            url = format!("{}/responses", url);
        }

        Ok((url, headers, body))
    }

    fn response_to_standard(&self, body: Value) -> Result<Value, anyhow::Error> {
        Ok(body)
    }

    fn response_stream_to_standard(
        &self,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        use futures::stream::StreamExt;
        let mut buffer = String::new();
        let mut last_text = String::new();
        let model_name_str = self.model.clone();

        let mapped = StreamExt::map(stream, move |res| match res {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                buffer.push_str(&text);
                let mut out_text = String::new();

                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].to_string();
                    buffer.drain(..=newline_pos);
                    let line = line.trim_end_matches('\r').to_string();

                    if line.starts_with("event: ") {
                        continue; // suppress RTK custom events
                    }

                    if line.starts_with("data: ") && line != "data: [DONE]" {
                        let json_str = &line[6..];
                        if let Ok(parsed) = serde_json::from_str::<Value>(json_str) {
                            if let Some(event_type) = parsed.get("type").and_then(|v| v.as_str()) {
                                if event_type == "response.output_text.delta"
                                    || event_type == "response.text.delta"
                                {
                                    if let Some(delta) =
                                        parsed.get("delta").and_then(|v| v.as_str())
                                    {
                                        // Handle cumulative text robustly
                                        let new_text = if delta.starts_with(&last_text) {
                                            let sliced = &delta[last_text.len()..];
                                            last_text = delta.to_string();
                                            sliced
                                        } else {
                                            last_text = delta.to_string();
                                            delta
                                        };

                                        if !new_text.is_empty() {
                                            let chunk = json!({
                                                "id": "chatcmpl-proxy",
                                                "object": "chat.completion.chunk",
                                                "model": model_name_str.clone(),
                                                "choices": [
                                                    {
                                                        "index": 0,
                                                        "delta": {
                                                            "content": new_text
                                                        }
                                                    }
                                                ]
                                            });
                                            out_text.push_str(&format!(
                                                "data: {}\n\n",
                                                chunk.to_string()
                                            ));
                                        }
                                    }
                                    continue;
                                } else if event_type == "response.done"
                                    || event_type == "response.completed"
                                {
                                    out_text.push_str("data: [DONE]\n\n");
                                    continue;
                                }
                            }
                        }
                    }
                    if !line.is_empty() {
                        out_text.push_str(&line);
                        out_text.push('\n');
                    }
                }
                Ok(Bytes::from(out_text))
            }
            Err(e) => Err(e),
        });
        Box::pin(mapped)
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
    let provider = settings.provider.to_ascii_lowercase().replace([' ', '-'], "_");
    let is_google_openai = provider.contains("gemini")
        || provider.contains("google")
        || base.contains("generativelanguage.googleapis.com");

    if is_google_openai {
        if base.ends_with("/openapi") || base.contains("/endpoints/openapi") {
            format!("{base}/chat/completions")
        } else if base.ends_with("/v1beta/openai") || base.ends_with("/v1beta1/openai") {
            format!("{base}/chat/completions")
        } else if base.ends_with("/v1beta") || base.ends_with("/v1beta1") {
            format!("{base}/openai/chat/completions")
        } else if base.contains("/v1beta/") || base.contains("/v1beta1/") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1beta/openai/chat/completions")
        }
    } else if base.ends_with("/backend-api/codex") {
        format!("{base}/responses")
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

        // Google Vertex AI OpenAPI 端点测试
        let vertex_settings = LLMSettings {
            provider: "Gemini Native".to_string(),
            base_url: "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/my-proj/locations/us-central1/endpoints/openapi".to_string(),
            api_key: "ya29.test".to_string(),
            ..LLMSettings::default()
        };
        assert_eq!(
            openai_chat_url(&vertex_settings),
            "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/my-proj/locations/us-central1/endpoints/openapi/chat/completions"
        );
    }
}
