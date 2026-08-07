use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::app_paths::AppPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LLMSettings {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f64,
    pub max_tokens: u32,
    pub ua_file_limit: u32,
    pub ua_sample_file_limit: u32,
    pub ua_max_file_chars: u32,
    pub updated_at: String,
}

impl Default for LLMSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "anthropic".into(),
            base_url: "http://127.0.0.1:8045".into(),
            api_key: std::env::var("DEEPCLOUD_API_KEY").unwrap_or_default(),
            model: "gemini-3-flash".into(),
            temperature: 0.2,
            max_tokens: 16000,
            ua_file_limit: 48,
            ua_sample_file_limit: 12,
            ua_max_file_chars: 60000,
            updated_at: "1970-01-01T00:00:00.000Z".into(),
        }
    }
}

pub fn settings_path() -> PathBuf {
    let app_paths = AppPaths::new();
    app_paths.data_path(&["settings", "llm.json"])
}

pub fn read_llm_settings() -> LLMSettings {
    let path = settings_path();
    if !path.exists() {
        return LLMSettings::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            // Editors/PowerShell may save the file with a UTF-8 BOM, which
            // serde_json rejects; strip it before parsing.
            let content = content.trim_start_matches('\u{feff}');
            serde_json::from_str(content).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse LLM settings: {}, using defaults", e);
                LLMSettings::default()
            })
        }
        Err(e) => {
            tracing::warn!("Failed to read LLM settings: {}, using defaults", e);
            LLMSettings::default()
        }
    }
}

/// Ensure the settings file exists on disk so configuration is always
/// inspectable and survives restarts. Called once at startup.
pub fn ensure_persisted() -> PathBuf {
    let path = settings_path();
    if !path.exists() {
        let app_paths = AppPaths::new();
        app_paths.ensure_data_dir(&["settings"]);
        let mut settings = LLMSettings::default();
        settings.updated_at = chrono::Utc::now().to_rfc3339();
        if let Ok(json) = serde_json::to_string_pretty(&settings) {
            match std::fs::write(&path, json) {
                Ok(_) => tracing::info!("LLM settings initialized at {}", path.display()),
                Err(e) => tracing::warn!("Failed to persist default LLM settings: {}", e),
            }
        }
    }
    path
}

pub fn write_llm_settings(partial: serde_json::Value) -> LLMSettings {
    let app_paths = AppPaths::new();
    app_paths.ensure_data_dir(&["settings"]);

    let current = read_llm_settings();
    let current_json = serde_json::to_value(&current).unwrap_or_default();

    // Merge partial into current
    let merged = json_merge(current_json, partial);
    let mut settings: LLMSettings = serde_json::from_value(merged).unwrap_or_default();
    settings.updated_at = chrono::Utc::now().to_rfc3339();

    // Normalize
    settings.temperature = settings.temperature.clamp(0.0, 2.0);
    settings.max_tokens = settings.max_tokens.clamp(256, 64000);
    settings.ua_file_limit = settings.ua_file_limit.clamp(1, 200);
    settings.ua_sample_file_limit = settings.ua_sample_file_limit.clamp(1, 50);
    settings.ua_max_file_chars = settings.ua_max_file_chars.clamp(1000, 200000);
    settings.base_url = settings.base_url.trim_end_matches('/').to_string();

    let path = settings_path();
    if let Ok(json) = serde_json::to_string_pretty(&settings) {
        std::fs::write(&path, json).ok();
    }
    settings
}

pub fn has_usable_settings(settings: &LLMSettings) -> bool {
    settings.enabled
        && (settings.provider == "openai-compatible" || settings.provider == "anthropic")
        && !settings.base_url.is_empty()
        && !settings.api_key.is_empty()
        && !settings.model.is_empty()
}

fn json_merge(base: serde_json::Value, overlay: serde_json::Value) -> serde_json::Value {
    match (base, overlay) {
        (serde_json::Value::Object(mut base_map), serde_json::Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                if value.is_null() {
                    continue;
                }
                let entry = base_map.entry(key).or_insert(serde_json::Value::Null);
                *entry = json_merge(entry.take(), value);
            }
            serde_json::Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}
