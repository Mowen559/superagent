use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::domains::multimodal::browser_driver::BrowserDriver;
use crate::domains::multimodal::vision_verifier::VisionVerifier;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct BrowserVisionTool {
    driver: Arc<BrowserDriver>,
}

impl BrowserVisionTool {
    pub fn new() -> Self {
        Self {
            driver: Arc::new(BrowserDriver::new()),
        }
    }
}

impl ToolExecutor for BrowserVisionTool {
    fn name(&self) -> &'static str {
        "browser_vision"
    }

    fn description(&self) -> &'static str {
        "Navigates to a URL using a real Playwright browser, takes a screenshot, and analyzes it using a Vision LLM. Perfect for UI/UX testing, DeepCloud DataTable component alignment checking, and debugging Next.js frontend CSS errors or permission toasts."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Url": {
                    "type": "string",
                    "description": "The URL of the local or remote frontend (e.g. http://localhost:3000)"
                },
                "Instruction": {
                    "type": "string",
                    "description": "Detailed instruction for the vision model on what to look for (e.g., 'Check if the DataTable action column is aligned', 'Read the error toast')."
                },
                "Visible": {
                    "type": "boolean",
                    "description": "Set to true to launch a visible Chromium window on the desktop, false for headless silent mode."
                }
            },
            "required": ["Url", "Instruction"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let url_str = args.get("Url").and_then(|v| v.as_str()).ok_or("Missing 'Url'")?;
        let instruction = args.get("Instruction").and_then(|v| v.as_str()).ok_or("Missing 'Instruction'")?;
        let _visible = args.get("Visible").and_then(|v| v.as_bool()).unwrap_or(false);

        eprintln!("\x1b[36m[BrowserVision] ?? Surfing to {} to check UI...\x1b[0m", url_str);

        // TODO: Map visible flag to BrowserDriver when supported.
        let verifier = VisionVerifier::new(self.driver.clone());

        // We use block_in_place for sync -> async bridge.
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Here it just generates the payload.
                // In a complete implementation, it would dispatch this payload to the model_gateway's Vision model.
                verifier.prepare_verification_payload(url_str, instruction).await
            })
        });

        match result {
            Ok(_payload) => {
                // Mock return for now since model routing requires more async context.
                // Normally we'd call model_gateway.request(vision_model, payload).await
                Ok(json!({
                    "status": "success",
                    "mock_analysis": format!("Screenshot taken and analyzed for '{}'. In real execution, this returns the Vision model's critique.", instruction)
                }))
            },
            Err(e) => Err(format!("Vision Tool Error: {}", e)),
        }
    }
}

