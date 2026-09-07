use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domains::model_gateway::providers::LlmProvider;

use crate::services::visual_pipeline::types::{
    CaptureProviderOutput, MonitorDescriptor, OcrBlock, OcrProvider, OcrProviderOutput,
    ProviderDescriptor, ProviderKind, ScreenshotProvider, TranslationInput, TranslationProvider,
    TranslationProviderOutput, VisualSettings,
};


use super::*;
use super::common::*;

pub struct HubLlmTranslationProvider {
    client: reqwest::Client,
}

impl HubLlmTranslationProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TranslationProvider for HubLlmTranslationProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        let ready = LlmProvider::new(self.client.clone()).is_available();
        descriptor(
            "hub-llm",
            ProviderKind::Translation,
            "Hub 模型网关（Gemini / GPT 等）",
            env!("CARGO_PKG_VERSION"),
            ready,
            (!ready).then(|| "请先在模型管理中配置可用账户".to_string()),
            vec!["Hub model account".to_string()],
            json!({ "models": ["gemini", "gpt", "openai-compatible"], "network": true }),
        )
    }

    async fn translate(
        &self,
        input: &TranslationInput,
        _settings: &VisualSettings,
    ) -> Result<TranslationProviderOutput, String> {
        let source = input
            .source_language
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("auto-detect");
        let message = json!({
            "role": "user",
            "content": format!(
                "Translate the following text from {source} to {}.\n\n{}",
                input.target_language,
                input.text
            )
        });

        let system_prompt = input.system_prompt.clone().unwrap_or_else(|| {
            "You are a pure translation engine. You MUST output ONLY the translated text without any explanation, markdown formatting, or surrounding quotes. Preserve the original line breaks.".to_string()
        });

        let request_body = json!({
            "model": input.model.clone().unwrap_or_else(|| "gpt-4o".to_string()),
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt
                },
                message
            ],
            "temperature": 0.0
        });

        let proxy_url = "http://127.0.0.1:3001/v1/chat/completions";
        let response = self.client.post(proxy_url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Failed to call internal proxy: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(format!("Proxy error {}: {}", status, err_text));
        }

        let resp_json: serde_json::Value = response.json().await
            .map_err(|e| format!("Failed to parse proxy response: {}", e))?;

        let mut translated_text = resp_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
            
        // Clean up markdown block if the model ignores the instruction
        if translated_text.starts_with("```") {
            if let Some(start_idx) = translated_text.find('\n') {
                translated_text = translated_text[start_idx..].trim_start().to_string();
            }
            if translated_text.ends_with("```") {
                translated_text = translated_text[..translated_text.len()-3].trim_end().to_string();
            }
        }
        
        Ok(TranslationProviderOutput {
            translated_text,
            detected_source_language: None,
        })
    }
}

pub struct DeepLTranslationProvider {
    client: reqwest::Client,
}

impl DeepLTranslationProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TranslationProvider for DeepLTranslationProvider {
    fn descriptor(&self, settings: &VisualSettings) -> ProviderDescriptor {
        let ready = !settings.deepl.api_key.is_empty();
        descriptor(
            "deepl",
            ProviderKind::Translation,
            "DeepL API",
            "v2",
            ready,
            (!ready).then(|| "未配置 DeepL 密钥 (deepl.apiKey)".to_string()),
            vec!["settings.deepl.apiKey".to_string()],
            json!({ "network": true, "sourceLanguageAutoDetect": true }),
        )
    }

    async fn translate(
        &self,
        input: &TranslationInput,
        settings: &VisualSettings,
    ) -> Result<TranslationProviderOutput, String> {
        let key = &settings.deepl.api_key;
        if key.is_empty() { return Err("未配置 DeepL 密钥".into()); }
        
        let endpoint = if !settings.deepl.custom_url.is_empty() {
            settings.deepl.custom_url.clone()
        } else if key.ends_with(":fx") {
            "https://api-free.deepl.com/v2/translate".into()
        } else {
            "https://api.deepl.com/v2/translate".into()
        };
        let mut body = json!({
            "text": [input.text],
            "target_lang": normalize_language(&input.target_language, true)
        });
        if let Some(source) = input
            .source_language
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            body["source_lang"] = json!(normalize_language(source, true));
        }
        let response = self
            .client
            .post(endpoint)
            .header("Authorization", format!("DeepL-Auth-Key {key}"))
            .json(&body)
            .timeout(std::time::Duration::from_secs(45))
            .send()
            .await
            .map_err(|error| format!("DeepL request failed: {error}"))?;
        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| format!("DeepL returned invalid JSON: {error}"))?;
        if !status.is_success() {
            return Err(format!("DeepL returned HTTP {status}"));
        }
        let translation = payload
            .get("translations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| "DeepL response has no translation".to_string())?;
        Ok(TranslationProviderOutput {
            translated_text: translation
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            detected_source_language: translation
                .get("detected_source_language")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

pub struct GoogleTranslationProvider {
    client: reqwest::Client,
}

impl GoogleTranslationProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TranslationProvider for GoogleTranslationProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        let ready = std::env::var("GOOGLE_TRANSLATE_API_KEY")
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        descriptor(
            "google-translate",
            ProviderKind::Translation,
            "Google Cloud Translation",
            "v2",
            ready,
            (!ready).then(|| "缺少环境变量 GOOGLE_TRANSLATE_API_KEY".to_string()),
            vec!["GOOGLE_TRANSLATE_API_KEY".to_string()],
            json!({ "network": true, "sourceLanguageAutoDetect": true }),
        )
    }

    async fn translate(
        &self,
        input: &TranslationInput,
        _settings: &VisualSettings,
    ) -> Result<TranslationProviderOutput, String> {
        let key = std::env::var("GOOGLE_TRANSLATE_API_KEY")
            .map_err(|_| "GOOGLE_TRANSLATE_API_KEY is not configured".to_string())?;
        let endpoint = std::env::var("GOOGLE_TRANSLATE_URL")
            .unwrap_or_else(|_| "https://translation.googleapis.com/language/translate/v2".into());
        let mut body = json!({
            "q": input.text,
            "target": normalize_language(&input.target_language, false),
            "format": "text"
        });
        if let Some(source) = input
            .source_language
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            body["source"] = json!(normalize_language(source, false));
        }
        let response = self
            .client
            .post(endpoint)
            .header("X-Goog-Api-Key", key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(45))
            .send()
            .await
            .map_err(|error| format!("Google Translation request failed: {error}"))?;
        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| format!("Google Translation returned invalid JSON: {error}"))?;
        if !status.is_success() {
            return Err(format!("Google Translation returned HTTP {status}"));
        }
        let translation = payload
            .pointer("/data/translations/0")
            .ok_or_else(|| "Google Translation response has no translation".to_string())?;
        Ok(TranslationProviderOutput {
            translated_text: translation
                .get("translatedText")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            detected_source_language: translation
                .get("detectedSourceLanguage")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

pub(crate) fn normalize_language(language: &str, uppercase: bool) -> String {
    let normalized = language.trim().replace('_', "-");
    if uppercase {
        normalized.to_ascii_uppercase()
    } else {
        normalized.to_ascii_lowercase()
    }
}


