use async_trait::async_trait;

use crate::services::agent::backend::{AgentBackend, BackendResult};
use crate::services::agent::stream_json::{self, StreamJsonSpec};
use crate::services::agent::types::{AgentSession, ExecOptions};

pub struct QwenBackend;

#[async_trait]
impl AgentBackend for QwenBackend {
    async fn execute(&self, prompt: String, opts: ExecOptions, cancel_rx: tokio::sync::watch::Receiver<bool>, _control_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::services::task_manager::TaskControl>>) -> BackendResult<AgentSession> {
        stream_json::run(
            StreamJsonSpec {
                command: "qwen".to_string(),
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
            opts, cancel_rx, _control_rx).await
    }

    async fn sync_config(
        &self,
        config: &crate::services::agent::types::SyncConfigPayload,
        _executable_path: Option<&str>,
    ) -> BackendResult<()> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".qwen");
        crate::services::agent::sync_utils::sync_claude_like(config, &global_dir, ".qwen.json")
    }
}


