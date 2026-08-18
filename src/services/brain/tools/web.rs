//! 网络爬取工具 (Web Fetch Tool)
//! 赋予大脑 B 通过网络拉取网页内容的能力

use crate::services::brain::executor::ToolExecutor;
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

                let res = client.get(url_str).send().await.map_err(|e| e.to_string())?;
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
