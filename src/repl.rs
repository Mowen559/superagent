use std::io::{self, Write};
use tokio::runtime::Builder;
use reqwest::Client;
use crate::services::llm_provider::LlmProvider;
use serde_json::json;

pub fn run_chat() -> ! {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for chat");

    runtime.block_on(async {
        println!("Welcome to DeepCloud Hub CLI Chat!");
        println!("Type 'exit' or 'quit' to leave.");
        
        let client = Client::new();
        let llm = LlmProvider::new(client);

        let mut conversation_history = Vec::new();
        let mut total_cost = 0.0;
        let mut total_prompt_tokens = 0;
        let mut total_completion_tokens = 0;

        loop {
            print!("> ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                break;
            }

            let text = input.trim();
            if text.is_empty() {
                continue;
            }

            if text == "exit" || text == "quit" {
                break;
            }

            conversation_history.push(json!({ "role": "user", "content": text }));

            // For now, this is a very simple blocking chat request.
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
    });

    std::process::exit(0);
}
