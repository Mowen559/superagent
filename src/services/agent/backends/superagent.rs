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
        control_rx: Option<
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
        if let Some(sid) = opts.resume_session_id.as_ref().filter(|s| !s.trim().is_empty()) {
            args.push("--resume-session-id".to_string());
            args.push(sid.clone());
        }
        if let Some(cwd) = opts.cwd.as_ref().filter(|p| !p.trim().is_empty()) {
            args.push("--project".to_string());
            args.push(cwd.clone());
        }
        if let Some(mode) = opts.tool_policy.as_ref().filter(|m| !m.trim().is_empty()) {
            args.push("--tool-policy".to_string());
            args.push(mode.clone());
        }
        for image in opts.image_paths.iter().filter(|p| !p.trim().is_empty()) {
            args.push("--image".to_string());
            args.push(image.trim().to_string());
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
                        } else if t == "thinking" {
                            if let Some(c) = evt.get("content").and_then(|v| v.as_str()) {
                                out.push(AgentMessage::Thinking {
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
                        } else if t == "context_usage" {
                            let p = evt.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let max_t = evt.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(100_000) as u32;
                            let pct = evt.get("percentage").and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or_else(|| {
                                if max_t > 0 { (p as f32 / max_t as f32) * 100.0 } else { 0.0 }
                            });
                            out.push(AgentMessage::ContextUsage {
                                percentage: pct,
                                prompt_tokens: p,
                                max_tokens: max_t,
                            });
                            let model_name = if st.active_model.is_empty() { "superagent".to_string() } else { st.active_model.clone() };
                            let c = evt.get("completion_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                            st.usage.insert(
                                model_name,
                                crate::services::agent::types::TokenUsage {
                                    input_tokens: p as i64,
                                    output_tokens: c,
                                    cache_read_tokens: 0,
                                    cache_write_tokens: 0,
                                    cost: evt.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                },
                            );
                        } else if t == "status" {
                            if let Some(s) = evt.get("status").and_then(|v| v.as_str()) {
                                st.forced_status = Some(s.to_string());
                            }
                            if let Some(u) = evt.get("usage") {
                                let p = u.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                                let c = u.get("completion_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                                if p > 0 || c > 0 {
                                    let model_name = if st.active_model.is_empty() { "superagent".to_string() } else { st.active_model.clone() };
                                    st.usage.insert(
                                        model_name,
                                        crate::services::agent::types::TokenUsage {
                                            input_tokens: p,
                                            output_tokens: c,
                                            cache_read_tokens: 0,
                                            cache_write_tokens: 0,
                                            cost: u.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                        },
                                    );
                                }
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
                                    let choice_details = payload
                                        .get("choice_details")
                                        .or_else(|| payload.get("choiceDetails"))
                                        .cloned()
                                        .and_then(|v| serde_json::from_value(v).ok())
                                        .unwrap_or_default();
                                    out.push(AgentMessage::InteractionRequest {
                                        request_id: req_id.to_string(),
                                        question: q.to_string(),
                                        choices,
                                        choice_details,
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
            control_rx,
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
