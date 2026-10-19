use super::app_paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

fn default_true() -> bool {
    true
}

fn default_weight() -> f32 {
    1.0
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ModelCategory {
    Chat,
    Embedding,
    Vision,
    Audio,
    Multimodal,
    Reasoning,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    pub category: ModelCategory,
    pub context_length: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub supports_function_calling: bool,
    pub supports_vision: bool,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            category: ModelCategory::Unknown,
            context_length: None,
            max_output_tokens: None,
            supports_function_calling: false,
            supports_vision: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LLMAccount {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub provider: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub models_info: Option<std::collections::HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub model_mapping: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub model_capabilities: Option<std::collections::HashMap<String, ModelCapabilities>>,
    pub base_url: String,
    pub api_key: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub project_id: Option<String>,
    #[serde(default)]
    pub quota_info: Option<serde_json::Value>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_weight")]
    pub weight: f32,
}

impl Default for LLMAccount {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            provider: String::new(),
            models: vec![],
            models_info: None,
            model_mapping: std::collections::HashMap::new(),
            model_capabilities: None,
            base_url: String::new(),
            api_key: String::new(),
            refresh_token: None,
            expires_at: None,
            project_id: None,
            quota_info: None,
            tags: vec![],
            priority: 0,
            weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LLMSettings {
    pub enabled: bool,
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<LLMAccount>,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub model: String,
    #[serde(default)]
    pub fast_model: Option<String>,
    pub temperature: f64,
    pub max_tokens: u32,
    pub ua_file_limit: u32,
    pub ua_sample_file_limit: u32,
    pub ua_max_file_chars: u32,
    pub updated_at: String,
    #[serde(default = "default_routing_strategy")]
    pub routing_strategy: String,
}

impl LLMSettings {
    /// Resolves the effective model name for Fast Brain operations
    pub fn resolved_fast_model(&self) -> String {
        if let Ok(env_model) = std::env::var("SUPERAGENT_FAST_MODEL") {
            if !env_model.trim().is_empty() {
                return env_model;
            }
        }
        if let Some(ref fast) = self.fast_model {
            if !fast.trim().is_empty() {
                return fast.clone();
            }
        }
        let lower = self.model.to_ascii_lowercase();
        if lower.contains("r1") {
            "deepseek-chat".to_string()
        } else if lower.contains("sonnet") || lower.contains("opus") {
            "claude-3-5-haiku".to_string()
        } else if lower.contains("pro") {
            "gemini-2.5-flash".to_string()
        } else if lower.contains("o1") || lower.contains("o3") {
            "gpt-4o-mini".to_string()
        } else {
            self.model.clone()
        }
    }
}

fn default_routing_strategy() -> String {
    "failover".to_string()
}

impl Default for LLMSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            active_account_id: None,
            accounts: vec![],
            provider: "anthropic".into(),
            base_url: "http://127.0.0.1:8045".into(),
            api_key: std::env::var("DEEPCLOUD_API_KEY").unwrap_or_default(),
            project_id: None,
            model: "claude-sonnet-4-6".into(),
            fast_model: None,
            temperature: 0.2,
            max_tokens: 16000,
            ua_file_limit: 48,
            ua_sample_file_limit: 12,
            ua_max_file_chars: 60000,
            updated_at: "1970-01-01T00:00:00.000Z".into(),
            routing_strategy: "failover".to_string(),
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
    match update_llm_settings(move |settings| {
        let current_json = serde_json::to_value(&*settings)
            .map_err(|error| format!("failed to serialize current LLM settings: {error}"))?;
        let merged = json_merge(current_json, partial);
        *settings = serde_json::from_value(merged)
            .map_err(|error| format!("invalid LLM settings update: {error}"))?;
        Ok(())
    }) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!("Failed to update LLM settings: {error}");
            read_llm_settings()
        }
    }
}

/// Apply one read-modify-write transaction under the settings lock. The
/// closure never receives a filesystem path, so account credentials and the
/// atomic checkpoint implementation stay local to this Module.
pub fn update_llm_settings<F>(update: F) -> Result<LLMSettings, String>
where
    F: FnOnce(&mut LLMSettings) -> Result<(), String>,
{
    let app_paths = AppPaths::new();
    app_paths.ensure_data_dir(&["settings"]);
    let path = settings_path();
    let lock_path = path.with_extension("json.lock");
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|error| format!("failed to open {}: {error}", lock_path.display()))?;
    let mut lock = fd_lock::RwLock::new(lock_file);
    let _guard = lock
        .write()
        .map_err(|error| format!("failed to lock {}: {error}", lock_path.display()))?;

    let mut settings = read_settings_file(&path).unwrap_or_default();
    update(&mut settings)?;
    normalize_settings(&mut settings);
    write_settings_checkpoint(&path, &settings)?;
    Ok(settings)
}

fn normalize_settings(settings: &mut LLMSettings) {
    if let Some(active_id) = &settings.active_account_id {
        if let Some(account) = settings
            .accounts
            .iter()
            .find(|account| account.id == *active_id)
        {
            settings.provider = account.provider.clone();
            settings.base_url = account.base_url.clone();
            settings.api_key = account.api_key.clone();
        }
    }

    settings.updated_at = chrono::Utc::now().to_rfc3339();
    settings.temperature = settings.temperature.clamp(0.0, 2.0);
    settings.max_tokens = settings.max_tokens.clamp(256, 64000);
    settings.ua_file_limit = settings.ua_file_limit.clamp(1, 200);
    settings.ua_sample_file_limit = settings.ua_sample_file_limit.clamp(1, 50);
    settings.ua_max_file_chars = settings.ua_max_file_chars.clamp(1000, 200000);
    settings.base_url = settings.base_url.trim_end_matches('/').to_string();
    for account in &mut settings.accounts {
        account.base_url = account.base_url.trim_end_matches('/').to_string();
    }
}

fn read_settings_file(path: &Path) -> Option<LLMSettings> {
    std::fs::read_to_string(path).ok().and_then(|content| {
        serde_json::from_str(content.trim_start_matches('\u{feff}'))
            .map_err(|error| {
                tracing::warn!("Failed to parse LLM settings: {error}, using defaults");
                error
            })
            .ok()
    })
}

fn write_settings_checkpoint(path: &Path, settings: &LLMSettings) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("failed to create LLM settings checkpoint: {error}"))?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), settings)
        .map_err(|error| format!("failed to serialize LLM settings checkpoint: {error}"))?;
    temporary
        .as_file_mut()
        .flush()
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("failed to flush LLM settings checkpoint: {error}"))?;
    temporary.persist(path).map_err(|error| {
        format!(
            "failed to atomically replace {}: {}",
            path.display(),
            error.error
        )
    })?;
    Ok(())
}

pub fn has_usable_settings(settings: &LLMSettings) -> bool {
    settings.enabled
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

pub fn read_allowed_models() -> Option<Vec<String>> {
    let app_paths = AppPaths::new();
    let mut path = app_paths.data_path(&["settings"]);
    path.push("allowed_models.json");

    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(models) = serde_json::from_str::<Vec<String>>(&content) {
            return Some(models);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_checkpoint_replaces_an_existing_file_without_partial_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("llm.json");
        std::fs::write(&path, r#"{"model":"old"}"#).unwrap();
        let expected = LLMSettings {
            model: "new-model".to_string(),
            api_key: "secret".to_string(),
            ..LLMSettings::default()
        };

        write_settings_checkpoint(&path, &expected).unwrap();
        let actual: LLMSettings =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(actual.model, "new-model");
        assert_eq!(actual.api_key, "secret");
    }
}
