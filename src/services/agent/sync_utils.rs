//! Utilities for updating local agent configurations.

use std::fs;
use std::path::Path;
use serde_json::Value;

use super::backend::{BackendError, BackendResult};
use super::types::SyncConfigPayload;

/// Helper to backup a file before modifying it.
fn backup_file(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        let mut backup_path = path.to_path_buf();
        let ext = backup_path.extension().unwrap_or_default().to_string_lossy();
        backup_path.set_extension(format!("{}.antigravity.bak", ext));
        fs::copy(path, backup_path)?;
    }
    Ok(())
}

/// Helper to ensure the directory for a file exists.
fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// Helper to sync Claude-like agent configurations.
/// It creates/updates the onboarding file and the `settings.json` file.
pub fn sync_claude_like(
    config: &SyncConfigPayload,
    global_dir: &Path,
    onboarding_file: &str,
) -> BackendResult<()> {
    ensure_parent(&global_dir.join("dummy"))
        .map_err(|e| BackendError::Launch(format!("failed to create dir: {}", e)))?;

    let onboard_path = global_dir.join(onboarding_file);
    if !onboard_path.exists() {
        fs::write(&onboard_path, b"")
            .map_err(|e| BackendError::Launch(format!("failed to write onboarding file: {}", e)))?;
    }

    let settings_path = global_dir.join("settings.json");
    let mut settings: Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path)
            .map_err(|e| BackendError::Launch(format!("failed to read settings.json: {}", e)))?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(obj) = settings.as_object_mut() {
        let env = obj.entry("env").or_insert(serde_json::json!({}));
        if let Some(env_obj) = env.as_object_mut() {
            if let Some(base_url) = &config.base_url {
                env_obj.insert("CLAUDE_BASE_URL".to_string(), serde_json::Value::String(base_url.clone()));
            }
            if let Some(api_key) = &config.api_key {
                env_obj.insert("CLAUDE_API_KEY".to_string(), serde_json::Value::String(api_key.clone()));
            }
            if let Some(model) = &config.model {
                env_obj.insert("CLAUDE_MODEL".to_string(), serde_json::Value::String(model.clone()));
            }
        }
    }

    backup_file(&settings_path)
        .map_err(|e| BackendError::Launch(format!("failed to backup settings.json: {}", e)))?;
    let new_content = serde_json::to_string_pretty(&settings)
        .map_err(|e| BackendError::Launch(format!("failed to serialize settings.json: {}", e)))?;
    
    fs::write(&settings_path, new_content)
        .map_err(|e| BackendError::Launch(format!("failed to write settings.json: {}", e)))?;

    Ok(())
}

/// Helper to sync OpenCode-like configurations.
pub fn sync_opencode(
    config: &SyncConfigPayload,
    global_dir: &Path,
) -> BackendResult<()> {
    ensure_parent(&global_dir.join("dummy"))
        .map_err(|e| BackendError::Launch(format!("failed to create dir: {}", e)))?;

    let config_path = global_dir.join("config.json");
    let mut settings: Value = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .map_err(|e| BackendError::Launch(format!("failed to read config.json: {}", e)))?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(obj) = settings.as_object_mut() {
        let providers = obj.entry("providers").or_insert(serde_json::json!({}));
        if let Some(prov_obj) = providers.as_object_mut() {
            let openai = prov_obj.entry("openai").or_insert(serde_json::json!({}));
            if let Some(openai_obj) = openai.as_object_mut() {
                if let Some(base_url) = &config.base_url {
                    openai_obj.insert("apiBase".to_string(), serde_json::Value::String(base_url.clone()));
                }
                if let Some(api_key) = &config.api_key {
                    openai_obj.insert("apiKey".to_string(), serde_json::Value::String(api_key.clone()));
                }
            }
        }
    }

    backup_file(&config_path)
        .map_err(|e| BackendError::Launch(format!("failed to backup config.json: {}", e)))?;
    let new_content = serde_json::to_string_pretty(&settings)
        .map_err(|e| BackendError::Launch(format!("failed to serialize config.json: {}", e)))?;
    
    fs::write(&config_path, new_content)
        .map_err(|e| BackendError::Launch(format!("failed to write config.json: {}", e)))?;

    Ok(())
}

/// Helper to sync Codex-like configurations.
pub fn sync_codex(
    config: &SyncConfigPayload,
    global_dir: &Path,
) -> BackendResult<()> {
    ensure_parent(&global_dir.join("dummy"))
        .map_err(|e| BackendError::Launch(format!("failed to create dir: {}", e)))?;

    let auth_path = global_dir.join("auth.json");
    let mut auth: Value = if auth_path.exists() {
        let content = fs::read_to_string(&auth_path)
            .map_err(|e| BackendError::Launch(format!("failed to read auth.json: {}", e)))?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(obj) = auth.as_object_mut() {
        let custom = obj.entry("custom").or_insert(serde_json::json!({}));
        if let Some(custom_obj) = custom.as_object_mut() {
            if let Some(api_key) = &config.api_key {
                custom_obj.insert("apiKey".to_string(), serde_json::Value::String(api_key.clone()));
            }
        }
    }

    backup_file(&auth_path)
        .map_err(|e| BackendError::Launch(format!("failed to backup auth.json: {}", e)))?;
    let new_auth_content = serde_json::to_string_pretty(&auth)
        .map_err(|e| BackendError::Launch(format!("failed to serialize auth.json: {}", e)))?;
    
    fs::write(&auth_path, new_auth_content)
        .map_err(|e| BackendError::Launch(format!("failed to write auth.json: {}", e)))?;

    let config_path = global_dir.join("config.toml");
    let mut doc = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .map_err(|e| BackendError::Launch(format!("failed to read config.toml: {}", e)))?;
        content.parse::<toml_edit::DocumentMut>().unwrap_or_default()
    } else {
        toml_edit::DocumentMut::new()
    };

    if config.base_url.is_some() || config.model.is_some() {
        if !doc.contains_key("model_providers") {
            doc["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        if let Some(providers) = doc["model_providers"].as_table_mut() {
            if !providers.contains_key("custom") {
                let mut custom_table = toml_edit::Table::new();
                custom_table.set_implicit(true);
                providers.insert("custom", toml_edit::Item::Table(custom_table));
            }
        }

        if let Some(base_url) = &config.base_url {
            doc["model_providers"]["custom"]["apiBase"] = toml_edit::value(base_url.clone());
        }
        if let Some(model) = &config.model {
            doc["model_providers"]["custom"]["model"] = toml_edit::value(model.clone());
        }
    }

    backup_file(&config_path)
        .map_err(|e| BackendError::Launch(format!("failed to backup config.toml: {}", e)))?;
    fs::write(&config_path, doc.to_string())
        .map_err(|e| BackendError::Launch(format!("failed to write config.toml: {}", e)))?;

    Ok(())
}
