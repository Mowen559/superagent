use async_trait::async_trait;

use crate::services::agent::backend::{AgentBackend, BackendError, BackendResult};
use crate::services::agent::models::{cached_discovery, Model};
use crate::services::agent::stream_json::{self, StreamJsonSpec};
use crate::services::agent::types::{AgentSession, ExecOptions};
use regex::Regex;
use std::process::Stdio;
use std::time::Duration;

pub struct CodebuddyBackend;

#[async_trait]
impl AgentBackend for CodebuddyBackend {
    async fn execute(
        &self,
        prompt: String,
        opts: ExecOptions,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        _control_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::domains::project_management::task_manager::TaskControl>,
        >,
    ) -> BackendResult<AgentSession> {
        stream_json::run(
            StreamJsonSpec {
                command: "codebuddy".to_string(),
                prompt_on_stdin: true,
                build_args: Box::new(|_prompt, opts, _mcp_path| {
                    let mut args = vec![];
                    if let Some(cwd) = &opts.cwd {
                        args.push("--cwd".to_string());
                        args.push(cwd.clone());
                    }
                    if let Some(m) = &opts.model {
                        args.push("--model".to_string());
                        args.push(m.clone());
                    }
                    if let Some(resume) = &opts.resume_session_id {
                        args.push("--session".to_string());
                        args.push(resume.clone());
                    }
                    if let Some(sys) = &opts.system_prompt {
                        args.push("--system".to_string());
                        args.push(sys.clone());
                    }
                    if let Some(effort) = &opts.thinking_level {
                        args.push("--effort".to_string());
                        args.push(effort.clone());
                    }
                    args.extend(opts.extra_args.clone());
                    args.extend(opts.custom_args.clone());
                    args
                }),
                envs: None,
            },
            prompt,
            opts,
            cancel_rx,
            _control_rx,
        )
        .await
    }

    async fn list_models(&self, executable_path: Option<&str>) -> BackendResult<Vec<Model>> {
        let exec = executable_path.unwrap_or("codebuddy").to_string();

        cached_discovery(
            "codebuddy".to_string(),
            exec.clone(),
            Duration::from_secs(60),
            || async move {
                let output = tokio::process::Command::new(&exec)
                    .arg("--help")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output()
                    .await
                    .map_err(|e| BackendError::Launch(e.to_string()))?;

                if !output.status.success() {
                    return Ok(vec![]);
                }

                let text = String::from_utf8_lossy(&output.stdout);
                let mut models = Vec::new();

                // Matches e.g. "Model to use [gpt-4o, claude-3-5-sonnet, gemini-1.5-pro]"
                let re = Regex::new(r"(?i)model\s+to\s+use\s+\[(.*?)\]").unwrap();

                if let Some(caps) = re.captures(&text) {
                    let models_str = caps.get(1).unwrap().as_str();
                    let model_names: Vec<&str> = models_str.split(',').map(|s| s.trim()).collect();

                    for (i, name) in model_names.iter().enumerate() {
                        if name.is_empty() {
                            continue;
                        }
                        let provider = if name.starts_with("claude-") {
                            Some("anthropic".to_string())
                        } else if name.starts_with("gpt-")
                            || name.starts_with("o1-")
                            || name.starts_with("o3-")
                        {
                            Some("openai".to_string())
                        } else if name.starts_with("gemini-") {
                            Some("google".to_string())
                        } else {
                            None
                        };

                        models.push(Model {
                            id: name.to_string(),
                            label: name.to_string(),
                            provider,
                            default: if i == 0 { Some(true) } else { None }, // Assume first is default
                            service_tiers: vec![],
                            thinking: None,
                        });
                    }
                }

                Ok(models)
            },
        )
        .await
    }

    async fn sync_config(
        &self,
        config: &crate::services::agent::types::SyncConfigPayload,
        _executable_path: Option<&str>,
    ) -> BackendResult<()> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".codebuddy");
        crate::services::agent::sync_utils::sync_claude_like(
            "codebuddy",
            config,
            &global_dir,
            ".codebuddy.json",
        )
    }

    async fn get_config_content(
        &self,
        _executable_path: Option<&str>,
    ) -> BackendResult<Option<String>> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".codebuddy");
        crate::services::agent::sync_utils::read_claude_like_config(&global_dir)
    }
}
