use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::io::{self, Write};

/// 人工介入提问工具
pub struct AskUserTool;

impl ToolExecutor for AskUserTool {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn description(&self) -> &'static str {
        "Ask the user a question and wait for their response. Use this when you are unsure, need permission for a destructive action, or require information you cannot find."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Question": {
                    "type": "string",
                    "description": "The exact question to display to the user."
                }
            },
            "required": ["Question"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let question = args
            .get("Question")
            .and_then(|v| v.as_str())
            .unwrap_or("Waiting for user input...");

        println!("\n\x1b[33m[Agent Asks]: {}\x1b[0m", question);
        print!("\x1b[33m[Your Answer]: \x1b[0m");
        let _ = io::stdout().flush();

        let mut buffer = String::new();
        match io::stdin().read_line(&mut buffer) {
            Ok(_) => {
                let answer = buffer.trim().to_string();
                Ok(json!({
                    "status": "success",
                    "response": answer
                }))
            }
            Err(e) => Err(format!("Failed to read stdin: {}", e)),
        }
    }
}
