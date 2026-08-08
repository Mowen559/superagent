use serde::{Deserialize, Serialize};
use crate::config::llm_settings::read_llm_settings;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaTier {
    pub name: String,
    pub utilization: f64,
    pub resets_at: Option<String>,
}

pub async fn handle_quota() {
    let settings = read_llm_settings();
    if settings.accounts.is_empty() {
        println!("No accounts configured.");
        return;
    }

    let client = reqwest::Client::new();

    for account in &settings.accounts {
        if !account.enabled {
            println!("Account: {} (Disabled)", account.name);
            continue;
        }

        println!("Account: {} (Provider: {})", account.name, account.provider);

        match account.provider.as_str() {
            "anthropic" => {
                query_claude_quota(&client, &account.api_key).await;
            }
            "openai" | "codex" => {
                query_openai_quota(&client, &account.api_key, account.project_id.as_deref()).await;
            }
            "gemini" | "gemini_native" => {
                query_gemini_quota(&client, account).await;
            }
            _ => {
                println!("  Quota query not supported for provider: {}", account.provider);
            }
        }
        println!();
    }
}

fn format_reset_time_iso(iso_time: &str) -> String {
    if iso_time == "unknown" {
        return "unknown".to_string();
    }
    
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_time) {
        let now = chrono::Utc::now();
        let diff = dt.with_timezone(&chrono::Utc) - now;
        
        if diff.num_seconds() <= 0 {
            return "resetting now".to_string();
        }
        
        let days = diff.num_days();
        let hours = diff.num_hours() % 24;
        let mins = diff.num_minutes() % 60;
        
        if days > 0 {
            format!("{}d {}h", days, hours)
        } else if hours > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}m", mins)
        }
    } else {
        iso_time.to_string()
    }
}

fn format_reset_time_unix(ts: i64) -> String {
    let dt = chrono::DateTime::from_timestamp(ts, 0).unwrap_or_default();
    let iso = dt.to_rfc3339();
    format_reset_time_iso(&iso)
}

async fn query_claude_quota(client: &reqwest::Client, access_token: &str) {
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                let mut found = false;
                if let Some(obj) = body.as_object() {
                    for (k, v) in obj {
                        if k == "extra_usage" { continue; }
                        if let Some(util) = v.get("utilization").and_then(|u| u.as_f64()) {
                            let resets = v.get("resets_at").and_then(|r| r.as_str()).unwrap_or("unknown");
                            let remaining = 100.0 - util;
                            println!("  - {}: Remaining: {:.2}% (Resets in: {})", k, remaining, format_reset_time_iso(resets));
                            found = true;
                        }
                    }
                }
                if !found {
                    println!("  No quota information found in response.");
                }
            } else {
                println!("  Failed to parse JSON response.");
            }
        }
        Ok(r) => {
            println!("  Failed to fetch quota: HTTP {}", r.status());
        }
        Err(e) => {
            println!("  Network error: {}", e);
        }
    }
}

async fn query_openai_quota(client: &reqwest::Client, access_token: &str, account_id: Option<&str>) {
    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", "codex-cli")
        .header("Accept", "application/json");

    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }

    match req.timeout(std::time::Duration::from_secs(10)).send().await {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                let mut found = false;
                if let Some(rate_limit) = body.get("rate_limit") {
                    for window_key in &["primary_window", "secondary_window"] {
                        if let Some(window) = rate_limit.get(window_key) {
                            if let Some(used) = window.get("used_percent").and_then(|u| u.as_f64()) {
                                let window_secs = window.get("limit_window_seconds").and_then(|u| u.as_i64()).unwrap_or(0);
                                let reset_at = window.get("reset_at").and_then(|u| u.as_i64());
                                let remaining = 100.0 - used;
                                
                                let reset_str = if let Some(ts) = reset_at {
                                    format_reset_time_unix(ts)
                                } else {
                                    "unknown".to_string()
                                };
                                
                                println!("  - {} ({}s): Remaining: {:.2}% (Resets in: {})", window_key, window_secs, remaining, reset_str);
                                found = true;
                            }
                        }
                    }
                }
                if !found {
                    println!("  No rate limit information found.");
                }
            } else {
                println!("  Failed to parse JSON response.");
            }
        }
        Ok(r) => println!("  Failed to fetch quota: HTTP {}", r.status()),
        Err(e) => println!("  Network error: {}", e),
    }
}

const GEMINI_OAUTH_CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GEMINI_OAUTH_CLIENT_SECRET: &str = "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";

async fn refresh_gemini_token(client: &reqwest::Client, refresh_token: &str) -> Option<String> {
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", GEMINI_OAUTH_CLIENT_ID),
            ("client_secret", GEMINI_OAUTH_CLIENT_SECRET),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body = resp.json::<serde_json::Value>().await.ok()?;
    body.get("access_token")?.as_str().map(String::from)
}

async fn query_gemini_quota(client: &reqwest::Client, account: &crate::config::llm_settings::LLMAccount) {
    let mut access_token = account.api_key.clone();

    let is_native = account.provider == "gemini_native";
    let base_url = if is_native {
        "https://daily-cloudcode-pa.sandbox.googleapis.com"
    } else {
        "https://cloudcode-pa.googleapis.com"
    };
    
    let ide_type = if is_native { "ANTIGRAVITY" } else { "GEMINI_CLI" };
    let plugin_type = if is_native { None } else { Some("GEMINI") };
    
    let mut load_req = client
        .post(format!("{}/v1internal:loadCodeAssist", base_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");
        
    if is_native {
        load_req = load_req.header("User-Agent", "vscode/1.X.X (Antigravity/4.3.0)");
    }
    
    let mut metadata = serde_json::json!({
        "ideType": ide_type
    });
    if let Some(pt) = plugin_type {
        metadata["pluginType"] = serde_json::Value::String(pt.to_string());
    }

    let load_resp = load_req
        .json(&serde_json::json!({
            "metadata": metadata
        }))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let mut project_id = None;
    match load_resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                project_id = body.get("cloudaicompanionProject")
                    .and_then(|v| {
                        if let Some(s) = v.as_str() { Some(s.to_string()) }
                        else if let Some(obj) = v.as_object() {
                            obj.get("id").or_else(|| obj.get("projectId")).and_then(|id| id.as_str()).map(|s| s.to_string())
                        } else { None }
                    });
            }
        }
        Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED || r.status() == reqwest::StatusCode::FORBIDDEN => {
            // Token might be expired, attempt refresh
            if let Some(ref rt) = account.refresh_token {
                if let Some(new_token) = refresh_gemini_token(client, rt).await {
                    access_token = new_token.clone();
                    
                    // Persist the new token to settings
                    let mut settings = read_llm_settings();
                    for a in &mut settings.accounts {
                        if a.id == account.id {
                            a.api_key = new_token.clone();
                            if let Some(expires_in) = 3600_i64.into() { // Approximate, usually 1h
                                a.expires_at = Some(chrono::Utc::now().timestamp_millis() + expires_in * 1000);
                            }
                            break;
                        }
                    }
                    crate::config::llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap_or_default());
                    
                    // Retry loadCodeAssist
                    let mut retry_req = client
                        .post(format!("{}/v1internal:loadCodeAssist", base_url))
                        .header("Authorization", format!("Bearer {}", access_token))
                        .header("Content-Type", "application/json");
                        
                    if is_native {
                        retry_req = retry_req.header("User-Agent", "vscode/1.X.X (Antigravity/4.3.0)");
                    }

                    let retry_resp = retry_req
                        .json(&serde_json::json!({
                            "metadata": metadata
                        }))
                        .timeout(std::time::Duration::from_secs(10))
                        .send()
                        .await;

                    if let Ok(rr) = retry_resp {
                        if rr.status().is_success() {
                            if let Ok(body) = rr.json::<serde_json::Value>().await {
                                project_id = body.get("cloudaicompanionProject")
                                    .and_then(|v| {
                                        if let Some(s) = v.as_str() { Some(s.to_string()) }
                                        else if let Some(obj) = v.as_object() {
                                            obj.get("id").or_else(|| obj.get("projectId")).and_then(|id| id.as_str()).map(|s| s.to_string())
                                        } else { None }
                                    });
                            }
                        } else {
                            println!("  Failed to loadCodeAssist after refresh: HTTP {}", rr.status());
                            let body = rr.text().await.unwrap_or_default();
                            if !body.is_empty() { println!("  Body: {}", body); }
                            return;
                        }
                    } else {
                        println!("  Network error after refresh");
                        return;
                    }
                } else {
                    println!("  Failed to refresh token (HTTP {})", r.status());
                    return;
                }
            } else {
                println!("  Authentication failed: HTTP {} (No refresh token available)", r.status());
                return;
            }
        }
        Ok(r) => {
            println!("  Failed to loadCodeAssist: HTTP {}", r.status());
            let body = r.text().await.unwrap_or_default();
            if !body.is_empty() { println!("  Body: {}", body); }
            return;
        }
        Err(e) => {
            println!("  Network error (loadCodeAssist): {}", e);
            return;
        }
    }

    let mut quota_body = serde_json::json!({});
    if let Some(pid) = project_id {
        quota_body["project"] = serde_json::Value::String(pid);
    }

    let mut quota_req = client
        .post(format!("{}/v1internal:retrieveUserQuota", base_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");
        
    if is_native {
        quota_req = quota_req.header("User-Agent", "vscode/1.X.X (Antigravity/4.3.0)");
    }

    let quota_resp = quota_req
        .json(&quota_body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match quota_resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                let mut found = false;
                if let Some(buckets) = body.get("buckets").and_then(|b| b.as_array()) {
                    for bucket in buckets {
                        let remaining = bucket.get("remainingFraction").and_then(|u| u.as_f64()).unwrap_or(1.0);
                        let model_id = bucket.get("modelId").and_then(|m| m.as_str()).unwrap_or("unknown");
                        let reset_time = bucket.get("resetTime").and_then(|m| m.as_str()).unwrap_or("unknown");
                        let remaining_pct = remaining * 100.0;
                        println!("  - {}: Remaining: {:.2}% (Resets in: {})", model_id, remaining_pct, format_reset_time_iso(reset_time));
                        found = true;
                    }
                }
                if !found {
                    println!("  No quota buckets found.");
                }
            } else {
                println!("  Failed to parse JSON response.");
            }
        }
        Ok(r) => {
            println!("  Failed to retrieveUserQuota: HTTP {}", r.status());
            let body = r.text().await.unwrap_or_default();
            if !body.is_empty() { println!("  Body: {}", body); }
        }
        Err(e) => println!("  Network error (retrieveUserQuota): {}", e),
    }
}
