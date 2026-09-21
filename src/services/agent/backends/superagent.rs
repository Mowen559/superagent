use crate::services::agent::backend::{AgentBackend, BackendResult};
use crate::services::agent::json_lines::{self, JsonSpec};
use crate::services::agent::types::AgentMessage;
use crate::services::agent::types::{AgentSession, ExecOptions};
use async_trait::async_trait;

pub struct SuperAgentBackend;

#[async_trait]
impl AgentBackend for SuperAgentBackend {
    async fn execute(
        &self,
        prompt: String,
        opts: ExecOptions,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        _control_rx: Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::domains::project_management::task_manager::TaskControl>,
        >,
    ) -> BackendResult<AgentSession> {
        let current_exe =
            std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("hub"));
        let mut args = vec![
            "superagent-worker".to_string(),
            "--prompt".to_string(),
            prompt,
        ];
        if let Some(model) = opts.model.as_ref().filter(|m| !m.trim().is_empty()) {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        if let Some(cwd) = opts.cwd.as_ref().filter(|p| !p.trim().is_empty()) {
            args.push("--project".to_string());
            args.push(cwd.clone());
        }

        json_lines::run(
            JsonSpec {
                command: current_exe.to_string_lossy().to_string(),
                args,
                map_event: Box::new(|evt, st| {
                    let mut out = Vec::new();
                    if let Some(t) = evt.get("type").and_then(|v| v.as_str()) {
                        if t == "text" {
                            if let Some(c) = evt.get("content").and_then(|v| v.as_str()) {
                                st.output.push_str(c);
                                out.push(AgentMessage::Text {
                                    content: c.to_string(),
                                });
                            }
                        } else if t == "tool_call" {
                            if let (Some(tool), Some(args)) = (
                                evt.get("tool").and_then(|v| v.as_str()),
                                evt.get("arguments"),
                            ) {
                                out.push(AgentMessage::ToolUse {
                                    call_id: "".to_string(),
                                    tool: tool.to_string(),
                                    input: Some(args.clone()),
                                });
                            }
                        } else if t == "tool_result" {
                            if let (Some(tool), Some(res)) =
                                (evt.get("tool").and_then(|v| v.as_str()), evt.get("result"))
                            {
                                out.push(AgentMessage::ToolResult {
                                    call_id: "".to_string(),
                                    tool: tool.to_string(),
                                    output: serde_json::to_string(&res).unwrap_or_default(),
                                });
                            }
                        } else if t == "error" || t == "failed" {
                            st.forced_status = Some("failed".to_string());
                            if let Some(c) = evt.get("content").and_then(|v| v.as_str()) {
                                out.push(AgentMessage::Error {
                                    content: c.to_string(),
                                });
                            }
                        } else if t == "status" {
                            if let Some(s) = evt.get("status").and_then(|v| v.as_str()) {
                                st.forced_status = Some(s.to_string());
                            }
                            if let Some(res) = evt.get("result").and_then(|v| v.as_str()) {
                                if !res.trim().is_empty() && !st.output.contains(res) {
                                    if !st.output.is_empty() {
                                        st.output.push('\n');
                                    }
                                    st.output.push_str(res);
                                    out.push(AgentMessage::Text {
                                        content: res.to_string(),
                                    });
                                }
                            }
                        } else if t == "AskUser" {
                            if let Some(payload) = evt.get("payload") {
                                if let (Some(req_id), Some(q)) = (
                                    payload.get("request_id").and_then(|v| v.as_str()),
                                    payload.get("question").and_then(|v| v.as_str()),
                                ) {
                                    let choices = payload
                                        .get("choices")
                                        .and_then(|v| v.as_array())
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|c| c.as_str().map(|s| s.to_string()))
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    out.push(AgentMessage::InteractionRequest {
                                        request_id: req_id.to_string(),
                                        question: q.to_string(),
                                        choices,
                                    });
                                }
                            }
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

    async fn sync_config(
        &self,
        _config: &crate::services::agent::types::SyncConfigPayload,
        _executable_path: Option<&str>,
    ) -> BackendResult<()> {
        Ok(())
    }
}
