use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub service_tiers: Vec<ModelServiceTier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ModelThinking>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelServiceTier {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelThinking {
    pub supported_levels: Vec<ThinkingLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThinkingLevel {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct CacheKey {
    pub provider: String,
    pub executable_path: String,
}

pub struct CacheEntry<T> {
    pub data: T,
    pub expires_at: Instant,
}

static MODEL_CACHE: OnceLock<RwLock<HashMap<CacheKey, CacheEntry<Vec<Model>>>>> = OnceLock::new();
static THINKING_CACHE: OnceLock<RwLock<HashMap<CacheKey, CacheEntry<HashMap<String, ModelThinking>>>>> = OnceLock::new();

/// Cached asynchronous discovery for models.
pub async fn cached_discovery<F, Fut>(
    provider: String,
    executable_path: String,
    ttl: Duration,
    fetch: F,
) -> crate::services::agent::backend::BackendResult<Vec<Model>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = crate::services::agent::backend::BackendResult<Vec<Model>>>,
{
    let key = CacheKey { provider, executable_path };
    let cache = MODEL_CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    if let Ok(guard) = cache.read() {
        if let Some(entry) = guard.get(&key) {
            if Instant::now() < entry.expires_at {
                return Ok(entry.data.clone());
            }
        }
    }

    let models = fetch().await?;

    if models.is_empty() {
        return Ok(models);
    }

    if let Ok(mut guard) = cache.write() {
        guard.insert(
            key,
            CacheEntry {
                data: models.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
    }

    Ok(models)
}

/// Cached asynchronous discovery for thinking levels (e.g. Claude).
pub async fn cached_thinking<F, Fut>(
    provider: String,
    executable_path: String,
    ttl: Duration,
    fetch: F,
) -> crate::services::agent::backend::BackendResult<HashMap<String, ModelThinking>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = crate::services::agent::backend::BackendResult<HashMap<String, ModelThinking>>>,
{
    let key = CacheKey { provider, executable_path };
    let cache = THINKING_CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    if let Ok(guard) = cache.read() {
        if let Some(entry) = guard.get(&key) {
            if Instant::now() < entry.expires_at {
                return Ok(entry.data.clone());
            }
        }
    }

    let thinking_map = fetch().await?;

    if let Ok(mut guard) = cache.write() {
        guard.insert(
            key,
            CacheEntry {
                data: thinking_map.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
    }

    Ok(thinking_map)
}
