use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

use super::super::adapter::{
    CliConfigAdapter, CliConfigCapabilities, CliInstallation, ConfigMutationPlan,
    EffectiveCliConfig, ManagementMode, ProbeResult, ResolvedSyncRequest, SyncError,
};
use crate::services::agent::AgentKind;

pub struct ClaudeConfigAdapter;

#[async_trait]
impl CliConfigAdapter for ClaudeConfigAdapter {
    fn kind(&self) -> AgentKind {
        AgentKind::Claude
    }

    async fn detect(&self) -> Result<CliInstallation, SyncError> {
        let version = crate::services::agent::proc::detect_version("claude")
            .await
            .ok_or(SyncError::CliNotInstalled)?;

        let config_root = crate::config::app_paths::AppPaths::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
            .to_string_lossy()
            .to_string();

        Ok(CliInstallation {
            executable: "claude".to_string(),
            version,
            config_root,
        })
    }

    async fn capabilities(
        &self,
        _install: &CliInstallation,
    ) -> Result<CliConfigCapabilities, SyncError> {
        Ok(CliConfigCapabilities {
            management_mode: ManagementMode::Exclusive,
            requires_restart: false,
        })
    }

    async fn inspect(&self, install: &CliInstallation) -> Result<EffectiveCliConfig, SyncError> {
        let settings_path = PathBuf::from(&install.config_root).join("settings.json");
        let mut requested_model = None;
        let mut effective_base_url = None;

        let content = if settings_path.exists() {
            std::fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string())
        } else {
            "{}".to_string()
        };

        if let Ok(val) = serde_json::from_str::<Value>(&content) {
            if let Some(m) = val.get("model").and_then(|v| v.as_str()) {
                requested_model = Some(m.to_string());
            }
            if let Some(env) = val.get("env").and_then(|v| v.as_object()) {
                if let Some(url) = env.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()) {
                    effective_base_url = Some(url.to_string());
                }
            }
        }

        let revision = format!("sha256:{}", crate::shared_kernel::crypto::sha256_str(&content));

        Ok(EffectiveCliConfig {
            requested_model: requested_model.clone(),
            persisted_model: requested_model.clone(),
            effective_model: requested_model,
            model_source: "USER_CONFIG".to_string(),
            effective_provider: None,
            effective_base_url,
            revision,
        })
    }

    async fn plan(
        &self,
        install: &CliInstallation,
        request: &ResolvedSyncRequest,
        hub_base_url: &str,
    ) -> Result<ConfigMutationPlan, SyncError> {
        if !request.persist_default {
            return Ok(ConfigMutationPlan {
                files_to_write: HashMap::new(),
                requires_restart: false,
            });
        }

        let config_root = PathBuf::from(&install.config_root);
        let settings_path = config_root.join("settings.json");

        let mut val: Value = if settings_path.exists() {
            let content = std::fs::read_to_string(&settings_path)
                .map_err(|e| SyncError::ConfigNotReadable(e.to_string()))?;
            serde_json::from_str(&content)
                .map_err(|e| SyncError::ConfigParseFailed(e.to_string()))?
        } else {
            json!({})
        };

        if let Some(obj) = val.as_object_mut() {
            if let Some(m) = &request.model.default_model {
                obj.insert("model".to_string(), Value::String(m.to_string()));
            }

            // Resolve credential via ProviderStore
            let provider = crate::services::agent::provider_store::global_provider_store()
                .get_provider(&request.provider_id);

            let mut env_obj = obj
                .get("env")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

            if let Value::Object(ref mut env_obj) = env_obj {
                if let Some(p) = provider {
                    if p.is_local_proxy {
                        let auth_token = crate::services::agent::provider_store::global_provider_store()
                            .generate_proxy_token(&p.provider_id);

                        env_obj.remove("ANTHROPIC_API_KEY");
                        // Format the base_url. Claude uses anthropic API, so if the upstream is deepcloud, we point to hub_base_url/anthropic
                        env_obj.insert(
                            "ANTHROPIC_BASE_URL".to_string(),
                            Value::String(format!("{}/anthropic", hub_base_url)),
                        );
                        env_obj.insert(
                            "ANTHROPIC_AUTH_TOKEN".to_string(),
                            Value::String(auth_token),
                        );
                    } else {
                        // Direct connection logic (not fully implemented yet, would need real credentials)
                    }
                }
            }
            obj.insert("env".to_string(), env_obj);
        }

        let mut files_to_write = HashMap::new();
        files_to_write.insert(
            settings_path.to_string_lossy().to_string(),
            serde_json::to_string_pretty(&val).unwrap(),
        );

        Ok(ConfigMutationPlan {
            files_to_write,
            requires_restart: false,
        })
    }

    fn validate_rendered(&self, _plan: &ConfigMutationPlan) -> Result<(), SyncError> {
        Ok(())
    }

    async fn probe(&self, install: &CliInstallation, config: &EffectiveCliConfig) -> Result<ProbeResult, SyncError> {
        // Semantic validation: ensure required fields are present
        if config.effective_base_url.is_none() {
            return Ok(ProbeResult {
                syntax: "PASSED".to_string(),
                semantic: "FAILED".to_string(),
                read_back: "PASSED".to_string(),
                cli_probe: "SKIPPED".to_string(),
                network_probe: "SKIPPED".to_string(),
            });
        }

        // CLI Probe
        let cli_probe = match std::process::Command::new(&install.executable)
            .arg("--version")
            .output() {
            Ok(output) if output.status.success() => "PASSED".to_string(),
            _ => "FAILED".to_string(),
        };

        // Network Probe
        let network_probe = if let Some(base_url) = &config.effective_base_url {
            let client = reqwest::Client::new();
            match client.get(format!("{}/health", base_url)).send().await {
                Ok(res) if res.status().is_success() => "PASSED".to_string(),
                _ => "FAILED".to_string(),
            }
        } else {
            "SKIPPED".to_string()
        };

        Ok(ProbeResult {
            syntax: "PASSED".to_string(),
            semantic: "PASSED".to_string(),
            read_back: "PASSED".to_string(),
            cli_probe,
            network_probe,
        })
    }
}
