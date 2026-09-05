use async_trait::async_trait;

use crate::services::agent::backend::{AgentBackend, BackendResult};
use crate::services::agent::models::{cached_discovery, Model, ModelThinking, ThinkingLevel};
use crate::services::agent::stream_json::{self, StreamJsonSpec};
use crate::services::agent::types::{AgentSession, ExecOptions};
use regex::Regex;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

pub struct ClaudeBackend;

#[async_trait]
impl AgentBackend for ClaudeBackend {
    async fn execute(
        &self,
        prompt: String,
        opts: ExecOptions,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        _control_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::services::task_manager::TaskControl>,
        >,
    ) -> BackendResult<AgentSession> {
        stream_json::run(
            StreamJsonSpec {
                command: "claude".to_string(),
                prompt_on_stdin: false,
                build_args: Box::new(|prompt, opts, _mcp_path| {
                    let mut args = vec![
                        "-p".to_string(),
                        prompt.to_string(),
                        "--output-format".to_string(),
                        "stream-json".to_string(),
                        "--verbose".to_string(),
                    ];
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
                    if let Some(settings) = &opts.claude_settings_path {
                        args.push("--settings".to_string());
                        args.push(settings.clone());
                    }
                    args.extend(opts.extra_args.clone());
                    args.extend(opts.custom_args.clone());
                    args
                }),
                envs: Some(Box::new(|_opts| {
                    let mut envs = HashMap::new();
                    // Point claude CLI to our internal protocol translation proxy
                    // By default, hub-rs runs on port 8080.
                    // TODO: Dynamically fetch port from config or context. Using 8080 for now.
                    envs.insert(
                        "ANTHROPIC_BASE_URL".to_string(),
                        "http://127.0.0.1:8080/proxy/anthropic".to_string(),
                    );
                    envs
                })),
            },
            prompt,
            opts,
            cancel_rx,
            _control_rx,
        )
        .await
    }

    async fn list_models(&self, executable_path: Option<&str>) -> BackendResult<Vec<Model>> {
        let exec = executable_path.unwrap_or("claude").to_string();

        cached_discovery(
            "claude".to_string(),
            exec.clone(),
            Duration::from_secs(60),
            || async move {
                let output = tokio::process::Command::new(&exec)
                    .arg("--help")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output()
                    .await;

                let mut thinking = None;
                if let Ok(out) = output {
                    if out.status.success() {
                        let text = String::from_utf8_lossy(&out.stdout);
                        // Match e.g. --effort <low|medium|high>
                        let re = Regex::new(r"(?i)--effort\s+<([^>]+)>").unwrap();
                        if let Some(caps) = re.captures(&text) {
                            let levels_str = caps.get(1).unwrap().as_str();
                            let levels: Vec<&str> =
                                levels_str.split('|').map(|s| s.trim()).collect();

                            let mut supported_levels = Vec::new();
                            for level in levels {
                                if level.is_empty() {
                                    continue;
                                }
                                let mut label = level.to_string();
                                if let Some(r) = label.get_mut(0..1) {
                                    r.make_ascii_uppercase();
                                }
                                supported_levels.push(ThinkingLevel {
                                    value: level.to_string(),
                                    label,
                                    description: None,
                                });
                            }

                            if !supported_levels.is_empty() {
                                thinking = Some(ModelThinking {
                                    supported_levels,
                                    default_level: Some("medium".to_string()),
                                });
                            }
                        }
                    }
                }

                // Hardcode standard claude models based on API availability
                let models = vec![
                    Model {
                        id: "claude-3-7-sonnet-20250219".to_string(),
                        label: "Claude 3.7 Sonnet".to_string(),
                        provider: Some("anthropic".to_string()),
                        default: Some(true),
                        service_tiers: vec![],
                        thinking: thinking.clone(), // Enable thinking if CLI supports effort
                    },
                    Model {
                        id: "claude-3-5-sonnet-20241022".to_string(),
                        label: "Claude 3.5 Sonnet".to_string(),
                        provider: Some("anthropic".to_string()),
                        default: None,
                        service_tiers: vec![],
                        thinking: None,
                    },
                    Model {
                        id: "claude-3-opus-20240229".to_string(),
                        label: "Claude 3 Opus".to_string(),
                        provider: Some("anthropic".to_string()),
                        default: None,
                        service_tiers: vec![],
                        thinking: None,
                    },
                    Model {
                        id: "claude-3-5-haiku-20241022".to_string(),
                        label: "Claude 3.5 Haiku".to_string(),
                        provider: Some("anthropic".to_string()),
                        default: None,
                        service_tiers: vec![],
                        thinking: None,
                    },
                ];

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
        let global_dir = dirs::home_dir().unwrap_or_default().join(".claude");
        crate::services::agent::sync_utils::sync_claude_like(
            "claude",
            config,
            &global_dir,
            ".claude.json",
        )
    }

    async fn get_config_content(
        &self,
        _executable_path: Option<&str>,
    ) -> BackendResult<Option<String>> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".claude");
        crate::services::agent::sync_utils::read_claude_like_config(&global_dir)
    }
}
