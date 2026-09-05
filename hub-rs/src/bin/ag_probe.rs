use reqwest::header::USER_AGENT;
use serde_json::json;

#[tokio::main]
async fn main() {
    let client = reqwest::Client::new();
    let Some(token) = hub_rs::services::keyring_auth::get_or_refresh_credential(
        &client,
        "gemini:antigravity",
        "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com",
        Some("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"),
    )
    .await
    else {
        eprintln!("failed to read gemini:antigravity token");
        return;
    };

    for base in [
        "https://daily-cloudcode-pa.googleapis.com",
        "https://daily-cloudcode-pa.sandbox.googleapis.com",
    ] {
        for method in ["loadCodeAssist", "fetchAvailableModels"] {
            let url = format!("{base}/v1internal:{method}");
            let body = if method == "loadCodeAssist" {
                json!({ "metadata": { "ideType": "ANTIGRAVITY" } })
            } else {
                json!({})
            };
            println!("\n== {method} {base} ==");
            match client
                .post(url)
                .bearer_auth(&token)
                .header(USER_AGENT, "vscode/1.X.X (Antigravity/4.3.0)")
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(response) => {
                    println!("status: {}", response.status());
                    match response.text().await {
                        Ok(text) => println!("{text}"),
                        Err(error) => println!("failed to read body: {error}"),
                    }
                }
                Err(error) => println!("request failed: {error}"),
            }
        }
    }
}
