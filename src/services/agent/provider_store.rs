use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for an upstream provider that Hub routes to or manages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub provider_id: String,
    pub display_name: String,
    pub upstream_protocol: String,
    pub models_endpoint: String,
    /// Whether this provider should be routed through Hub's local proxy.
    pub is_local_proxy: bool,
}

pub struct ProviderStore {
    providers: HashMap<String, ProviderConfig>,
}

impl ProviderStore {
    pub fn new() -> Self {
        let mut providers = HashMap::new();
        // A single deepcloud-local provider mapped to the hub's proxy
        providers.insert(
            "deepcloud-local".to_string(),
            ProviderConfig {
                provider_id: "deepcloud-local".to_string(),
                display_name: "DeepCloud Local Gateway".to_string(),
                upstream_protocol: "OPENAI_RESPONSES".to_string(),
                models_endpoint: "/v1/models".to_string(),
                is_local_proxy: true,
            },
        );
        // Also support "deepcloud" legacy reference just in case
        providers.insert(
            "deepcloud".to_string(),
            ProviderConfig {
                provider_id: "deepcloud".to_string(),
                display_name: "DeepCloud Gateway (Legacy)".to_string(),
                upstream_protocol: "OPENAI_RESPONSES".to_string(),
                models_endpoint: "/v1/models".to_string(),
                is_local_proxy: true,
            },
        );
        Self { providers }
    }

    pub fn get_provider(&self, provider_id: &str) -> Option<ProviderConfig> {
        self.providers.get(provider_id).cloned()
    }

    pub fn generate_proxy_token(&self, _provider_id: &str) -> String {
        // In a full implementation, this might generate a secure random UUID
        // and store it in an active session cache for the proxy to validate.
        // For now, it returns a static mock token.
        "deepcloud_proxy_token_v1".to_string()
    }
}

use std::sync::OnceLock;

pub fn global_provider_store() -> &'static ProviderStore {
    static STORE: OnceLock<ProviderStore> = OnceLock::new();
    STORE.get_or_init(ProviderStore::new)
}
