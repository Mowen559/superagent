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


use super::*;

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

pub(crate) async fn handle_deepcloud_sso_flow() -> Result<LLMAccount, String> {
    println!("[Auth] Starting web login flow to authenticate with DeepCloud SSO...");
    // For MVP, just return an error and instruct user to use custom key
    Err("DeepCloud SSO is currently undergoing migration, please use Custom API Key".to_string())
}

pub(crate) async fn handle_custom_api_key() -> Result<LLMAccount, String> {
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
        model_capabilities: None,
    })
}

pub(crate) async fn handle_custom_api_key_with_defaults(
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
        model_capabilities: None,
    })
}

