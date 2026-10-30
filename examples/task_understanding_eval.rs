use clap::Parser;
use hub_rs::config::llm_settings;
use hub_rs::domains::model_gateway::providers::LlmProvider;
use hub_rs::services::brain::task_evaluation::{EvalSettings, cases, evaluate};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
struct Args {
    /// Make bounded real requests through the configured Canonical Model Gateway.
    #[arg(long)]
    live: bool,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value = "outputs/task-understanding-eval.json")]
    output: PathBuf,
    /// Evaluate one built-in scenario, keeping the same paired variants.
    #[arg(long)]
    case: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut fixtures = cases();
    if let Some(id) = args.case {
        fixtures.retain(|c| c.id == id);
        if fixtures.is_empty() {
            return Err("Unknown built-in case id".into());
        }
    }
    if !args.live {
        println!(
            "Offline preview: {} synthetic cases × 2 variants; at most {} model turns, 1536 output tokens per turn. Add --live to run. No model requests were made.",
            fixtures.len(),
            fixtures.len() * 2 * 4
        );
        return Ok(());
    }
    let mut settings = llm_settings::read_llm_settings();
    if let Some(model) = args.model {
        settings.model = model;
    }
    settings.max_tokens = settings.max_tokens.clamp(1, 1536);
    settings.temperature = 0.0;
    let eval_settings = EvalSettings {
        requested_model: settings.model.clone(),
        temperature: settings.temperature,
        max_output_tokens: settings.max_tokens,
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()?;
    let provider =
        LlmProvider::with_fixed_settings(client, settings).with_purpose("eval:task-understanding");
    if !provider.is_available() {
        return Err(
            "No configured provider route is available; no evaluation requests were made".into(),
        );
    }
    let mut report = evaluate(&provider, &fixtures).await;
    report.settings = Some(eval_settings);
    if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args.output, serde_json::to_vec_pretty(&report)?)?;
    for result in &report.results {
        println!(
            "{} / {}: {}; term checks {}/{}; unnecessary questions {}; turns {}",
            result.case_id,
            result.variant,
            result.status,
            result.covered_term_checks.len(),
            result.covered_term_checks.len() + result.missing_term_checks.len(),
            result.unnecessary_questions,
            result.model_turns
        );
    }
    println!(
        "Same actual model verified: {}. Report: {}. Synthetic planning only; term coverage does not prove real task success.",
        report.same_actual_model,
        args.output.display()
    );
    if !report.workflow_complete(&fixtures) {
        return Err("Evaluation contains unfinished planning workflows; inspect the bounded report".into());
    }
    Ok(())
}
