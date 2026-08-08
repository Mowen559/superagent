use crate::config::llm_settings::{self, LLMAccount};
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dialoguer::{theme::ColorfulTheme, Select, Input};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct TokenRes {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthRes {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: i64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

struct AppState {
    expected_state: String,
    tx: std::sync::Mutex<Option<oneshot::Sender<Result<String, String>>>>,
}

fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn ensure_logged_in(force: bool) -> Result<(), String> {
    let mut settings = llm_settings::read_llm_settings();

    // If an account is active and has credentials, we are good.
    if !force && settings.active_account_id.is_some() && !settings.api_key.is_empty() {
        return Ok(());
    }

    loop {
        println!("\n[Auth] No active session found.");
        
        // Use dialoguer to present options
        let mut options = vec![];
        for acc in &settings.accounts {
            let quota_str = acc.quota_info.as_ref()
                .and_then(|q| {
                    if q.is_null() { None }
                    else if let Some(s) = q.as_str() { Some(s.to_string()) }
                    else { Some(q.to_string()) }
                })
                .map(|s| format!(" [Quota: {}]", s))
                .unwrap_or_default();
            options.push(format!("Use existing: {} ({}){}", acc.name, acc.provider, quota_str));
        }
        options.push("Add New Provider...".to_string());
        if !settings.accounts.is_empty() {
            options.push("Remove Provider...".to_string());
        }
        options.push("Exit".to_string());
        
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select an account to use")
            .default(0)
            .items(&options)
            .interact()
            .map_err(|e| format!("Failed to read selection: {}", e))?;

        if selection < settings.accounts.len() {
            // Switch to existing account
            let selected_id = settings.accounts[selection].id.clone();
            settings.active_account_id = Some(selected_id);
            llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap());
            println!("[Auth] Switched to account: {}", options[selection]);
            return Ok(());
        } else if options[selection] == "Add New Provider..." {
            break;
        } else if options[selection] == "Remove Provider..." {
            let mut remove_options = vec![];
            for acc in &settings.accounts {
                let quota_str = acc.quota_info.as_ref()
                    .and_then(|q| {
                        if q.is_null() { None }
                        else if let Some(s) = q.as_str() { Some(s.to_string()) }
                        else { Some(q.to_string()) }
                    })
                    .map(|s| format!(" [Quota: {}]", s))
                    .unwrap_or_default();
                remove_options.push(format!("{} ({}){}", acc.name, acc.provider, quota_str));
            }
            remove_options.push("Cancel".to_string());
            
            let remove_selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select an account to remove")
                .default(0)
                .items(&remove_options)
                .interact()
                .map_err(|e| format!("Failed to read selection: {}", e))?;
                
            if remove_selection < settings.accounts.len() {
                let removed = settings.accounts.remove(remove_selection);
                if settings.active_account_id == Some(removed.id.clone()) {
                    settings.active_account_id = None;
                }
                llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap());
                println!("[Auth] Removed account: {} ({})", removed.name, removed.provider);
            }
            continue;
        } else {
            return Err("Cancelled auth flow".into());
        }
    }

    // Add New Provider Flow - Categories
    let categories = vec!["Official", "Third-Party", "Aggregator", "Custom"];
    let cat_selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select Provider Category")
        .default(0)
        .items(&categories)
        .interact()
        .map_err(|e| format!("Failed to read category selection: {}", e))?;
        
    let selected_category = categories[cat_selection];
    
    let new_account = if selected_category == "Custom" {
        handle_custom_api_key().await?
    } else {
        // Filter providers by category
        let mut available_providers: Vec<&crate::services::provider_presets::ProviderPreset> = crate::services::provider_presets::PROVIDERS
            .iter()
            .filter(|p| p.category == selected_category)
            .collect();
            
        let provider_names: Vec<String> = available_providers.iter().map(|p| p.name.to_string()).collect();
        
        let provider_selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Select Provider in {}", selected_category))
            .default(0)
            .items(&provider_names)
            .interact()
            .map_err(|e| format!("Failed to read provider selection: {}", e))?;
            
        let provider = available_providers[provider_selection];
        
        if provider.supports_oauth {
            let auth_options = vec!["OAuth (Browser Login)", "API Key (Manual Entry)"];
            let auth_selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select Authentication Method")
                .default(0)
                .items(&auth_options)
                .interact()
                .map_err(|e| format!("Failed to read auth method: {}", e))?;
                
            if auth_selection == 0 {
                // Route to specific OAuth flow based on provider name
                if provider.name == "OpenAI Official" {
                    handle_openai_device_flow().await?
                } else if provider.name == "Gemini Native" {
                    handle_gemini_oauth_flow().await?
                } else {
                    // Fallback to API Key if no local OAuth implementation exists yet for others
                    println!("[Auth] Note: Native OAuth for {} is not yet implemented in hub-rs. Falling back to API Key.", provider.name);
                    let default_url = if provider.base_url.is_empty() { "https://api.openai.com/v1" } else { provider.base_url };
                    handle_custom_api_key_with_defaults(provider.name, &provider.name.to_lowercase().replace(" ", "_"), default_url).await?
                }
            } else {
                let default_url = if provider.base_url.is_empty() { "https://api.openai.com/v1" } else { provider.base_url };
                handle_custom_api_key_with_defaults(provider.name, &provider.name.to_lowercase().replace(" ", "_"), default_url).await?
            }
        } else {
            let default_url = if provider.base_url.is_empty() { "https://api.openai.com/v1" } else { provider.base_url };
            handle_custom_api_key_with_defaults(provider.name, &provider.name.to_lowercase().replace(" ", "_"), default_url).await?
        }
    };

    settings.accounts.push(new_account.clone());
    settings.active_account_id = Some(new_account.id);
    llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap());

    println!("[Auth] \x1b[32mLogin successful!\x1b[0m Your session has been saved.\n");
    Ok(())
}

async fn handle_openai_device_flow() -> Result<LLMAccount, String> {
    println!("[Auth] Starting Device Code flow for OpenAI...");
    let client = Client::new();
    let client_id = "app_EMoamEEZ73f0CkXaXp7hrann"; // cc-switch Codex client id
    
    // 1. Get device code
    let res_text = client.post("https://auth0.openai.com/oauth/device/code")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "client_id": client_id,
            "audience": "https://api.openai.com/v1",
            "scope": "openid profile email offline_access"
        }))
        .send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let device_auth: DeviceAuthRes = serde_json::from_str(&res_text)
        .map_err(|e| format!("Failed to parse device code response: {} (raw: {})", e, res_text))?;

    println!("\n=======================================================");
    println!("Please open this URL in your browser:");
    println!("  {}", device_auth.verification_uri);
    println!("\nAnd enter the following code:");
    println!("  \x1b[1m\x1b[33m{}\x1b[0m", device_auth.user_code);
    println!("=======================================================\n");

    let _ = open::that(&device_auth.verification_uri);

    // 2. Poll for token
    println!("Waiting for authorization...");
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(device_auth.interval.max(3)));
    
    loop {
        interval.tick().await;
        let token_res_text = client.post("https://auth0.openai.com/oauth/token")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": client_id,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                "device_code": device_auth.device_code,
            }))
            .send().await.map_err(|e| e.to_string())?
            .text().await.map_err(|e| e.to_string())?;
            
        let token_res: TokenRes = serde_json::from_str(&token_res_text).unwrap_or_else(|_| TokenRes {
            access_token: None, refresh_token: None, expires_in: None, error: Some("Parse error".into())
        });
        
        if let Some(err) = token_res.error {
            if err == "authorization_pending" {
                continue;
            } else if err == "expired_token" {
                return Err("Device code expired".to_string());
            } else if err == "slow_down" {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            } else {
                return Err(format!("OAuth error: {}", err));
            }
        }
        
        if let Some(access_token) = token_res.access_token {
            let expires_at = token_res.expires_in.map(|secs| chrono::Utc::now().timestamp() + secs);
            
            let name: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Account Name")
                .default("OpenAI ChatGPT".into())
                .interact_text()
                .map_err(|e| e.to_string())?;
                
            return Ok(LLMAccount {
                id: Uuid::new_v4().to_string(),
                name,
                enabled: true,
                provider: "openai".to_string(),
                models: vec![],
                model_mapping: std::collections::HashMap::new(),
                base_url: "https://chatgpt.com/backend-api/codex".to_string(), // match cc-switch
                api_key: access_token,
                refresh_token: token_res.refresh_token,
                expires_at,
                project_id: None,
                quota_info: None,
            });
        }
    }
}

async fn fetch_cloudaicompanion_project(access_token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let res = client.post("https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:loadCodeAssist")
        .bearer_auth(access_token)
        .header("User-Agent", "vscode/1.X.X (Antigravity/4.3.0)")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "metadata": { "ideType": "ANTIGRAVITY" }
        }))
        .send().await.map_err(|e| format!("Failed to request loadCodeAssist: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("loadCodeAssist returned error: {}", res.status()));
    }

    let data: serde_json::Value = res.json().await.map_err(|e| format!("Failed to parse response: {}", e))?;
    
    if let Some(project_id) = data.get("cloudaicompanionProject").and_then(|v| v.as_str()) {
        return Ok(project_id.to_string());
    }

    Err("Could not find cloudaicompanionProject in response".to_string())
}

async fn handle_gemini_oauth_flow() -> Result<LLMAccount, String> {
    println!("[Auth] Starting web login flow to authenticate with Google (Gemini Native)...");
    let client_id = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
    let client_secret = "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";
    let auth_endpoint = "https://accounts.google.com/o/oauth2/v2/auth";
    let token_endpoint = "https://oauth2.googleapis.com/token";
    let scope = "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
    
    let (access_token, refresh_token, expires_in) = execute_pkce_flow(client_id, Some(client_secret), auth_endpoint, token_endpoint, scope).await?;
    let expires_at = expires_in.map(|secs| chrono::Utc::now().timestamp() + secs);
    
    println!("[Auth] Fetching Google Cloud AI Companion Project...");
    let project_id = match fetch_cloudaicompanion_project(&access_token).await {
        Ok(pid) => pid,
        Err(e) => {
            println!("[Auth] Warning: {}", e);
            return Err("Failed to fetch companion project. Your account might not have access.".to_string());
        }
    };
    
    println!("[Auth] Successfully bound to project: {}", project_id);
    let base_url = format!("https://us-central1-aiplatform.googleapis.com/v1beta1/projects/{}/locations/us-central1/endpoints/openapi", project_id);
    
    // Save to Windows Credential Manager for the local proxy
    #[cfg(target_os = "windows")]
    {
        let expiry = expires_at.map(|ts| (chrono::Utc::now() + chrono::Duration::seconds(expires_in.unwrap_or(3599)))
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true));
            
        let payload = serde_json::json!({
            "token": {
                "access_token": access_token.clone(),
                "token_type": "Bearer",
                "refresh_token": refresh_token.clone().unwrap_or_default(),
                "expiry": expiry
            },
            "auth_method": "web_oauth"
        });

        if let Ok(payload_str) = serde_json::to_string(&payload) {
            let target_name = "gemini:antigravity";
            if crate::services::keyring_auth::write_credential(target_name, payload_str.as_bytes()) {
                println!("[Auth] Successfully saved OAuth token to Windows Credential Manager as '{}'", target_name);
            } else {
                eprintln!("[Auth] Failed to save token to Windows Credential Manager.");
            }
        }
    }

    let name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Account Name")
        .default("Gemini Native (Copilot)".into())
        .interact_text()
        .map_err(|e| e.to_string())?;
                
    Ok(LLMAccount {
        id: Uuid::new_v4().to_string(),
        name,
        enabled: true,
        provider: "gemini_native".to_string(),
        models: vec![],
        model_mapping: std::collections::HashMap::new(),
        base_url,
        api_key: access_token,
        refresh_token,
        expires_at,
        project_id: Some(project_id),
        quota_info: None,
    })
}

async fn handle_deepcloud_sso_flow() -> Result<LLMAccount, String> {
    println!("[Auth] Starting web login flow to authenticate with DeepCloud SSO...");
    // For MVP, just return an error and instruct user to use custom key
    Err("DeepCloud SSO is currently undergoing migration, please use Custom API Key".to_string())
}

async fn handle_custom_api_key() -> Result<LLMAccount, String> {
    let name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Account Name (e.g. My Provider)")
        .interact_text()
        .map_err(|e| e.to_string())?;
        
    let base_url: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Base URL")
        .default("https://api.openai.com/v1".into())
        .interact_text()
        .map_err(|e| e.to_string())?;
        
    let api_key: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("API Key")
        .interact_text()
        .map_err(|e| e.to_string())?;

    Ok(LLMAccount {
        id: Uuid::new_v4().to_string(),
        name,
        enabled: true,
        provider: "openai-compatible".to_string(),
        models: vec![],
        model_mapping: std::collections::HashMap::new(),
        base_url,
        api_key,
        refresh_token: None,
        expires_at: None,
        project_id: None,
        quota_info: None,
    })
}

async fn handle_custom_api_key_with_defaults(display_name: &str, provider: &str, default_url: &str) -> Result<LLMAccount, String> {
    let name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Account Name")
        .default(format!("{} (API Key)", display_name))
        .interact_text()
        .map_err(|e| e.to_string())?;
        
    let base_url: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Base URL")
        .default(default_url.into())
        .interact_text()
        .map_err(|e| e.to_string())?;
        
    let api_key: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("API Key")
        .interact_text()
        .map_err(|e| e.to_string())?;

    Ok(LLMAccount {
        id: Uuid::new_v4().to_string(),
        name,
        enabled: true,
        provider: provider.to_string(),
        models: vec![],
        model_mapping: std::collections::HashMap::new(),
        base_url,
        api_key,
        refresh_token: None,
        expires_at: None,
        project_id: None,
        quota_info: None,
    })
}

async fn execute_pkce_flow(
    client_id: &str, 
    client_secret: Option<&str>,
    auth_endpoint: &str, 
    token_endpoint: &str, 
    scope: &str
) -> Result<(String, Option<String>, Option<i64>), String> {
    let port = 8765;
    let redirect_uri = format!("http://127.0.0.1:{}/callback", port);

    // Generate PKCE params
    let mut verifier_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut verifier_bytes);
    let verifier = b64url(&verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = b64url(&hasher.finalize());

    let mut state_bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut state_bytes);
    let state = b64url(&state_bytes);

    // Construct Authorization URL
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&scope={}&state={}",
        auth_endpoint,
        client_id,
        urlencoding::encode(&redirect_uri),
        challenge,
        urlencoding::encode(scope),
        state
    );

    // Start Local Server to receive callback
    let (tx, rx) = oneshot::channel();
    let app_state = std::sync::Arc::new(AppState {
        expected_state: state.clone(),
        tx: std::sync::Mutex::new(Some(tx)),
    });

    let app = Router::new()
        .route("/callback", get(handle_callback))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("Failed to bind to port {}: {}", port, e))?;
    
    // Spawn server task
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Open Browser
    println!("Opening your browser to log in...");
    println!("{}", auth_url);
    if let Err(e) = open::that(&auth_url) {
        eprintln!("Failed to automatically open browser ({}). Please open the URL manually.", e);
    }

    // Wait for Callback
    let code = match rx.await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(format!("Login failed: {}", e)),
        Err(_) => return Err("Server channel closed unexpectedly".to_string()),
    };

    println!("[Auth] Received callback. Exchanging code for token...");

    // Exchange Code for Token
    let client = Client::new();
    let req = client.post(token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded");
        
    let mut body = format!(
        "grant_type=authorization_code&code={}&client_id={}&code_verifier={}&redirect_uri={}",
        code,
        client_id,
        verifier,
        urlencoding::encode(&redirect_uri)
    );
    
    if let Some(secret) = client_secret {
        body.push_str(&format!("&client_secret={}", secret));
    }

    let token_res_text = req.body(body)
        .send().await.map_err(|e| format!("Token request failed: {}", e))?
        .text().await.map_err(|e| format!("Failed to read token response: {}", e))?;

    let token_res: TokenRes = serde_json::from_str(&token_res_text)
        .map_err(|e| format!("Failed to parse token response ({}): {}", e, token_res_text))?;

    if let Some(err) = token_res.error {
        return Err(format!("OAuth Error during token exchange: {}", err));
    }

    let access_token = token_res.access_token.ok_or("No access_token returned")?;
    Ok((access_token, token_res.refresh_token, token_res.expires_in))
}

async fn handle_callback(
    State(state): State<std::sync::Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
) -> impl IntoResponse {
    let mut tx_guard = state.tx.lock().unwrap();
    if let Some(tx) = tx_guard.take() {
        if let Some(err) = query.error {
            let _ = tx.send(Err(err));
            return Html("<html><body><h1>Login Failed</h1><p>Check the terminal.</p></body><script>window.close()</script></html>");
        }
        
        let req_state = query.state.unwrap_or_default();
        if req_state != state.expected_state {
            let _ = tx.send(Err("State mismatch. Possible CSRF attack.".into()));
            return Html("<html><body><h1>Login Failed</h1><p>State mismatch.</p></body><script>window.close()</script></html>");
        }

        if let Some(code) = query.code {
            let _ = tx.send(Ok(code));
            return Html("<html><body><h1>Login Successful!</h1><p>You can close this tab and return to the terminal.</p></body><script>window.close()</script></html>");
        }
        
        let _ = tx.send(Err("No code found in callback".into()));
        Html("<html><body><h1>Login Failed</h1><p>No code found.</p></body><script>window.close()</script></html>")
    } else {
        Html("<html><body><p>Login already processed. You can close this tab.</p></body><script>window.close()</script></html>")
    }
}
