use crate::config::llm_settings::{self, LLMAccount};
use dialoguer::{theme::ColorfulTheme, Input};
use reqwest::Client;
use serde::Deserialize;
use uuid::Uuid;


use super::server::execute_pkce_flow;

#[derive(Debug, Deserialize)]
pub(crate) struct TokenRes {
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_in: Option<i64>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeviceAuthRes {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    pub(crate) expires_in: i64,
    pub(crate) interval: u64,
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
        model_capabilities: None,
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
        model_capabilities: None,
            });
        }
    }
}

pub(crate) async fn fetch_cloudaicompanion_project(access_token: &str) -> Result<String, String> {
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
    let (project_id, base_url) = match fetch_cloudaicompanion_project(&access_token).await {
        Ok(pid) => {
            println!("[Auth] Successfully bound to project: {}", pid);
            let url = format!(
                "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/{}/locations/us-central1/endpoints/openapi",
                pid
            );
            (Some(pid), url)
        }
        Err(e) => {
            println!("[Auth] Note: Companion project not available ({}), falling back to direct Generative AI endpoint.", e);
            (None, "https://generativelanguage.googleapis.com/v1beta".to_string())
        }
    };

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
        project_id,
        quota_info: None,
        tags: vec![],
        priority: 0,
        weight: 1.0,
        model_capabilities: None,
    })
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
    let (verifier, challenge, state) = crate::shared_kernel::crypto::generate_pkce_params();

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
        model_capabilities: None,
    })
}

pub async fn force_refresh_account_token(
    account: &mut crate::config::llm_settings::LLMAccount,
    http_client: &reqwest::Client,
) -> bool {
    let refresh_token = match &account.refresh_token {
        Some(rt) if !rt.is_empty() => rt.clone(),
        _ => return false,
    };

    let mut needs_save = false;
    tracing::info!("[Auth] Executing OAuth token refresh for provider '{}', account '{}'", account.provider, account.id);

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
            } else if account.provider == "anthropic" || account.provider == "claude" {
                "https://platform.claude.com/v1/oauth/token"
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
            } else if account.provider == "anthropic" || account.provider == "claude" {
                "9d1c250a-e61b-44d9-88ed-5944d1962f5e"
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

            let mut req = http_client
                .post(token_url)
                .header("Content-Type", "application/x-www-form-urlencoded");

            if account.provider == "anthropic" || account.provider == "claude" {
                req = req.header("User-Agent", "Mozilla/5.0");
            }

            let mut body_str = format!(
                "grant_type=refresh_token&refresh_token={}&client_id={}",
                urlencoding::encode(&refresh_token),
                client_id
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

                        #[cfg(target_os = "windows")]
                        if account.provider == "anthropic" || account.provider == "claude" {
                            let _ = keyring::Entry::new("claude_desktop_access_token", "default")
                                .and_then(|e| e.set_password(&account.api_key).map_err(|err| keyring::Error::PlatformFailure(err.into())));
                        }
                    }
                }
            }
        }
    }
    needs_save
}

pub async fn refresh_account_token_if_needed(
    account: &mut crate::config::llm_settings::LLMAccount,
    http_client: &reqwest::Client,
) -> bool {
    if let (Some(expires_at), Some(_)) = (account.expires_at, &account.refresh_token) {
        let is_expired = if expires_at > 100_000_000_000 {
            chrono::Utc::now().timestamp_millis() + 300_000 >= expires_at
        } else {
            chrono::Utc::now().timestamp() + 300 >= expires_at
        };
        if is_expired {
            tracing::info!("Token expired or expiring soon, refreshing...");
            return force_refresh_account_token(account, http_client).await;
        }
    }
    false
}
