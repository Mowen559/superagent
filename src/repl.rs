use crate::domains::model_gateway::providers::LlmProvider;
use reqwest::Client;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde_json::json;
use tokio::runtime::Builder;

pub fn run_chat() -> ! {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for chat");

    runtime.block_on(async {
        println!("Welcome to DeepCloud Hub CLI Chat!");
        println!("Type '/help' for commands, '/exit' or 'quit' to leave.");

        let client = Client::new();
        let llm = LlmProvider::new(client);

        let mut conversation_history = Vec::new();
        let mut total_cost = 0.0;
        let mut total_prompt_tokens = 0;
        let mut total_completion_tokens = 0;

        let mut rl = DefaultEditor::new().expect("Failed to create rustyline editor");

        loop {
            let readline = rl.readline("> ");
            match readline {
                Ok(line) => {
                    let text = line.trim();
                    if text.is_empty() {
                        continue;
                    }

                    let _ = rl.add_history_entry(text);

                    // Handle slash commands
                    if text.starts_with('/') {
                        let parts: Vec<&str> = text.split_whitespace().collect();
                        let cmd = parts[0];
                        match cmd {
                            "/exit" | "/quit" => break,
                            "/clear" => {
                                conversation_history.clear();
                                println!("Conversation history cleared.");
                                continue;
                            }
                            "/help" => {
                                println!("Available commands:");
                                println!("  /help     - Show this help message");
                                println!("  /clear    - Clear conversation history");
                                println!("  /exit     - Exit the chat");
                                continue;
                            }
                            _ => {
                                println!("Unknown command: {}. Type /help for available commands.", cmd);
                                continue;
                            }
                        }
                    } else if text == "exit" || text == "quit" {
                        break;
                    }

                    conversation_history.push(json!({ "role": "user", "content": text }));

                    let mut system_prompt = "You are DeepCloud Hub, an advanced AI programming assistant and workflow orchestrator running in the user's terminal. Follow the Engineering & Orchestration Principles strictly. Be concise, diagnostic, and truthful.".to_string();

                    if let Some(instructions) = crate::services::project_context::load_project_instructions(None) {
                        system_prompt.push_str(&format!("\n\n[Project Instructions from .deepcloud.md]:\n{}", instructions));
                    }

                    match llm.chat(conversation_history.clone(), Some(&system_prompt)).await {
                        Ok((response, usage)) => {
                            total_prompt_tokens += usage.prompt_tokens;
                            total_completion_tokens += usage.completion_tokens;
                            total_cost += usage.cost;

                            println!("\n{}\n", response);
                            println!(
                                "\x1b[90m[Tokens: In {}, Out {} | Cost: ${:.4} | Time: {}ms | Session Total: ${:.4}]\x1b[0m",
                                usage.prompt_tokens, usage.completion_tokens, usage.cost, usage.duration_ms, total_cost
                            );
                            conversation_history.push(json!({ "role": "assistant", "content": response }));
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            conversation_history.pop();
                        }
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break;
                }
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        }
    });

    std::process::exit(0);
}
