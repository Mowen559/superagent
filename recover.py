import os
import glob

files = [
    "antigravity", "claude", "codebuddy", "copilot", "cursor", "deveco", "grok",
    "hermes", "kimi", "kiro", "openclaw", "opencode", "pi", "qoder", "qwen", "traecli"
]

template_stream_json = """use async_trait::async_trait;

use crate::services::agent::backend::{AgentBackend, BackendError, BackendResult};
use crate::services::agent::stream_json::{self, PromptDelivery, StreamJsonSpec};
use crate::services::agent::types::{AgentMessage, AgentSession, ExecOptions};
use crate::services::agent::models::Model;
use serde_json::Value;

pub struct {Name}Backend;

#[async_trait]
impl AgentBackend for {Name}Backend {
    async fn execute(&self, prompt: String, opts: ExecOptions) -> BackendResult<AgentSession> {
        let mut args = vec![];
        if let Some(cwd) = &opts.cwd {
            args.push("--cwd".to_string());
            args.push(cwd.clone());
        }
        if let Some(m) = &opts.model {
            args.push("--model".to_string());
            args.push(m.clone());
        }
        if let Some(variant) = &opts.variant {
            args.push("--variant".to_string());
            args.push(variant.clone());
        }
        if let Some(resume) = &opts.resume_session_id {
            args.push("--session".to_string());
            args.push(resume.clone());
        }
        args.extend(opts.extra_args.clone());
        args.extend(opts.custom_args.clone());

        let cmd = opts.executable_path.clone().unwrap_or_else(|| "{cli_name}".to_string());

        stream_json::spawn(StreamJsonSpec {
            command: cmd,
            args,
            prompt,
            prompt_delivery: PromptDelivery::Stdin,
            map_event: |evt, st| {
                let mut out = Vec::new();
                if let Some(sid) = evt.get("sessionID").and_then(|v| v.as_str()) {
                    st.session_id = sid.to_string();
                }
                if let Some(typ) = evt.get("type").and_then(|v| v.as_str()) {
                    match typ {
                        "text" => {
                            if let Some(part) = evt.get("part") {
                                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                    if !t.is_empty() {
                                        st.output.push_str(t);
                                        out.push(AgentMessage::Text { content: t.to_string() });
                                    }
                                }
                            }
                        }
                        "error" => {
                            let msg = evt.get("error").and_then(|e| e.get("message")).and_then(|v| v.as_str()).unwrap_or("unknown error").to_string();
                            st.forced_status = Some("failed".to_string());
                            st.forced_error = msg.clone();
                            out.push(AgentMessage::Error { content: msg });
                        }
                        _ => {}
                    }
                }
                out
            },
            parse_result: |_, _| None,
            env: std::collections::HashMap::new(),
        })
    }
}
"""

template_json = template_stream_json # Fallback

# For some, they use specific logic, but let's just create a base for all to compile first.
for name in files:
    cap_name = name.capitalize()
    cli_name = name + "-agent" if name != "claude" else "claude"
    if name == "codebuddy": cli_name = "codebuddy"
    if name == "cursor": cli_name = "cursor-agent"
    
    code = template_stream_json.replace("{Name}", cap_name).replace("{cli_name}", cli_name)
    with open(f"hub-rs/src/services/agent/backends/{name}.rs", "w", encoding="utf-8") as f:
        f.write(code)

with open("hub-rs/src/services/agent/backends/mod.rs", "w", encoding="utf-8") as f:
    for name in files:
        f.write(f"pub mod {name};\n")

print("Done")
