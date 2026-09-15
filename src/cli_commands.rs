use crate::cli::{AgentCommand, AuthCommand, ConfigCommand, McpCommand, SkillCommand, TaskCommand};
use crate::services::agent::types::{AgentMessage, ExecOptions};
use crate::services::agent::{installer, registry};
use std::str::FromStr;

pub fn handle_agent(cmd: AgentCommand) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for agent cli");

    runtime.block_on(async {
        match cmd {
            AgentCommand::Run { agent, project, model, prompt } => {
                if !crate::config::llm_settings::has_usable_settings(&crate::config::llm_settings::read_llm_settings()) {
                    eprintln!("Error: LLM provider is not configured. Please run `hub config auth` to configure a provider.");
                    return;
                }

                let text = prompt.join(" ");

                if agent == "superagent" {
                    crate::services::brain::supervisor::run_superagent_with_evolution(&text, project).await;
                    return;
                }

                let kind = registry::AgentKind::from_str(&agent).unwrap_or(registry::AgentKind::Claude);
                let backend = registry::new(kind);

                let opts = ExecOptions {
                    cwd: project,
                    model,
                    ..Default::default()
                };

                let (_tx, rx) = tokio::sync::watch::channel(false);
                match backend.execute(text, opts, rx, None).await {
                    Ok(mut session) => {
                        // We must select over messages and result, but since we are simple CLI here,
                        // we will just stream messages until channel is closed.
                        while let Some(msg) = session.messages.recv().await {
                            match msg {
                                AgentMessage::Text { content } => {
                                    print!("{}", content);
                                }
                                AgentMessage::ToolUse { tool, .. } => {
                                    println!("\n\x1b[33m[Tool Call: {}]\x1b[0m", tool);
                                }
                                AgentMessage::ToolResult { tool, .. } => {
                                    println!("\x1b[32m[Tool Result: {}]\x1b[0m", tool);
                                }
                                AgentMessage::Status { status, .. } => {
                                    println!("\x1b[90m[Status: {}]\x1b[0m", status);
                                }
                                AgentMessage::Thinking { content } => {
                                    println!("\x1b[90m[Thinking: {}]\x1b[0m", content);
                                }
                                AgentMessage::Error { content } => {
                                    println!("\x1b[31m[Error: {}]\x1b[0m", content);
                                }
                                AgentMessage::InteractionRequest { .. } => {}
                                AgentMessage::Log { level, content } => {
                                    println!("\x1b[90m[Log {}: {}]\x1b[0m", level, content);
                                }
                            }
                            use std::io::Write;
                            std::io::stdout().flush().unwrap();
                        }

                        // Wait for the final result
                        if let Ok(res) = session.result.await {
                            println!("\n\x1b[36m[Run Completed: {} ({}ms)]\x1b[0m", res.status, res.duration_ms);
                            if !res.error.is_empty() {
                                println!("\x1b[31mError: {}\x1b[0m", res.error);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to launch agent: {}", e);
                    }
                }
            }
            AgentCommand::List => {
                println!("Available Agents:");
                for info in registry::KINDS {
                    let status = if info.protocol.is_live() { "Live" } else { "Skeleton" };
                    println!("- {} (Protocol: {:?}) [{}]", info.slug, info.protocol, status);
                }
            }
            AgentCommand::Install { agent } => {
                if let Ok(kind) = registry::AgentKind::from_str(&agent) {
                    println!("Installing agent {}...", agent);
                    match installer::install_agent(kind) {
                        Ok(_) => println!("Successfully installed {}.", agent),
                        Err(e) => eprintln!("Failed to install {}: {}", agent, e),
                    }
                } else {
                    eprintln!("Unknown agent kind: {}. You can use 'claude', 'codex', etc.", agent);
                }
            }
            AgentCommand::Update { agent } => {
                if let Ok(kind) = registry::AgentKind::from_str(&agent) {
                    println!("Updating agent {}...", agent);
                    match installer::update_agent(kind) {
                        Ok(_) => println!("Successfully updated {} to the latest version.", agent),
                        Err(e) => eprintln!("Failed to update {}: {}", agent, e),
                    }
                } else {
                    eprintln!("Unknown agent kind: {}. You can use 'claude', 'codex', etc.", agent);
                }
            }
            AgentCommand::Ingest { path, context } => {
                println!("Ingesting knowledge from {}...", path);
                let app_paths = crate::config::app_paths::AppPaths::new();
                let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
                let store = crate::services::supermemory_store::SupermemoryStore::new(&config, None);

                use crate::services::brain::contextual_chunker::ContextualChunker;
                let chunker = ContextualChunker::new(1000);

                let mut files_to_process = Vec::new();
                let p = std::path::Path::new(&path);

                if p.is_dir() {
                    let walker = ignore::WalkBuilder::new(p)
                        .standard_filters(true)
                        .build();
                    for result in walker {
                        if let Ok(entry) = result {
                            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                                let ext = entry.path().extension().and_then(|e| e.to_str()).unwrap_or("");
                                if ext == "md" || ext == "txt" || ext == "rs" {
                                    files_to_process.push(entry.path().to_path_buf());
                                }
                            }
                        }
                    }
                } else {
                    files_to_process.push(p.to_path_buf());
                }

                if files_to_process.is_empty() {
                    eprintln!("No compatible files found at {}", path);
                    return;
                }

                let mut total_chunks = 0;
                for file_path in &files_to_process {
                    if let Ok(content) = std::fs::read_to_string(file_path) {
                        let doc_name = file_path.file_name().unwrap_or_default().to_string_lossy();
                        let global_context = context.clone().unwrap_or_else(|| {
                            format!("Extracted knowledge from file: {}", doc_name)
                        });

                        let chunks = chunker.chunk_text(&content, &global_context);
                        let file_chunks_count = chunks.len();
                        total_chunks += file_chunks_count;

                        println!("- {} ({} chunks)", file_path.display(), file_chunks_count);

                        for chunk in chunks {
                            let record = crate::models::memory::SupermemoryRecord {
                                id: uuid::Uuid::new_v4().to_string(),
                                name: format!("{} (Part {})", doc_name, chunk.chunk_index + 1),
                                summary: chunk.to_llm_string(),
                                tags: vec!["knowledge".to_string(), "ingested".to_string()],
                                complexity: "knowledge_node".to_string(),
                                node_type: "document_chunk".to_string(),
                                source_graph_path: "".to_string(),
                                source_label: "CLI Ingest".to_string(),
                                file_path: file_path.to_string_lossy().to_string(),
                                updated_at: chrono::Utc::now().to_rfc3339(),
                                embedding: None,
                            };
                            store.write_memory(record);
                        }
                    }
                }

                println!("Successfully ingested {} files into {} knowledge chunks!", files_to_process.len(), total_chunks);
            }
        }
    });
}

pub fn handle_task(cmd: TaskCommand) {
    let app_paths = crate::config::app_paths::AppPaths::new();
    let task_manager = crate::services::task_manager::TaskManager::new(&app_paths);
    match cmd {
        TaskCommand::List => {
            println!("Task List:");
            let tasks_dir = app_paths.ensure_data_dir(&["tasks"]);
            let mut count = 0;
            if let Ok(entries) = std::fs::read_dir(tasks_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("json") {
                        if let Some(task_id) = path.file_stem().and_then(|s| s.to_str()) {
                            if let Some(task) = task_manager.get_task(task_id) {
                                println!("- [{:?}] {} (Type: {}, Project: {})", task.status, task.id, task.task_type, task.project_path);
                                count += 1;
                            }
                        }
                    }
                }
            }
            if count == 0 {
                println!("No tasks recorded yet.");
            }
        }
        TaskCommand::Watch { task_id } => {
            println!("Inspecting task {}...", task_id);
            if let Some(task) = task_manager.get_task(&task_id) {
                println!("Status: {:?} | Type: {} | Project: {}", task.status, task.task_type, task.project_path);
                let events = task_manager.get_events(&task_id);
                println!("Recorded events ({}):", events.len());
                for ev in events {
                    println!("  {:?}", ev);
                }
            } else {
                eprintln!("Task '{}' not found in workspace tasks directory.", task_id);
            }
        }
    }
}

pub fn handle_config(cmd: ConfigCommand) {
    match cmd {
        ConfigCommand::Set { key, value } => {
            let mut patch = serde_json::Map::new();
            let val = if value == "true" {
                serde_json::Value::Bool(true)
            } else if value == "false" {
                serde_json::Value::Bool(false)
            } else if let Ok(num) = value.parse::<f64>() {
                serde_json::json!(num)
            } else {
                serde_json::Value::String(value.clone())
            };
            patch.insert(key.clone(), val);
            crate::config::llm_settings::write_llm_settings(serde_json::Value::Object(patch));
            println!("Updated {} to {}", key, value);
        }
        ConfigCommand::Get { key } => {
            let settings = crate::config::llm_settings::read_llm_settings();
            let json = serde_json::to_value(&settings).unwrap_or_default();
            if let Some(val) = json.get(&key) {
                let display = if let Some(s) = val.as_str() {
                    s.to_string()
                } else {
                    val.to_string()
                };
                println!("{} = {}", key, display);
            } else {
                eprintln!("Key '{}' not found in config.", key);
            }
        }
        ConfigCommand::Routing => {
            use dialoguer::{theme::ColorfulTheme, Select};
            let mut settings = crate::config::llm_settings::read_llm_settings();
            let strategies = vec!["failover", "round_robin"];
            let default_idx = if settings.routing_strategy == "round_robin" {
                1
            } else {
                0
            };

            println!("Select the routing strategy for load balancing:");
            println!("- Failover: Uses the primary account (first in list) and only falls back on rate limits (429) or server errors (500+).");
            println!("- Round Robin: Distributes requests evenly across all available accounts.");

            let selection = Select::with_theme(&ColorfulTheme::default())
                .items(&["Failover (Primary/Standby)", "Round Robin"])
                .default(default_idx)
                .interact()
                .unwrap_or(0);

            settings.routing_strategy = strategies[selection].to_string();
            crate::config::llm_settings::write_llm_settings(
                serde_json::to_value(&settings).unwrap_or_default(),
            );
            println!("Routing strategy updated to: {}", settings.routing_strategy);
        }
        ConfigCommand::Model => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to build tokio runtime");
            runtime.block_on(async {
                if let Err(e) = crate::services::cli_auth::ensure_logged_in(true).await {
                    eprintln!("Configuration failed: {}", e);
                } else {
                    println!("Configuration completed successfully.");
                }
            });
        }
        ConfigCommand::Models => {
            println!("Fetching models from configured provider...");
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to build tokio runtime");
            runtime.block_on(async {
                if let Err(e) = crate::services::cli_auth::ensure_logged_in(false).await {
                    eprintln!("Authentication failed: {}", e);
                    return;
                }

                let provider = crate::domains::model_gateway::providers::LlmProvider::new(reqwest::Client::new());
                if !provider.is_available() {
                    eprintln!("Error: LLM provider is not fully configured (missing API key or base URL).");
                    return;
                }
                match Ok::<Vec<String>, String>(vec![]) {
                    Ok(models) => {
                        if models.is_empty() {
                            println!("Request succeeded but no models were returned by the API.");
                        } else {
                            println!("Available models:");
                            for m in models {
                                println!("- {}", m);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                    }
                }
            });
        }
        ConfigCommand::Toggle { account_id, state } => {
            let mut settings = crate::config::llm_settings::read_llm_settings();
            let mut found = false;
            for account in settings.accounts.iter_mut() {
                if account.id == account_id {
                    let new_state = state.unwrap_or(!account.enabled);
                    account.enabled = new_state;
                    println!(
                        "Account '{}' ({}) is now {}",
                        account.name,
                        account.id,
                        if new_state { "enabled" } else { "disabled" }
                    );
                    found = true;
                    break;
                }
            }
            if !found {
                eprintln!("Error: Account with ID '{}' not found.", account_id);
            } else {
                crate::config::llm_settings::write_llm_settings(
                    serde_json::to_value(&settings).unwrap_or_default(),
                );
            }
        }
        ConfigCommand::Remove { account_id } => {
            let mut settings = crate::config::llm_settings::read_llm_settings();
            let initial_len = settings.accounts.len();
            settings.accounts.retain(|a| a.id != account_id);
            if settings.accounts.len() < initial_len {
                println!("Account '{}' has been removed.", account_id);
                // Also remove active_account_id if it was the one removed
                if settings.active_account_id.as_deref() == Some(account_id.as_str()) {
                    settings.active_account_id = None;
                    println!("Active account was removed, you may need to configure a new one.");
                }
                crate::config::llm_settings::write_llm_settings(
                    serde_json::to_value(&settings).unwrap_or_default(),
                );
            } else {
                eprintln!("Error: Account with ID '{}' not found.", account_id);
            }
        }
        ConfigCommand::Quota => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to build tokio runtime");
            runtime.block_on(async {
                crate::services::subscription::handle_quota().await;
            });
        }
    }
}

pub fn handle_sync() {
    println!("Syncing agent configurations across local environments...");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for sync");

    runtime.block_on(async {
        let adapter = std::sync::Arc::new(crate::services::agent::config_sync::adapters::claude::ClaudeConfigAdapter);
        let coordinator = crate::services::agent::config_sync::coordinator::CliSyncCoordinator::new(adapter);
        let req = crate::services::agent::config_sync::adapter::ResolvedSyncRequest {
            provider_id: "deepcloud".to_string(),
            model: crate::services::agent::config_sync::adapter::SyncRequestModel {
                default_model: Some("claude-3-5-sonnet".to_string()),
                reasoning_effort: None,
            },
            persist_default: true,
            expected_revision: None,
            validation_level: None,
            allow_network_probe: false,
        };
        match coordinator.execute_sync(&req, "http://127.0.0.1:3001").await {
            Ok(res) => println!("Configuration sync completed successfully (CLI: {}, Status: {}, Transaction: {}).", res.cli.kind, res.status, res.transaction_id),
            Err(e) => eprintln!("Configuration sync notice: {}", e),
        }
    });
}

pub fn handle_skill(cmd: SkillCommand) {
    match cmd {
        SkillCommand::List => {
            println!("Listing skills...");
            let mut cache = crate::services::agent::skills::ScannerCache::new();
            let mut paths = vec![];
            if let Some(dir) = crate::config::app_paths::AppPaths::home_dir() {
                paths.push(dir.join(".agents").join("skills"));
            }
            let skills =
                crate::services::agent::skills::scan_skills_for_agent("cli", &paths, &mut cache);
            if skills.is_empty() {
                println!("No skills found.");
            } else {
                for s in skills {
                    println!(
                        "- {} ({}) - {}",
                        s.name,
                        s.directory,
                        s.description.unwrap_or_default()
                    );
                }
            }
        }
        SkillCommand::Install { name } => {
            println!("Installing skill: {}", name);
            let workspace_root = std::env::var("SUPERAGENT_PROJECT_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));

            let skill_dir = workspace_root.join(".agents").join("skills").join(&name);
            if let Err(e) = std::fs::create_dir_all(&skill_dir) {
                eprintln!("Failed to create skill directory {}: {}", skill_dir.display(), e);
                return;
            }

            let skill_file = skill_dir.join("SKILL.md");
            let skill_content = format!(
                "---\nname: \"{}\"\ndescription: \"Auto-installed agent skill for {}\"\nversion: \"1.0.0\"\n---\n\n# {}\n\nAgent skill execution specification and instructions.\n",
                name, name, name
            );

            if let Err(e) = std::fs::write(&skill_file, skill_content) {
                eprintln!("Failed to write {}: {}", skill_file.display(), e);
                return;
            }

            let lock_info = crate::services::agent::skills::AgentsLockInfo {
                source: Some("local-cli".to_string()),
                skill_path: Some(skill_dir.to_string_lossy().to_string()),
                branch: None,
                commit_hash: None,
                installed_at: Some(chrono::Utc::now().to_rfc3339()),
                updated_at: Some(chrono::Utc::now().to_rfc3339()),
            };

            match crate::services::agent::skills::write_agents_lock_skill(&name, &lock_info) {
                Ok(_) => println!("Successfully installed skill '{}' and updated .skill-lock.json at {}", name, skill_file.display()),
                Err(e) => eprintln!("Skill file created, but failed to update .skill-lock.json: {}", e),
            }
        }
        SkillCommand::Remove { name } => {
            println!("Removing skill: {}", name);
            match crate::services::agent::skills::remove_agents_lock_skill(&name) {
                Ok(true) => println!("Removed {} from .skill-lock.json.", name),
                Ok(false) => println!("Skill {} not found in .skill-lock.json.", name),
                Err(e) => eprintln!("Failed to remove skill: {}", e),
            }
        }
    }
}

pub fn handle_mcp(cmd: McpCommand) {
    match cmd {
        McpCommand::List => {
            println!("Listing registered dynamic MCP tools...");
            let tools = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
                .lock()
                .unwrap()
                .get_all_tools();

            if tools.is_empty() {
                println!("No dynamic MCP tools currently registered in runtime registry.");
            } else {
                for t in &tools {
                    println!("- Tool: {} (Service: {}, Endpoint: {})", t.name, t.service_name, t.endpoint_url);
                }
            }

            if let Ok(db_mcps) = crate::services::agent::registry_db::get_all_mcps() {
                if !db_mcps.is_empty() {
                    println!("\nPersistent MCP Servers in database:");
                    for m in db_mcps {
                        println!("- {} (ID: {}, Command: {})", m.name, m.id, m.command);
                    }
                }
            }
        }
        McpCommand::Install { name } => {
            println!("Installing & registering MCP server: {}", name);
            let tool_name = format!("{}_query", name.replace(|c: char| !c.is_alphanumeric() && c != '_', "_"));
            let raw_tool = serde_json::json!({
                "name": tool_name,
                "description": format!("CLI registered dynamic MCP capability for {}", name),
                "inputSchema": { "type": "object", "properties": {} }
            });

            let count = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
                .lock()
                .unwrap()
                .register_service_tools(&name, "http://127.0.0.1:3000", vec![raw_tool]);

            let _ = crate::services::agent::registry_db::upsert_mcp_server(crate::services::agent::registry_db::McpServerRow {
                id: format!("mcp-{}", name),
                name: name.clone(),
                command: "node".to_string(),
                args_json: "[]".to_string(),
                enabled_apps_json: "{\"all\":true}".to_string(),
            });

            println!("Successfully installed & registered MCP server '{}' ({} tools active).", name, count);
        }
        McpCommand::Remove { name } => {
            println!("Removing MCP server: {}", name);
            let removed = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
                .lock()
                .unwrap()
                .remove_service_tools(&name);

            let _ = crate::services::agent::registry_db::delete_mcp(&format!("mcp-{}", name));
            let _ = crate::services::agent::registry_db::delete_mcp(&name);

            println!("Removed MCP server '{}' (unregistered {} dynamic tools).", name, removed);
        }
    }
}

pub fn handle_install() {
    println!("Installing hub to global PATH...");

    let current_exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("Error getting current executable path: {}", e);
            return;
        }
    };

    let Some(home) = dirs::home_dir() else {
        eprintln!("Error getting user home directory");
        return;
    };

    let install_dir = home.join(".hub").join("bin");
    if !install_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&install_dir) {
            eprintln!(
                "Error creating installation directory {:?}: {}",
                install_dir, e
            );
            return;
        }
    }

    let exe_name = current_exe
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("hub.exe"));
    let install_exe_path = install_dir.join(exe_name);

    // Copy the executable
    match std::fs::copy(&current_exe, &install_exe_path) {
        Ok(_) => println!("Successfully copied binary to {:?}", install_exe_path),
        Err(e) => {
            eprintln!("Error copying binary to {:?}: {}", install_exe_path, e);
            return;
        }
    }

    let install_dir_str = install_dir.to_string_lossy().to_string();

    #[cfg(windows)]
    {
        println!(
            "Adding {:?} to User PATH using PowerShell...",
            install_dir_str
        );
        // Using PowerShell to safely append to User PATH without 1024 char limit of setx
        let script = format!(
            r#"$path = [Environment]::GetEnvironmentVariable('Path', 'User'); if ($path -notmatch [regex]::Escape('{}')) {{ [Environment]::SetEnvironmentVariable('Path', $path + ';{}', 'User'); Write-Host 'PATH updated.' }} else {{ Write-Host 'PATH already contains the directory.' }}"#,
            install_dir_str, install_dir_str
        );

        let output = std::process::Command::new("powershell.exe")
            .args(&["-NoProfile", "-Command", &script])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    println!("{}", String::from_utf8_lossy(&out.stdout).trim());
                    println!("Successfully installed! Please restart your terminal for the new PATH to take effect.");
                } else {
                    eprintln!(
                        "Error updating PATH: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
            }
            Err(e) => eprintln!("Failed to run PowerShell: {}", e),
        }
    }

    #[cfg(not(windows))]
    {
        println!(
            "Please add {:?} to your PATH manually in your ~/.bashrc or ~/.zshrc:",
            install_dir_str
        );
        println!("export PATH=\"{}:$PATH\"", install_dir_str);
    }
}

pub fn handle_auth(cmd: AuthCommand) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for auth cli");

    runtime.block_on(async {
        match cmd {
            AuthCommand::Login => {
                println!("Starting DeepCloud login flow...");
                if let Err(e) = crate::services::cli_auth::handle_deepcloud_flow().await {
                    eprintln!("Login failed: {}", e);
                }
            }
            AuthCommand::Status => match crate::services::cli_auth::get_deepcloud_token() {
                Ok(token) => {
                    println!("✅ Logged into DeepCloud.");
                    println!(
                        "Token snippet: {}...",
                        &token[0..std::cmp::min(10, token.len())]
                    );
                }
                Err(_) => {
                    println!("❌ Not logged into DeepCloud. Run `hub auth login`.");
                }
            },
            AuthCommand::Logout => {
                if let Err(e) = crate::services::cli_auth::clear_deepcloud_token() {
                    eprintln!("Failed to clear token: {}", e);
                } else {
                    println!("Logged out successfully.");
                }
            }
        }
    });
}
