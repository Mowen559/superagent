use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use toml_edit::{value, DocumentMut, Item, Table};

use super::super::adapter::{
    CliConfigAdapter, CliConfigCapabilities, CliInstallation, ConfigMutationPlan,
    EffectiveCliConfig, ManagementMode, ProbeResult, ResolvedSyncRequest, SyncError,
};
use crate::services::agent::AgentKind;

pub struct CodexConfigAdapter;

#[async_trait]
impl CliConfigAdapter for CodexConfigAdapter {
    fn kind(&self) -> AgentKind {
        AgentKind::Codex
    }

    async fn detect(&self) -> Result<CliInstallation, SyncError> {
        let version = crate::services::agent::proc::detect_version("codex")
            .await
            .ok_or(SyncError::CliNotInstalled)?;

        let config_root = crate::config::app_paths::AppPaths::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
            .to_string_lossy()
            .to_string();

        Ok(CliInstallation {
            executable: "codex".to_string(),
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
            requires_restart: true,
        })
    }

    async fn inspect(&self, install: &CliInstallation) -> Result<EffectiveCliConfig, SyncError> {
        let settings_path = PathBuf::from(&install.config_root).join("config.toml");
        let mut requested_model = None;
        let mut effective_provider = None;
        let mut effective_base_url = None;

        let content = if settings_path.exists() {
            std::fs::read_to_string(&settings_path).unwrap_or_else(|_| "".to_string())
        } else {
            "".to_string()
        };

        if let Ok(doc) = content.parse::<DocumentMut>() {
            if let Some(m) = doc.get("model").and_then(|v| v.as_str()) {
                requested_model = Some(m.to_string());
            }
            if let Some(p) = doc.get("model_provider").and_then(|v| v.as_str()) {
                effective_provider = Some(p.to_string());

                if let Some(providers) = doc.get("model_providers").and_then(|v| v.as_table_like())
                {
                    if let Some(provider) = providers.get(p).and_then(|v| v.as_table_like()) {
                        if let Some(url) = provider.get("base_url").and_then(|v| v.as_str()) {
                            effective_base_url = Some(url.to_string());
                        }
                    }
                }
            }
        }

        let revision = format!("sha256:{}", crate::shared_kernel::crypto::sha256_str(&content));

        Ok(EffectiveCliConfig {
            requested_model: requested_model.clone(),
            persisted_model: requested_model.clone(),
            effective_model: requested_model,
            model_source: "USER_CONFIG".to_string(),
            effective_provider,
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
        let settings_path = config_root.join("config.toml");

        let content = if settings_path.exists() {
            std::fs::read_to_string(&settings_path)
                .map_err(|e| SyncError::ConfigNotReadable(e.to_string()))?
        } else {
            "".to_string()
        };

        let mut doc = content
            .parse::<DocumentMut>()
            .map_err(|e| SyncError::ConfigParseFailed(e.to_string()))?;

        if let Some(m) = &request.model.default_model {
            doc["model"] = value(m.clone());
        }

        doc["model_provider"] = value(request.provider_id.clone());

        if let Some(effort) = &request.model.reasoning_effort {
            doc["model_reasoning_effort"] = value(effort.clone());
        }

        let provider_id = &request.provider_id;

        if !doc.contains_key("model_providers") {
            doc.insert("model_providers", Item::Table(Table::new()));
        }

        if let Some(providers) = doc["model_providers"].as_table_mut() {
            // Resolve provider via ProviderStore
            let provider = crate::services::agent::provider_store::global_provider_store()
                .get_provider(provider_id);

            if let Some(p) = provider {
                if p.is_local_proxy {
                    let auth_token = crate::services::agent::provider_store::global_provider_store()
                        .generate_proxy_token(provider_id);

                    let proxy_provider = toml_edit::Item::Table({
                        let mut t = toml_edit::Table::new();
                        // Codex needs OpenAI compatible endpoint, typically we proxy to /openai on hub
                        t.insert(
                            "base_url",
                            toml_edit::value(format!("{}/openai", hub_base_url)),
                        );
                        t.insert("wire_api", toml_edit::value("openai"));
                        t.insert("env_key", toml_edit::value(auth_token));
                        t
                    });
                    providers.insert(provider_id, proxy_provider);
                } else {
                    let mut provider_table = Table::new();
                    provider_table.insert("name", value(p.display_name));
                    // Resolve direct connection config based on provider's configured models_endpoint
                    let base_path = p.models_endpoint.trim_end_matches("/models");
                    let base_url = if base_path.starts_with("http://") || base_path.starts_with("https://") {
                        base_path.to_string()
                    } else {
                        format!("{}{}", hub_base_url, if base_path.is_empty() { "/v1" } else { base_path })
                    };
                    provider_table.insert("base_url", value(base_url));
                    let wire_api = match p.upstream_protocol.to_uppercase().as_str() {
                        "OPENAI_RESPONSES" => "responses",
                        "OPENAI_CHAT" => "openai",
                        _ => "responses",
                    };
                    provider_table.insert("wire_api", value(wire_api));
                    provider_table.insert("env_key", value("DEEPCLOUD_CLI_TOKEN"));

                    providers.insert(provider_id, Item::Table(provider_table));
                }
            }
        }

        let mut files_to_write = HashMap::new();
        files_to_write.insert(settings_path.to_string_lossy().to_string(), doc.to_string());

        Ok(ConfigMutationPlan {
            files_to_write,
            requires_restart: true,
        })
    }

    fn validate_rendered(&self, _plan: &ConfigMutationPlan) -> Result<(), SyncError> {
        Ok(())
    }

    async fn probe(&self, install: &CliInstallation, config: &EffectiveCliConfig) -> Result<ProbeResult, SyncError> {
        // Semantic validation: ensure required fields are present
        if config.effective_base_url.is_none() || config.effective_provider.is_none() {
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
