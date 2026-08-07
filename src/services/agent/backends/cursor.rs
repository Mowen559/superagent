use async_trait::async_trait;

use crate::services::agent::backend::{AgentBackend, BackendError, BackendResult};
use crate::services::agent::stream_json::{self, StreamJsonSpec};
use crate::services::agent::types::{AgentSession, ExecOptions};
use std::time::Duration;
use crate::services::agent::models::{Model, cached_discovery};
use regex::Regex;
use std::process::Stdio;

pub struct CursorBackend;

#[async_trait]
impl AgentBackend for CursorBackend {
    async fn execute(&self, prompt: String, opts: ExecOptions, cancel_rx: tokio::sync::watch::Receiver<bool>) -> BackendResult<AgentSession> {
        stream_json::run(
            StreamJsonSpec {
                command: "cursor-agent".to_string(),
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
            opts, cancel_rx).await
    }

    async fn list_models(&self, executable_path: Option<&str>) -> BackendResult<Vec<Model>> {
        let exec = executable_path.unwrap_or("cursor-agent").to_string();
        
        cached_discovery(
            "cursor".to_string(),
            exec.clone(),
            Duration::from_secs(60),
            || async move {
                let output = tokio::process::Command::new(&exec)
                    .arg("--list-models")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output()
                    .await
                    .map_err(|e| BackendError::Launch(e.to_string()))?;

                if !output.status.success() {
                    return Ok(vec![]); // fallback gracefully
                }

                let text = String::from_utf8_lossy(&output.stdout);
                let mut models = Vec::new();
                
                // Matches e.g. "gpt-4 (default)" or "claude-3-5-sonnet"
                // Group 1: model name
                // Group 2: optional " (default)"
                let re = Regex::new(r"^\s*([a-zA-Z0-9\-\.]+)(?:\s+(\(default\)))?").unwrap();

                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some(caps) = re.captures(line) {
                        let name = caps.get(1).unwrap().as_str().to_string();
                        let is_default = caps.get(2).is_some();
                        let provider = if name.starts_with("claude-") {
                            Some("anthropic".to_string())
                        } else if name.starts_with("gpt-") || name.starts_with("o1-") || name.starts_with("o3-") {
                            Some("openai".to_string())
                        } else if name.starts_with("gemini-") {
                            Some("google".to_string())
                        } else {
                            None
                        };

                        models.push(Model {
                            id: name.clone(),
                            label: name,
                            provider,
                            default: if is_default { Some(true) } else { None },
                            service_tiers: vec![],
                            thinking: None,
                        });
                    }
                }
                
                Ok(models)
            },
        ).await
    }

    async fn sync_config(
        &self,
        config: &crate::services::agent::types::SyncConfigPayload,
        _executable_path: Option<&str>,
    ) -> BackendResult<()> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".cursor");
        crate::services::agent::sync_utils::sync_claude_like(config, &global_dir, ".cursor.json")
    }
}


