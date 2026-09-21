//! 网络爬取工具 (Web Fetch Tool)
//! 赋予大脑 B 通过网络拉取网页内容的能力

use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::{json, Value};

/// 网页内容拉取工具
pub struct WebFetchTool;

impl ToolExecutor for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch content from a URL via HTTP request. Use when extracting text from public pages, reading static content/documentation."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Url": {
                    "type": "string",
                    "description": "URL to read content from."
                }
            },
            "required": ["Url"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let url_str = args
            .get("Url")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'Url' argument")?;

        eprintln!("[Brain B] Fetching URL: {}", url_str);

        // 使用 block_in_place 来在同步上下文里安全调用 async 代码
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = reqwest::Client::builder()
                    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
                    .build()
                    .map_err(|e| e.to_string())?;

                let res = client
                    .get(url_str)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !res.status().is_success() {
                    return Err(format!("HTTP Error: {}", res.status()));
                }
                res.text().await.map_err(|e| e.to_string())
            })
        });

        match result {
            Ok(content) => Ok(json!({ "content": content })),
            Err(e) => Err(format!("Failed to fetch URL: {}", e)),
        }
    }
}

/// 无头浏览器直接交互工具 (Headless Browser Tool)
pub struct HeadlessBrowserTool;

impl ToolExecutor for HeadlessBrowserTool {
    fn name(&self) -> &'static str {
        "headless_browser"
    }

    fn description(&self) -> &'static str {
        "Uses a real headless Chromium browser (via Playwright) to navigate to a URL, execute JavaScript, wait for network idle, and extract the fully rendered text content. Crucial for SPA and React/Vue websites."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Url": {
                    "type": "string",
                    "description": "URL to navigate to and render."
                }
            },
            "required": ["Url"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let url_str = args
            .get("Url")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'Url' argument")?;

        eprintln!(
            "\x1b[36m[Web Explorer] 🌐 Spawning Headless Chromium to render: {}\x1b[0m",
            url_str
        );

        // Find scripts/headless_crawler.py
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let script_path = cwd.join("scripts").join("headless_crawler.py");

        if !script_path.exists() {
            return Err("headless_crawler.py not found in scripts/ directory. Please ensure python and playwright are installed.".into());
        }

        let output = std::process::Command::new("python")
            .arg(&script_path)
            .arg(url_str)
            .output()
            .map_err(|e| format!("Failed to spawn python playwright process: {}", e))?;

        if output.stdout.is_empty() && !output.stderr.is_empty() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Headless Browser Error: {}", err));
        }

        let out_str = String::from_utf8_lossy(&output.stdout);
        let parsed: Value = serde_json::from_str(&out_str).unwrap_or_else(|_| {
            json!({
                "error": "Failed to parse json from browser output",
                "raw_output": out_str.to_string()
            })
        });

        Ok(parsed)
    }
}
