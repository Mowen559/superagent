import re

content = open('D:/mowen/deepcloud/super-agent/hub-rs/src/services/cli_auth.rs', encoding='utf-8').read()

new_ensure_logged_in = """pub async fn ensure_logged_in() -> Result<(), String> {
    let mut settings = llm_settings::read_llm_settings();

    // If an account is active and has credentials, we are good.
    if settings.active_account_id.is_some() && !settings.api_key.is_empty() {
        return Ok(());
    }

    println!("\\n[Auth] No active session found.");
    
    // Use dialoguer to present options
    let mut options = vec![];
    for acc in &settings.accounts {
        options.push(format!("Use existing: {} ({})", acc.name, acc.provider));
    }
    options.push("Add New Provider...".to_string());
    
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

    println!("[Auth] \\x1b[32mLogin successful!\\x1b[0m Your session has been saved.\\n");
    Ok(())
}
"""

start_str = "pub async fn ensure_logged_in() -> Result<(), String> {"
end_str = "    Ok(())\n}\n"

start_idx = content.find(start_str)
end_idx = content.find(end_str, start_idx) + len(end_str)

new_content = content[:start_idx] + new_ensure_logged_in + content[end_idx:]

with open('D:/mowen/deepcloud/super-agent/hub-rs/src/services/cli_auth.rs', 'w', encoding='utf-8') as f:
    f.write(new_content)

print("Updated cli_auth.rs")
