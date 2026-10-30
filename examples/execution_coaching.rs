use clap::Parser;
use hub_rs::{
    config::llm_settings, domains::model_gateway::providers::LlmProvider,
    services::brain::execution_coaching,
};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
struct Args {
    /// Explicitly send bounded real model requests; offline by default.
    #[arg(long)]
    live: bool,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    case: Option<String>,
    #[arg(long, default_value = "outputs/execution-coaching.json")]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut cases = execution_coaching::cases();
    if let Some(id) = args.case {
        cases.retain(|c| c.id == id);
        if cases.is_empty() {
            return Err("Unknown case".into());
        }
    }
    if !args.live {
        println!(
            "Offline preview: {} cases, 2 user turns each; at most {} model requests, 1536 output tokens per request. No model requests were made; no settings loaded.",
            cases.len(),
            cases.len() * execution_coaching::MAX_TURNS
        );
        return Ok(());
    }
    let mut settings = llm_settings::read_llm_settings();
    if let Some(model) = args.model {
        settings.model = model;
    }
    settings.max_tokens = settings.max_tokens.clamp(1, 1536);
    settings.temperature = 0.0;
    let requested_model = settings.model.clone();
    let provider = LlmProvider::with_fixed_settings(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .build()?,
        settings,
    )
    .with_purpose("eval:execution-coaching");
    if !provider.is_available() {
        return Err("No configured model route; no evaluation requests made".into());
    }
    let report = execution_coaching::evaluate(&provider, &cases).await;
    if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let artifact = serde_json::json!({"requested_model":requested_model,"temperature":0.0,"max_output_tokens":1536,"report":report});
    std::fs::write(&args.output, serde_json::to_vec_pretty(&artifact)?)?;
    let passed = artifact["report"]["results"]
        .as_array()
        .is_some_and(|results| {
            results.len() == cases.len() && results.iter().all(|r| r["status"] == "passed")
        });
    println!(
        "Execution coaching passed: {passed}. Report: {}. Component test only; not full AgentLoop certification.",
        args.output.display()
    );
    if !passed {
        return Err("Execution conversation did not pass; inspect saved report".into());
    }
    Ok(())
}
