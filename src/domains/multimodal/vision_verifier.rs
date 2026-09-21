use super::browser_driver::BrowserDriver;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::Arc;

pub struct VisionVerifier {
    driver: Arc<BrowserDriver>,
}

impl VisionVerifier {
    pub fn new(driver: Arc<BrowserDriver>) -> Self {
        Self { driver }
    }

    /// Navigates to a URL, takes a screenshot, and formats it for a Vision LLM.
    /// In a real scenario, this payload is handed to the model_gateway.
    pub async fn prepare_verification_payload(&self, url: &str, instruction: &str) -> Result<serde_json::Value, String> {
        let img_bytes = self.driver.navigate_and_screenshot(url).await?;
        let b64_img = STANDARD.encode(&img_bytes);
        
        // This constructs the standard OpenAI-compatible Vision API payload format.
        let payload = serde_json::json!([
            {
                "type": "text",
                "text": instruction
            },
            {
                "type": "image_url",
                "image_url": {
                    "url": format!("data:image/png;base64,{}", b64_img)
                }
            }
        ]);
        
        Ok(payload)
    }
}

