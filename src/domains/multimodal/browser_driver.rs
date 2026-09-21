use playwright::Playwright;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct BrowserDriver {
    pub headless: bool,
    playwright: Arc<Mutex<Option<Playwright>>>,
}

impl BrowserDriver {
    pub fn new() -> Self {
        Self {
            headless: true,
            playwright: Arc::new(Mutex::new(None)),
        }
    }
    
    pub async fn ensure_initialized(&self) -> Result<(), String> {
        let mut pw_lock = self.playwright.lock().await;
        if pw_lock.is_none() {
            let p = Playwright::initialize().await.map_err(|e| format!("Failed to init playwright: {}", e))?;
            p.prepare().map_err(|e| format!("Failed to install browsers: {}", e))?;
            *pw_lock = Some(p);
        }
        Ok(())
    }

    pub async fn navigate_and_screenshot(&self, url: &str) -> Result<Vec<u8>, String> {
        self.ensure_initialized().await?;
        
        let pw_lock = self.playwright.lock().await;
        let p = pw_lock.as_ref().unwrap();
        
        let chromium = p.chromium();
        let browser = chromium.launcher().headless(self.headless).launch().await.map_err(|e| format!("Launch error: {}", e))?;
        let context = browser.context_builder().build().await.map_err(|e| format!("Context error: {}", e))?;
        let page = context.new_page().await.map_err(|e| format!("Page error: {}", e))?;
        
        page.goto_builder(url).goto().await.map_err(|e| format!("Navigation error: {}", e))?;
        
        let bytes = page.screenshot_builder().screenshot().await.map_err(|e| format!("Screenshot error: {}", e))?;
        Ok(bytes)
    }
}

