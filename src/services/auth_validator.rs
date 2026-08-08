use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

fn token_cache() -> &'static RwLock<HashMap<String, (String, Instant)>> {
    static CACHE: OnceLock<RwLock<HashMap<String, (String, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

#[derive(Debug, Deserialize)]
struct SessionResponse {
    #[serde(rename = "userId")]
    _user_id: Option<String>,
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    error: Option<String>,
}

/// Helper function to check if the provided auth string is a static API key or an OAuth token.
pub fn is_api_key(token: &str) -> bool {
    // Standard static keys usually start with prefix like 'sk-', 'sm_', etc.
    // OAuth tokens (like JWTs) are typically much longer and structured (e.g. eyJ...).
    token.starts_with("sk-") || token.starts_with("sm_") || !token.starts_with("eyJ")
}

/// Validate the given OAuth token against the deepcloud/supermemory SSO endpoint.
/// If valid, returns the real underlying LLM API key.
pub async fn validate_oauth_token(client: &Client, token: &str, sso_api_url: &str) -> Result<String, String> {
    // 1. Check local cache first
    {
        let cache = token_cache().read().await;
        if let Some((api_key, expires_at)) = cache.get(token) {
            if Instant::now() < *expires_at {
                return Ok(api_key.clone());
            }
        }
    }

    // 2. Introspect via remote server
    let endpoint = format!("{}/v3/mcp/session-with-key", sso_api_url.trim_end_matches('/'));
    
    let res = client
        .get(&endpoint)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Network error during token validation: {}", e))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Token validation failed ({}): {}", status, body));
    }

    let session_data: SessionResponse = res.json()
        .await
        .map_err(|e| format!("Failed to parse session response: {}", e))?;

    if let Some(err) = session_data.error {
        return Err(format!("SSO Error: {}", err));
    }

    if let Some(api_key) = session_data.api_key {
        // 3. Update cache (TTL = 10 minutes)
        let mut cache = token_cache().write().await;
        cache.insert(
            token.to_string(), 
            (api_key.clone(), Instant::now() + Duration::from_secs(600))
        );
        Ok(api_key)
    } else {
        Err("Invalid session response: missing apiKey".to_string())
    }
}
