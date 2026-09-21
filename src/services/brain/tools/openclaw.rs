use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::domains::token_compression::{compress, FilterLevel};
use serde_json::{json, Value};

pub struct OpenclawTool;

impl ToolExecutor for OpenclawTool {
    fn name(&self) -> &'static str {
        "openclaw_rtk"
    }

    fn description(&self) -> &'static str {
        "OpenClaw Capability: Transparently optimizes and rewrites large text/outputs to their RTK equivalents, reducing token context usage by up to 90% while preserving AST structures."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "RawContent": {
                    "type": "string",
                    "description": "The verbose text, shell output, or HTML that needs token optimization."
                },
                "Mode": {
                    "type": "string",
                    "enum": ["shell_output", "code_ast", "html_dom"],
                    "description": "The parsing mode to apply for RTK optimization."
                }
            },
            "required": ["RawContent", "Mode"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let raw_content = args
            .get("RawContent")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mode = args
            .get("Mode")
            .and_then(|v| v.as_str())
            .unwrap_or("shell_output");

        eprintln!(
            "\x1b[35m[OpenClaw] Applying RTK Optimization (Mode: {})...\x1b[0m",
            mode
        );

        let original_len = raw_content.len();

        let optimized = match mode {
            "shell_output" => compress(raw_content, "shell.log", FilterLevel::Aggressive).content,
            "code_ast" => compress(raw_content, "source.code", FilterLevel::Aggressive).content,
            "html_dom" => {
                let stripped = raw_content
                    .replace("<script", "<!--<script")
                    .replace("</script>", "</script>-->");
                compress(&stripped, "dom.html", FilterLevel::Minimal).content
            }
            _ => raw_content.to_string(),
        };

        let opt_len = optimized.len();
        let savings = if original_len > 0 {
            100.0 - ((opt_len as f64 / original_len as f64) * 100.0)
        } else {
            0.0
        };

        Ok(json!({
            "status": "success",
            "original_length": original_len,
            "optimized_length": opt_len,
            "token_savings_percent": format!("{:.2}%", savings),
            "content": optimized
        }))
    }
}
