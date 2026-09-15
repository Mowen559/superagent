use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone)]
pub struct ProxyTokenSession {
    pub token: String,
    pub provider_id: String,
    pub created_at: Instant,
    pub ttl: Duration,
}

pub struct ProviderStore {
    providers: HashMap<String, ProviderConfig>,
    active_tokens: RwLock<HashMap<String, ProxyTokenSession>>,
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
        Self {
            providers,
            active_tokens: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_provider(&self, provider_id: &str) -> Option<ProviderConfig> {
        self.providers.get(provider_id).cloned()
    }

    pub fn generate_proxy_token(&self, provider_id: &str) -> String {
        let token = format!("dc-token-{}-{}", provider_id, uuid::Uuid::new_v4());
        let session = ProxyTokenSession {
            token: token.clone(),
            provider_id: provider_id.to_string(),
            created_at: Instant::now(),
            ttl: Duration::from_secs(86400),
        };
        if let Ok(mut tokens) = self.active_tokens.write() {
            tokens.retain(|_, s| s.created_at.elapsed() < s.ttl);
            tokens.insert(token.clone(), session);
        }
        token
    }

    pub fn validate_proxy_token(&self, token: &str) -> bool {
        if token == "deepcloud_proxy_token_v1" {
            return true;
        }
        if let Ok(tokens) = self.active_tokens.read() {
            if let Some(session) = tokens.get(token) {
                return session.created_at.elapsed() < session.ttl;
            }
        }
        false
    }
}

pub fn global_provider_store() -> &'static ProviderStore {
    static STORE: OnceLock<ProviderStore> = OnceLock::new();
    STORE.get_or_init(ProviderStore::new)
}

