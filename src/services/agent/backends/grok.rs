use crate::services::agent::acp::{discover_acp_models, AcpDiscoveryProvider};
use crate::services::agent::models::cached_discovery;
use std::time::Duration;

use async_trait::async_trait;

use crate::services::agent::acp::{self, AcpSpec};
use crate::services::agent::backend::{AgentBackend, BackendError, BackendResult};
use crate::services::agent::models::Model;
use crate::services::agent::types::{AgentSession, ExecOptions};

pub struct GrokBackend;

#[async_trait]
impl AgentBackend for GrokBackend {
    async fn execute(
        &self,
        prompt: String,
        opts: ExecOptions,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        _control_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::domains::project_management::task_manager::TaskControl>,
        >,
    ) -> BackendResult<AgentSession> {
        acp::run(
            AcpSpec {
                command: "grok-agent".to_string(),
                build_args: Box::new(|opts, _mcp_path| {
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
                    args.extend(opts.extra_args.clone());
                    args.extend(opts.custom_args.clone());
                    args
                }),
            },
            prompt,
            opts,
            cancel_rx,
            _control_rx,
        )
        .await
    }

    async fn list_models(&self, executable_path: Option<&str>) -> BackendResult<Vec<Model>> {
        let ep = executable_path.unwrap_or("grok").to_string();
        cached_discovery(
            "grok".to_string(),
            ep.clone(),
            Duration::from_secs(60),
            || async move {
                discover_acp_models(
                    Some(&ep),
                    AcpDiscoveryProvider {
                        default_bin: "grok".to_string(),
                        client_name: "multica-model-discovery".to_string(),
                        extra_env: vec![],
                        acp_args: vec![
                            "--no-auto-update".to_string(),
                            "agent".to_string(),
                            "--always-approve".to_string(),
                            "stdio".to_string(),
                        ],
                        select_auth_method: None,
                    },
                )
                .await
                .map_err(BackendError::Launch)
            },
        )
        .await
    }

    async fn sync_config(
        &self,
        config: &crate::services::agent::types::SyncConfigPayload,
        _executable_path: Option<&str>,
    ) -> BackendResult<()> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".grok");
        crate::services::agent::sync_utils::sync_claude_like(
            "grok",
            config,
            &global_dir,
            ".grok.json",
        )
    }

    async fn get_config_content(
        &self,
        _executable_path: Option<&str>,
    ) -> BackendResult<Option<String>> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".grok");
        crate::services::agent::sync_utils::read_claude_like_config(&global_dir)
    }
}
