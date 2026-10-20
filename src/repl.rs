use crate::domains::model_gateway::providers::LlmProvider;
use reqwest::Client;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tokio::runtime::Builder;

/// 启动交互式终端对话 (CLI Chat REPL)
pub fn run_chat(
    model_override: Option<String>,
    project_override: Option<String>,
    system_override: Option<String>,
) -> ! {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for chat");

    runtime.block_on(async {
        let client = Client::new();
        let mut llm = LlmProvider::with_model_override(client, model_override.clone());

        println!("\x1b[36m============================================================\x1b[0m");
        println!("\x1b[1;36m  🤖 DeepCloud Hub Super Agent CLI Interactive Chat\x1b[0m");
        println!("\x1b[36m============================================================\x1b[0m");
        println!("  • \x1b[1mActive Model\x1b[0m     : \x1b[32m{}\x1b[0m", llm.current_model());
        println!("  • \x1b[1mActive Provider\x1b[0m  : \x1b[33m{}\x1b[0m", llm.provider_name());
        println!("  • \x1b[1mTemperature\x1b[0m      : {:.2}", llm.current_temperature());
        if let Some(proj) = &project_override {
            println!("  • \x1b[1mProject Context\x1b[0m  : {}", proj);
        }
        if let Ok(Some(last_sess)) = crate::services::brain::session_checkpoint_store::SessionCheckpointStore::get_last_interrupted_session() {
            println!("  \x1b[33m⚡ 提示: 发现最近未完成的会话 [{}] (目标: {})\x1b[0m", last_sess.session_id, last_sess.goal);
            println!("  \x1b[90m可输入 '/resume' 立即恢复上下文继续推进。\x1b[0m");
        }
        println!("\n  \x1b[90mTip: Type '/help' for commands, '/sessions' to view checkpoints, '/model <name>' to switch models, '/exit' to quit.\x1b[0m");
        println!("\x1b[36m------------------------------------------------------------\x1b[0m\n");

        let mut conversation_history = Vec::new();
        let mut total_cost = 0.0;
        let mut total_prompt_tokens = 0;
        let mut total_completion_tokens = 0;
        let mut query_count = 0;

        let mut rl = DefaultEditor::new().expect("Failed to create rustyline editor");

        // 历史会话文件（便于上下方向键翻阅历史命令）
        let history_file = dirs::home_dir().map(|h| h.join(".hub").join("chat_history.txt"));
        if let Some(h_file) = &history_file {
            let _ = rl.load_history(h_file);
        }

        let mut multiline_buffer = String::new();
        let mut in_multiline = false;

        loop {
            let prompt_indicator = if in_multiline {
                "... "
            } else {
                "> "
            };

            let readline = rl.readline(prompt_indicator);
            match readline {
                Ok(line) => {
                    let text = line.trim();

                    // 支持三引号 """ 进入/退出多行代码粘贴模式
                    if text == "\"\"\"" {
                        if in_multiline {
                            in_multiline = false;
                            let full_text = multiline_buffer.trim().to_string();
                            multiline_buffer.clear();
                            if full_text.is_empty() {
                                continue;
                            }
                            let _ = rl.add_history_entry(&full_text);
                            // 将 full_text 作为普通用户消息执行
                            execute_chat_turn(
                                &full_text,
                                &mut llm,
                                &mut conversation_history,
                                &project_override,
                                &system_override,
                                &mut total_prompt_tokens,
                                &mut total_completion_tokens,
                                &mut total_cost,
                                &mut query_count,
                            ).await;
                        } else {
                            in_multiline = true;
                            multiline_buffer.clear();
                            println!("\x1b[90m[Entered multiline mode. Type '\"\"\"' on a new line to send]\x1b[0m");
                        }
                        continue;
                    }

                    if in_multiline {
                        multiline_buffer.push_str(&line);
                        multiline_buffer.push('\n');
                        continue;
                    }

                    if text.is_empty() {
                        continue;
                    }

                    let _ = rl.add_history_entry(text);

                    // 斜杠命令分发
                    if text.starts_with('/') {
                        let parts: Vec<&str> = text.split_whitespace().collect();
                        let cmd = parts[0];
                        match cmd {
                            "/exit" | "/quit" => break,
                            "/clear" => {
                                conversation_history.clear();
                                println!("\x1b[33m✓ Conversation history cleared.\x1b[0m");
                                continue;
                            }
                            "/undo" => {
                                if conversation_history.len() >= 2 {
                                    conversation_history.pop(); // 移除 assistant
                                    conversation_history.pop(); // 移除 user
                                    println!("\x1b[33m✓ Undid last conversation turn.\x1b[0m");
                                } else {
                                    println!("\x1b[90mNothing to undo.\x1b[0m");
                                }
                                continue;
                            }
                            "/model" => {
                                if parts.len() == 1 {
                                    println!("Current Model: \x1b[32m{}\x1b[0m", llm.current_model());
                                    println!("Provider     : \x1b[33m{}\x1b[0m", llm.provider_name());
                                    println!("Temperature  : {:.2}", llm.current_temperature());
                                    println!("\nTo switch models, type: /model <model_name>");
                                } else {
                                    let new_model = parts[1];
                                    llm.set_model(new_model);
                                    println!("\x1b[32m✓ Switched active model to: {}\x1b[0m", new_model);
                                }
                                continue;
                            }
                            "/models" => {
                                println!("\x1b[1mAvailable & Configured Models:\x1b[0m");
                                let models = llm.list_configured_models();
                                if models.is_empty() {
                                    println!("  - \x1b[32m{}\x1b[0m (active)", llm.current_model());
                                } else {
                                    for m in models {
                                        if m == llm.current_model() {
                                            println!("  * \x1b[32m{} (active)\x1b[0m", m);
                                        } else {
                                            println!("  - {}", m);
                                        }
                                    }
                                }
                                println!("\nSwitch with: /model <name>");
                                continue;
                            }
                            "/sessions" => {
                                crate::cli_commands::handle_sessions(10);
                                continue;
                            }
                            "/resume" => {
                                let sess_arg = parts.get(1).map(|s| s.to_string());
                                crate::cli_commands::handle_resume(sess_arg, None);
                                return;
                            }
                            "/fast" => {
                                let settings = crate::config::llm_settings::read_llm_settings();
                                let fast_model = settings.resolved_fast_model();
                                llm.set_model(&fast_model);
                                println!("\x1b[32m✓ Switched to Fast Model: {}\x1b[0m", fast_model);
                                continue;
                            }
                            "/temp" => {
                                if parts.len() == 2 {
                                    if let Ok(t) = parts[1].parse::<f64>() {
                                        llm.set_temperature(t);
                                        println!("\x1b[32m✓ Temperature updated to: {:.2}\x1b[0m", llm.current_temperature());
                                    } else {
                                        println!("Invalid float value: {}", parts[1]);
                                    }
                                } else {
                                    println!("Current temperature: {:.2}. Usage: /temp <0.0-2.0>", llm.current_temperature());
                                }
                                continue;
                            }
                            "/status" => {
                                println!("\x1b[1m=== Session Diagnostics ===\x1b[0m");
                                println!("  • Active Model      : \x1b[32m{}\x1b[0m", llm.current_model());
                                println!("  • Active Provider   : \x1b[33m{}\x1b[0m", llm.provider_name());
                                println!("  • Temperature       : {:.2}", llm.current_temperature());
                                println!("  • Context Turns     : {}", conversation_history.len() / 2);
                                println!("  • Queries Sent      : {}", query_count);
                                println!("  • Total In-Tokens   : {}", total_prompt_tokens);
                                println!("  • Total Out-Tokens  : {}", total_completion_tokens);
                                println!("  • Estimated Cost    : ${:.4}", total_cost);
                                continue;
                            }
                            "/save" => {
                                let target_path = if parts.len() > 1 {
                                    PathBuf::from(parts[1])
                                } else {
                                    let filename = format!("chat-export-{}.md", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
                                    PathBuf::from(filename)
                                };
                                let mut md = format!("# DeepCloud Hub Chat Export\n\n- Date: {}\n- Model: {}\n\n---\n\n",
                                    chrono::Utc::now().to_rfc3339(), llm.current_model());
                                for msg in &conversation_history {
                                    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                                    let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                    if role == "user" {
                                        md.push_str(&format!("### 🧑 User\n\n{}\n\n", content));
                                    } else {
                                        md.push_str(&format!("### 🤖 Assistant ({})\n\n{}\n\n", llm.current_model(), content));
                                    }
                                }
                                match fs::write(&target_path, md) {
                                    Ok(_) => println!("\x1b[32m✓ Chat exported to: {}\x1b[0m", target_path.display()),
                                    Err(e) => eprintln!("\x1b[31mFailed to save chat: {}\x1b[0m", e),
                                }
                                continue;
                            }
                            "/help" => {
                                print_help_menu();
                                continue;
                            }
                            _ => {
                                println!("Unknown command: '{}'. Type /help for available commands.", cmd);
                                continue;
                            }
                        }
                    } else if text == "exit" || text == "quit" {
                        break;
                    }

                    // 普通问答轮次
                    execute_chat_turn(
                        text,
                        &mut llm,
                        &mut conversation_history,
                        &project_override,
                        &system_override,
                        &mut total_prompt_tokens,
                        &mut total_completion_tokens,
                        &mut total_cost,
                        &mut query_count,
                    ).await;
                }
                Err(ReadlineError::Interrupted) => {
                    println!("\n\x1b[90m[CTRL-C: Use /exit or quit to exit]\x1b[0m");
                }
                Err(ReadlineError::Eof) => {
                    println!("\nBye!");
                    break;
                }
                Err(err) => {
                    eprintln!("Error: {:?}", err);
                    break;
                }
            }
        }

        if let Some(h_file) = &history_file {
            let _ = rl.save_history(h_file);
        }
    });

    std::process::exit(0);
}

/// 执行单轮对话
async fn execute_chat_turn(
    text: &str,
    llm: &mut LlmProvider,
    conversation_history: &mut Vec<serde_json::Value>,
    project_override: &Option<String>,
    system_override: &Option<String>,
    total_prompt_tokens: &mut u64,
    total_completion_tokens: &mut u64,
    total_cost: &mut f64,
    query_count: &mut usize,
) {
    let app_paths = crate::config::app_paths::AppPaths::new();
    let persona = crate::config::persona_settings::load_persona(&app_paths);

    let mut system_prompt = if let Some(sys) = system_override {
        sys.clone()
    } else {
        format!(
            "{}\n\nFollow the Engineering & Orchestration Principles strictly. Be concise, diagnostic, and truthful.",
            persona.render_prompt_section()
        )
    };

    let proj_path = project_override.as_deref().map(PathBuf::from);
    if let Some(instructions) = crate::services::project_context::load_project_instructions(proj_path.as_deref()) {
        system_prompt.push_str(&format!("\n\n[Project Instructions from .deepcloud.md]:\n{}", instructions));
    }

    conversation_history.push(json!({ "role": "user", "content": text }));
    *query_count += 1;

    print!("\x1b[90mThinking...\x1b[0m\r");

    match llm.chat(conversation_history.clone(), Some(&system_prompt)).await {
        Ok((response, usage)) => {
            *total_prompt_tokens += usage.prompt_tokens as u64;
            *total_completion_tokens += usage.completion_tokens as u64;
            *total_cost += usage.cost;

            println!("\r\x1b[K\n{}\n", response);
            println!(
                "\x1b[90m[Model: {} | Tokens: In {}, Out {} | Cost: ${:.4} | Time: {}ms | Session Total: ${:.4}]\x1b[0m",
                llm.current_model(), usage.prompt_tokens, usage.completion_tokens, usage.cost, usage.duration_ms, total_cost
            );
            conversation_history.push(json!({ "role": "assistant", "content": response }));
        }
        Err(e) => {
            eprintln!("\r\x1b[K\x1b[31mError:\x1b[0m {}", e);
            conversation_history.pop();
        }
    }
}

fn print_help_menu() {
    println!("\x1b[1mAvailable REPL Commands:\x1b[0m");
    println!("  \x1b[36m/model\x1b[0m            - Show current active model & provider");
    println!("  \x1b[36m/model <name>\x1b[0m     - Switch active model immediately (e.g. /model gpt-4o)");
    println!("  \x1b[36m/models\x1b[0m           - List all available models configured in system");
    println!("  \x1b[36m/fast\x1b[0m             - Switch to configured Fast Brain model");
    println!("  \x1b[36m/temp <val>\x1b[0m       - Set temperature (0.0 - 2.0, e.g. /temp 0.3)");
    println!("  \x1b[36m/status\x1b[0m           - Show session statistics, tokens and estimated cost");
    println!("  \x1b[36m/undo\x1b[0m             - Revert the last user query and AI answer");
    println!("  \x1b[36m/save [path]\x1b[0m      - Export current conversation history to Markdown");
    println!("  \x1b[36m/clear\x1b[0m            - Clear current conversation history");
    println!("  \x1b[36m\"\"\"\x1b[0m               - Enter / exit multiline code pasting mode");
    println!("  \x1b[36m/exit\x1b[0m             - Exit the interactive chat");
}
