use crate::config::llm_settings::{self, LLMAccount};
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dialoguer::{theme::ColorfulTheme, Input, Select};
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
            let quota_str = acc
                .quota_info
                .as_ref()
                .and_then(|q| {
                    if q.is_null() {
                        None
                    } else if let Some(s) = q.as_str() {
                        Some(s.to_string())
                    } else {
                        Some(q.to_string())
                    }
                })
                .map(|s| format!(" [Quota: {}]", s))
                .unwrap_or_default();
            options.push(format!(
                "Use existing: {} ({}){}",
                acc.name, acc.provider, quota_str
            ));
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
                let quota_str = acc
                    .quota_info
                    .as_ref()
                    .and_then(|q| {
                        if q.is_null() {
                            None
                        } else if let Some(s) = q.as_str() {
                            Some(s.to_string())
                        } else {
                            Some(q.to_string())
                        }
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
                println!(
                    "[Auth] Removed account: {} ({})",
                    removed.name, removed.provider
                );
            }
            continue;
        } else {
            return Err("Cancelled auth flow".into());
        }
    }

    // Add New Provider Flow - Categories
    let categories = vec![
        "OAuth (Native)",
        "Official",
        "Third-Party",
        "Aggregator",
        "Custom",
    ];
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
        let mut available_providers: Vec<&crate::services::provider_presets::ProviderPreset> =
            crate::services::provider_presets::PROVIDERS
                .iter()
                .filter(|p| p.category == selected_category)
                .collect();

        let provider_names: Vec<String> = available_providers
            .iter()
            .map(|p| p.name.to_string())
            .collect();

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
                if provider.name == "OpenAI Native" {
                    handle_openai_pkce_flow(None).await?
                } else if provider.name == "Gemini Native" {
                    handle_gemini_oauth_flow(None).await?
                } else if provider.name == "GitHub Copilot" {
                    handle_github_copilot_flow(None).await?
                } else if provider.name == "Claude Desktop Official" {
                    handle_claude_oauth_flow(None).await?
                } else {
                    // Fallback to API Key if no local OAuth implementation exists yet for others
                    println!("[Auth] Note: Native OAuth for {} is not yet implemented in hub-rs. Falling back to API Key.", provider.name);
                    let default_url = if provider.base_url.is_empty() {
                        "https://api.openai.com/v1"
                    } else {
                        provider.base_url
                    };
                    handle_custom_api_key_with_defaults(
                        provider.name,
                        &provider.name.to_lowercase().replace(" ", "_"),
                        default_url,
                    )
                    .await?
                }
            } else {
                let default_url = if provider.base_url.is_empty() {
                    "https://api.openai.com/v1"
                } else {
                    provider.base_url
                };
                handle_custom_api_key_with_defaults(
                    provider.name,
                    &provider.name.to_lowercase().replace(" ", "_"),
                    default_url,
                )
                .await?
            }
        } else {
            let default_url = if provider.base_url.is_empty() {
                "https://api.openai.com/v1"
            } else {
                provider.base_url
            };
            handle_custom_api_key_with_defaults(
                provider.name,
                &provider.name.to_lowercase().replace(" ", "_"),
                default_url,
            )
            .await?
        }
    };

    settings.accounts.push(new_account.clone());
    settings.active_account_id = Some(new_account.id);
    llm_settings::write_llm_settings(serde_json::to_value(&settings).unwrap());

    println!("[Auth] \x1b[32mLogin successful!\x1b[0m Your session has been saved.\n");
    Ok(())
}

pub async fn handle_openai_pkce_flow(account_name: Option<String>) -> Result<LLMAccount, String> {
    println!("[Auth] Starting PKCE flow for OpenAI...");

    let client_id = "app_EMoamEEZ73f0CkXaXp7hrann"; // Codex / OpenAI Native client id
    let auth_endpoint = "https://auth.openai.com/oauth/authorize";
    let token_endpoint = "https://auth.openai.com/oauth/token";
    let scope = "openid profile email offline_access";
    let port = 1455;
    let redirect_path = "/auth/callback";
    let redirect_uri = format!("http://localhost:{}{}", port, redirect_path);

    let extra_auth_params =
        "&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=codex_cli_rs";

    let (access_token, refresh_token, expires_in) = execute_pkce_flow(
        client_id,
        None,
        auth_endpoint,
        token_endpoint,
        scope,
        port,
        &redirect_uri,
        redirect_path,
        extra_auth_params,
    )
    .await?;

    let expires_at = expires_in.map(|secs| chrono::Utc::now().timestamp() + secs);

    // Save to Windows Credential Manager for the local proxy
    #[cfg(target_os = "windows")]
    {
        let expiry = expires_at
            .map(|ts| {
                (chrono::Utc::now() + chrono::Duration::seconds(expires_in.unwrap_or(3599)))
                    .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            })
            .unwrap_or_else(|| {
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            });

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
            let target_name = "openai:antigravity";
            if crate::services::keyring_auth::write_credential(target_name, payload_str.as_bytes())
            {
                println!(
                    "[Auth] Successfully saved OAuth token to Windows Credential Manager as '{}'",
                    target_name
                );
            } else {
                eprintln!("[Auth] Failed to save token to Windows Credential Manager.");
            }
        }
    }

    let name: String = match account_name {
        Some(n) => n,
        None => Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Account Name")
            .default("OpenAI Native".into())
            .interact_text()
            .map_err(|e| e.to_string())?,
    };

    Ok(LLMAccount {
        id: Uuid::new_v4().to_string(),
        name,
        enabled: true,
        provider: "openai".to_string(),
        models: vec![
            "gpt-4o".to_string(),
            "gpt-4-turbo".to_string(),
            "gpt-3.5-turbo".to_string(),
        ],
        models_info: None,
        model_mapping: std::collections::HashMap::new(),
        base_url: "https://chatgpt.com/backend-api/codex".to_string(),
        api_key: access_token,
        refresh_token,
        expires_at,
        project_id: None,
        quota_info: None,
        tags: vec![],
        priority: 0,
        weight: 1.0,
    })
}

pub async fn handle_github_copilot_flow(
    account_name: Option<String>,
) -> Result<LLMAccount, String> {
    println!("[Auth] Starting Device Code flow for GitHub Copilot...");
    let client = Client::new();
    let client_id = "01ab8ac9400c4e429b23"; // Standard GitHub Copilot Plugin Client ID

    // 1. Get device code
    let res_text = client
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&[("client_id", client_id), ("scope", "read:user")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let device_auth: DeviceAuthRes = serde_json::from_str(&res_text).map_err(|e| {
        format!(
            "Failed to parse device code response: {} (raw: {})",
            e, res_text
        )
    })?;

    println!("\n=======================================================");
    println!("Please open this URL in your browser:");
    println!("  {}", device_auth.verification_uri);
    println!("\nAnd enter the following code:");
    println!("  \x1b[1m\x1b[33m{}\x1b[0m", device_auth.user_code);
    println!("=======================================================\n");

    let _ = open::that(&device_auth.verification_uri);

    // 2. Poll for token
    println!("Waiting for authorization...");
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(device_auth.interval.max(3)));

    loop {
        interval.tick().await;
        let token_res_text = client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_auth.device_code),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;

        let token_res: TokenRes =
            serde_json::from_str(&token_res_text).unwrap_or_else(|_| TokenRes {
                access_token: None,
                refresh_token: None,
                expires_in: None,
                error: Some("Parse error".into()),
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
            let name: String = match account_name {
                Some(n) => n,
                None => Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Account Name")
                    .default("GitHub Copilot".into())
                    .interact_text()
                    .map_err(|e| e.to_string())?,
            };

            // Fetch the initial Copilot session token
            println!("[Auth] Authenticated with GitHub. Fetching Copilot session token...");
            let copilot_token_req = client
                .get("https://api.github.com/copilot_internal/v2/token")
                .header("Authorization", format!("token {}", access_token))
                .header("Accept", "application/json")
                .header("User-Agent", "GitHubCopilotChat/0.11.1")
                .send()
                .await;

            let (api_key, expires_at) = match copilot_token_req {
                Ok(res) if res.status().is_success() => {
                    let json_res: serde_json::Value = res.json().await.unwrap_or_default();
                    let token = json_res
                        .get("token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let expires = json_res.get("expires_at").and_then(|v| v.as_i64()); // This is unix timestamp in seconds
                    (token, expires)
                }
                _ => {
                    println!("[Auth] Failed to fetch initial Copilot token. You may not have an active Copilot subscription.");
                    return Err("Failed to fetch initial Copilot token. Ensure you have an active GitHub Copilot subscription.".to_string());
                }
            };

            return Ok(LLMAccount {
                id: Uuid::new_v4().to_string(),
                name,
                enabled: true,
                provider: "github_copilot".to_string(),
                models: vec!["gpt-4o".into(), "gpt-4".into(), "gpt-3.5-turbo".into()],
                models_info: None,
                model_mapping: std::collections::HashMap::new(),
                base_url: "https://api.githubcopilot.com".to_string(),
                api_key,
                refresh_token: Some(access_token), // Store the GitHub OAuth token here!
                expires_at,
                project_id: None,
                quota_info: None,
                tags: vec![],
                priority: 0,
                weight: 1.0,
            });
        }
    }
}

async fn fetch_cloudaicompanion_project(access_token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let res = client
        .post("https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:loadCodeAssist")
        .bearer_auth(access_token)
        .header("User-Agent", "vscode/1.X.X (Antigravity/4.3.0)")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "metadata": { "ideType": "ANTIGRAVITY" }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to request loadCodeAssist: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("loadCodeAssist returned error: {}", res.status()));
    }

    let data: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if let Some(project_id) = data.get("cloudaicompanionProject").and_then(|v| v.as_str()) {
        return Ok(project_id.to_string());
    }

    Err("Could not find cloudaicompanionProject in response".to_string())
}

pub async fn handle_gemini_oauth_flow(account_name: Option<String>) -> Result<LLMAccount, String> {
    println!("[Auth] Starting web login flow to authenticate with Google (Gemini Native)...");
    let client_id = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
    let client_secret = "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";
    let auth_endpoint = "https://accounts.google.com/o/oauth2/v2/auth";
    let token_endpoint = "https://oauth2.googleapis.com/token";
    let scope = "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";

    let (access_token, refresh_token, expires_in) = execute_pkce_flow(
        client_id,
        Some(client_secret),
        auth_endpoint,
        token_endpoint,
        scope,
        8765,
        "http://127.0.0.1:8765/callback",
        "/callback",
        "",
    )
    .await?;
    let expires_at = expires_in.map(|secs| chrono::Utc::now().timestamp() + secs);

    println!("[Auth] Fetching Google Cloud AI Companion Project...");
    let project_id = match fetch_cloudaicompanion_project(&access_token).await {
        Ok(pid) => pid,
        Err(e) => {
            println!("[Auth] Warning: {}", e);
            return Err(
                "Failed to fetch companion project. Your account might not have access."
                    .to_string(),
            );
        }
    };

    println!("[Auth] Successfully bound to project: {}", project_id);
    let base_url = format!("https://us-central1-aiplatform.googleapis.com/v1beta1/projects/{}/locations/us-central1/endpoints/openapi", project_id);

    // Save to Windows Credential Manager for the local proxy
    #[cfg(target_os = "windows")]
    {
        let expiry = expires_at
            .map(|ts| {
                (chrono::Utc::now() + chrono::Duration::seconds(expires_in.unwrap_or(3599)))
                    .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            })
            .unwrap_or_else(|| {
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            });

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
            if crate::services::keyring_auth::write_credential(target_name, payload_str.as_bytes())
            {
                println!(
                    "[Auth] Successfully saved OAuth token to Windows Credential Manager as '{}'",
                    target_name
                );
            } else {
                eprintln!("[Auth] Failed to save token to Windows Credential Manager.");
            }
        }
    }

    let name: String = match account_name {
        Some(n) => n,
        None => Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Account Name")
            .default("Gemini Native (Copilot)".into())
            .interact_text()
            .map_err(|e| e.to_string())?,
    };

    Ok(LLMAccount {
        id: Uuid::new_v4().to_string(),
        name,
        enabled: true,
        provider: "gemini_native".to_string(),
        models: vec![],
        models_info: None,
        model_mapping: std::collections::HashMap::new(),
        base_url,
        api_key: access_token,
        refresh_token,
        expires_at,
        project_id: Some(project_id),
        quota_info: None,
        tags: vec![],
        priority: 0,
        weight: 1.0,
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
        models_info: None,
        model_mapping: std::collections::HashMap::new(),
        base_url,
        api_key,
        refresh_token: None,
        expires_at: None,
        project_id: None,
        quota_info: None,
        tags: vec![],
        priority: 0,
        weight: 1.0,
    })
}

async fn handle_custom_api_key_with_defaults(
    display_name: &str,
    provider: &str,
    default_url: &str,
) -> Result<LLMAccount, String> {
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
        models_info: None,
        model_mapping: std::collections::HashMap::new(),
        base_url,
        api_key,
        refresh_token: None,
        expires_at: None,
        project_id: None,
        quota_info: None,
        tags: vec![],
        priority: 0,
        weight: 1.0,
    })
}

async fn execute_pkce_flow(
    client_id: &str,
    client_secret: Option<&str>,
    auth_endpoint: &str,
    token_endpoint: &str,
    scope: &str,
    port: u16,
    redirect_uri: &str,
    redirect_path: &str,
    extra_auth_params: &str,
) -> Result<(String, Option<String>, Option<i64>), String> {
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
        "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&scope={}&state={}{}",
        auth_endpoint,
        client_id,
        urlencoding::encode(redirect_uri),
        challenge,
        urlencoding::encode(scope),
        state,
        extra_auth_params
    );

    // Start Local Server to receive callback
    let (tx, rx) = oneshot::channel();
    let app_state = std::sync::Arc::new(AppState {
        expected_state: state.clone(),
        tx: std::sync::Mutex::new(Some(tx)),
    });

    let app = Router::new()
        .route(redirect_path, get(handle_callback))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Failed to bind to port {}: {}", port, e))?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // Spawn server task
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // Open Browser
    println!("Opening your browser to log in...");
    println!("{}", auth_url);
    if let Err(e) = open::that(&auth_url) {
        eprintln!(
            "Failed to automatically open browser ({}). Please open the URL manually.",
            e
        );
    }

    // Wait for Callback
    let code = match rx.await {
        Ok(Ok(c)) => {
            let _ = shutdown_tx.send(());
            c
        }
        Ok(Err(e)) => {
            let _ = shutdown_tx.send(());
            return Err(format!("Login failed: {}", e));
        }
        Err(_) => {
            let _ = shutdown_tx.send(());
            return Err("Server channel closed unexpectedly".to_string());
        }
    };

    println!("[Auth] Received callback. Exchanging code for token...");

    // Exchange Code for Token
    let client = Client::new();
    let req = client
        .post(token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded");

    let mut body = format!(
        "grant_type=authorization_code&code={}&client_id={}&code_verifier={}&redirect_uri={}",
        code,
        client_id,
        verifier,
        urlencoding::encode(redirect_uri)
    );

    if let Some(secret) = client_secret {
        body.push_str(&format!("&client_secret={}", secret));
    }

    let token_res_text = req
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read token response: {}", e))?;

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

// -----------------------------------------------------------------------------
// DeepCloud Custom Auth Flow
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DeepcloudCallbackQuery {
    token: Option<String>,
    error: Option<String>,
}

pub async fn handle_deepcloud_flow() -> Result<(), String> {
    let port = 1455;
    let redirect_uri = format!("http://localhost:{}/auth/deepcloud/callback", port);

    // Default to a common local dev port if not specified,
    // deepcloud-ui usually runs on 3000
    let deepcloud_url = "http://localhost:3000/cli/login";

    let login_url = format!(
        "{}?callback={}",
        deepcloud_url,
        urlencoding::encode(&redirect_uri)
    );

    println!("Opening browser to login to DeepCloud: {}", login_url);
    if webbrowser::open(&login_url).is_err() {
        println!("Could not open browser automatically. Please navigate to:");
        println!("{}", login_url);
    }

    let (tx, rx) = oneshot::channel();
    let state = std::sync::Arc::new(AppState {
        expected_state: "".to_string(), // not strictly needed for this simplified flow
        tx: std::sync::Mutex::new(Some(tx)),
    });

    let app = Router::new()
        .route("/auth/deepcloud/callback", get(handle_deepcloud_callback))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;

    let server = axum::serve(listener, app);

    tokio::select! {
        _ = server => {
            Err("Server stopped unexpectedly".into())
        }
        res = rx => {
            let token = res.map_err(|_| "Failed to receive token".to_string())??;

            // Save to keyring
            let entry = keyring::Entry::new("deepcloud_hub", "access_token")
                .map_err(|e| format!("Failed to access keyring: {}", e))?;

            entry.set_password(&token)
                .map_err(|e| format!("Failed to save token to keyring: {}", e))?;

            println!("✅ Successfully logged into DeepCloud and saved credentials securely.");
            Ok(())
        }
    }
}

async fn handle_deepcloud_callback(
    Query(query): Query<DeepcloudCallbackQuery>,
    State(state): State<std::sync::Arc<AppState>>,
) -> impl IntoResponse {
    let mut tx_guard = state.tx.lock().unwrap();
    if let Some(tx) = tx_guard.take() {
        if let Some(err) = query.error {
            let _ = tx.send(Err(err));
            return Html("<html><body><h1>Login Failed</h1><p>Check the terminal.</p></body><script>window.close()</script></html>");
        }

        if let Some(token) = query.token {
            let _ = tx.send(Ok(token));
            return Html("<html><body><h1>Login Successful!</h1><p>You can close this tab and return to the terminal.</p></body><script>window.close()</script></html>");
        }

        let _ = tx.send(Err("No token found in callback".into()));
        Html("<html><body><h1>Login Failed</h1><p>No token found.</p></body><script>window.close()</script></html>")
    } else {
        Html("<html><body><p>Login already processed. You can close this tab.</p></body><script>window.close()</script></html>")
    }
}

pub fn get_deepcloud_token() -> Result<String, String> {
    let entry = keyring::Entry::new("deepcloud_hub", "access_token")
        .map_err(|e| format!("Failed to access keyring: {}", e))?;

    entry
        .get_password()
        .map_err(|e| format!("Token not found or keyring error: {}", e))
}

pub fn clear_deepcloud_token() -> Result<(), String> {
    let entry = keyring::Entry::new("deepcloud_hub", "access_token")
        .map_err(|e| format!("Failed to access keyring: {}", e))?;

    entry
        .delete_credential()
        .map_err(|e| format!("Failed to delete token from keyring: {}", e))
}

pub async fn handle_claude_oauth_flow(
    session_key_opt: Option<String>,
) -> Result<LLMAccount, String> {
    println!("[Auth] Starting OAuth flow for Claude Desktop Official...");
    let session_key: String = match session_key_opt {
        Some(k) => k,
        None => Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Please enter your claude.ai sessionKey cookie")
            .interact_text()
            .map_err(|e| e.to_string())?,
    };

    let client = reqwest::Client::new();

    // 1. Get Org UUID
    println!("[Auth] Fetching organization info...");
    let orgs_resp = client
        .get("https://claude.ai/api/organizations")
        .header("Cookie", format!("sessionKey={}", session_key))
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !orgs_resp.status().is_success() {
        return Err(format!(
            "Failed to fetch organizations. Status: {}",
            orgs_resp.status()
        ));
    }

    let orgs: Vec<serde_json::Value> = orgs_resp.json().await.map_err(|e| e.to_string())?;
    let org_uuid = orgs
        .first()
        .and_then(|o| o.get("uuid"))
        .and_then(|u| u.as_str())
        .ok_or("No organization found")?;

    // 2. PKCE params
    let mut verifier_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut verifier_bytes);
    let verifier = b64url(&verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = b64url(&hasher.finalize());

    let mut state_bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut state_bytes);
    let state = b64url(&state_bytes);

    // 3. Get Auth Code
    println!("[Auth] Getting authorization code...");
    let client_id = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
    let redirect_uri = "https://platform.claude.com/oauth/code/callback";
    let scope =
        "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

    let req_body = serde_json::json!({
        "response_type": "code",
        "client_id": client_id,
        "organization_uuid": org_uuid,
        "redirect_uri": redirect_uri,
        "scope": scope,
        "state": state,
        "code_challenge": challenge,
        "code_challenge_method": "S256"
    });

    let auth_resp = client
        .post(&format!(
            "https://claude.ai/v1/oauth/{}/authorize",
            org_uuid
        ))
        .header("Cookie", format!("sessionKey={}", session_key))
        .header("User-Agent", "Mozilla/5.0")
        .header("Content-Type", "application/json")
        .json(&req_body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !auth_resp.status().is_success() {
        return Err(format!(
            "Failed to get authorization code. Status: {}",
            auth_resp.status()
        ));
    }

    let auth_result: serde_json::Value = auth_resp.json().await.map_err(|e| e.to_string())?;
    let returned_redirect = auth_result
        .get("redirect_uri")
        .and_then(|r| r.as_str())
        .ok_or("No redirect_uri in response")?;

    // Parse code from redirect URI
    let parsed_url = reqwest::Url::parse(returned_redirect).map_err(|e| e.to_string())?;
    let mut auth_code = None;
    let mut returned_state = None;
    for (k, v) in parsed_url.query_pairs() {
        if k == "code" {
            auth_code = Some(v.into_owned());
        } else if k == "state" {
            returned_state = Some(v.into_owned());
        }
    }

    let code = auth_code.ok_or("No code found in redirect URI")?;
    let full_code = if let Some(s) = returned_state {
        format!("{}#{}", code, s)
    } else {
        code
    };

    // 4. Exchange Token
    println!("[Auth] Exchanging code for token...");
    let token_body = format!(
        "grant_type=authorization_code&code={}&client_id={}&code_verifier={}&redirect_uri={}",
        urlencoding::encode(&full_code),
        client_id,
        verifier,
        urlencoding::encode(redirect_uri)
    );

    let token_resp = client
        .post("https://platform.claude.com/v1/oauth/token")
        .header("User-Agent", "Mozilla/5.0")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(token_body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !token_resp.status().is_success() {
        return Err(format!(
            "Failed to exchange token. Status: {}",
            token_resp.status()
        ));
    }

    let token_data: TokenRes = token_resp.json().await.map_err(|e| e.to_string())?;
    let access_token = token_data.access_token.ok_or("No access_token returned")?;
    let expires_in = token_data.expires_in;
    let refresh_token = token_data.refresh_token;

    let expires_at = expires_in.map(|secs| chrono::Utc::now().timestamp() + secs);

    // Save to Windows Credential Manager
    #[cfg(target_os = "windows")]
    {
        let expiry = expires_at
            .map(|ts| {
                (chrono::Utc::now() + chrono::Duration::seconds(expires_in.unwrap_or(3599)))
                    .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            })
            .unwrap_or_else(|| {
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            });

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
            let target_name = "claude:antigravity";
            if crate::services::keyring_auth::write_credential(target_name, payload_str.as_bytes())
            {
                println!(
                    "[Auth] Successfully saved OAuth token to Windows Credential Manager as '{}'",
                    target_name
                );
            } else {
                eprintln!("[Auth] Failed to save token to Windows Credential Manager.");
            }
        }
    }

    let name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Account Name")
        .default("Claude Desktop Official".into())
        .interact_text()
        .map_err(|e| e.to_string())?;

    Ok(LLMAccount {
        id: Uuid::new_v4().to_string(),
        name,
        enabled: true,
        provider: "anthropic".to_string(),
        models: vec![
            "claude-3-7-sonnet-20250219".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-opus-20240229".to_string(),
        ],
        models_info: None,
        model_mapping: std::collections::HashMap::new(),
        base_url: "https://api.anthropic.com".to_string(),
        api_key: access_token,
        refresh_token,
        expires_at,
        project_id: None,
        quota_info: None,
        tags: vec![],
        priority: 0,
        weight: 1.0,
    })
}

pub async fn refresh_account_token_if_needed(
    account: &mut crate::config::llm_settings::LLMAccount,
    http_client: &reqwest::Client,
) -> bool {
    let mut needs_save = false;
    if let (Some(expires_at), Some(refresh_token)) = (account.expires_at, &account.refresh_token) {
        if chrono::Utc::now().timestamp() + 300 >= expires_at {
            tracing::info!("Token expired or expiring soon, refreshing...");

            if account.provider == "github_copilot" {
                let req = http_client
                    .get("https://api.github.com/copilot_internal/v2/token")
                    .header("Authorization", format!("token {}", refresh_token))
                    .header("Accept", "application/json")
                    .header("User-Agent", "GitHubCopilotChat/0.11.1");

                if let Ok(res) = req.send().await {
                    if let Ok(json_res) = res.json::<serde_json::Value>().await {
                        if let Some(token) = json_res.get("token").and_then(|v| v.as_str()) {
                            account.api_key = token.to_string();
                            if let Some(expires_in) =
                                json_res.get("expires_at").and_then(|v| v.as_i64())
                            {
                                account.expires_at = Some(expires_in);
                            }
                            needs_save = true;
                        }
                    }
                }
            } else {
                let token_url =
                    if account.provider == "openai" || account.provider == "openai_native" {
                        "https://auth.openai.com/oauth/token"
                    } else if account.provider == "gemini" || account.provider == "gemini_native" {
                        "https://oauth2.googleapis.com/token"
                    } else {
                        ""
                    };

                if !token_url.is_empty() {
                    let client_id = if account.provider == "openai"
                        || account.provider == "openai_native"
                    {
                        "app_EMoamEEZ73f0CkXaXp7hrann"
                    } else if account.provider == "gemini_native" {
                        "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
                    } else {
                        "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com"
                    };

                    let client_secret = if account.provider == "gemini" {
                        Some("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")
                    } else if account.provider == "gemini_native" {
                        Some("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")
                    } else {
                        None
                    };

                    let req = http_client
                        .post(token_url)
                        .header("Content-Type", "application/x-www-form-urlencoded");

                    let mut body_str = format!(
                        "grant_type=refresh_token&refresh_token={}&client_id={}",
                        refresh_token, client_id
                    );
                    if let Some(secret) = client_secret {
                        body_str.push_str(&format!("&client_secret={}", secret));
                    }

                    if let Ok(res) = req.body(body_str).send().await {
                        if let Ok(res_json) = res.json::<serde_json::Value>().await {
                            if let Some(access_token) =
                                res_json.get("access_token").and_then(|v| v.as_str())
                            {
                                account.api_key = access_token.to_string();

                                if let Some(new_refresh) =
                                    res_json.get("refresh_token").and_then(|v| v.as_str())
                                {
                                    account.refresh_token = Some(new_refresh.to_string());
                                }

                                if let Some(expires_in) =
                                    res_json.get("expires_in").and_then(|v| v.as_i64())
                                {
                                    account.expires_at =
                                        Some(chrono::Utc::now().timestamp() + expires_in);
                                }
                                needs_save = true;
                            }
                        }
                    }
                }
            }
        }
    }
    needs_save
}
