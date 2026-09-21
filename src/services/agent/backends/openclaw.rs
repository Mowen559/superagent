use crate::services::agent::acp::{discover_acp_models, AcpDiscoveryProvider};
use crate::services::agent::models::cached_discovery;
use std::time::Duration;

use async_trait::async_trait;

use crate::services::agent::backend::{AgentBackend, BackendError, BackendResult};
use crate::services::agent::json_lines::{self, JsonSpec};
use crate::services::agent::models::Model;
use crate::services::agent::types::AgentMessage;
use crate::services::agent::types::{AgentSession, ExecOptions};

pub struct OpenclawBackend;

#[async_trait]
impl AgentBackend for OpenclawBackend {
    async fn execute(
        &self,
        prompt: String,
        opts: ExecOptions,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        _control_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::domains::project_management::task_manager::TaskControl>,
        >,
    ) -> BackendResult<AgentSession> {
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
        args.push(prompt);

        json_lines::run(
            JsonSpec {
                command: "openclaw-agent".to_string(),
                args,
                map_event: Box::new(|evt, st| {
                    let mut out = Vec::new();
                    if let Some(t) = evt.get("type").and_then(|v| v.as_str()) {
                        if t == "text" || t == "assistant.message" || t == "message" {
                            if let Some(c) = evt
                                .get("text")
                                .or_else(|| evt.get("content"))
                                .or_else(|| evt.pointer("/data/content"))
                                .and_then(|v| v.as_str())
                            {
                                st.output.push_str(c);
                                out.push(AgentMessage::Text {
                                    content: c.to_string(),
                                });
                            }
                        } else if t == "error" || t == "failed" {
                            st.forced_status = Some("failed".to_string());
                        }
                    }
                    out
                }),
                map_raw: None,
            },
            opts,
            cancel_rx,
            _control_rx,
        )
        .await
    }

    async fn list_models(&self, executable_path: Option<&str>) -> BackendResult<Vec<Model>> {
        let ep = executable_path.unwrap_or("openclaw").to_string();
        cached_discovery(
            "openclaw".to_string(),
            ep.clone(),
            Duration::from_secs(60),
            || async move {
                discover_acp_models(
                    Some(&ep),
                    AcpDiscoveryProvider {
                        default_bin: "openclaw".to_string(),
                        client_name: "multica-model-discovery".to_string(),
                        extra_env: vec![],
                        acp_args: vec!["acp".to_string()],
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
        let global_dir = dirs::home_dir().unwrap_or_default().join(".openclaw");
        crate::services::agent::sync_utils::sync_claude_like(
            "openclaw",
            config,
            &global_dir,
            ".openclaw.json",
        )
    }

    async fn get_config_content(
        &self,
        _executable_path: Option<&str>,
    ) -> BackendResult<Option<String>> {
        let global_dir = dirs::home_dir().unwrap_or_default().join(".openclaw");
        crate::services::agent::sync_utils::read_claude_like_config(&global_dir)
    }
}
