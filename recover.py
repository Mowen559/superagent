import os
import glob

files = [
    "antigravity", "claude", "codebuddy", "copilot", "cursor", "deveco", "grok",
    "hermes", "kimi", "kiro", "openclaw", "opencode", "pi", "qoder", "qwen", "traecli"
]

template_stream_json = """use async_trait::async_trait;

use crate::services::agent::backend::{AgentBackend, BackendError, BackendResult};
use crate::services::agent::stream_json::{self, StreamJsonSpec};
use crate::services::agent::types::{AgentSession, ExecOptions};
use crate::services::agent::models::Model;

pub struct {Name}Backend;

#[async_trait]
impl AgentBackend for {Name}Backend {
    async fn execute(&self, prompt: String, opts: ExecOptions) -> BackendResult<AgentSession> {
        stream_json::run(
            StreamJsonSpec {
                command: "{cli_name}".to_string(),
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
                    args.extend(opts.extra_args.clone());
                    args.extend(opts.custom_args.clone());
                    args
                }),
            },
            prompt,
            opts,
        ).await
    }
}
"""

template_json = template_stream_json # Fallback

for name in files:
    cap_name = name.capitalize()
    cli_name = name + "-agent" if name != "claude" else "claude"
    if name == "codebuddy": cli_name = "codebuddy"
    if name == "cursor": cli_name = "cursor-agent"
    
    code = template_stream_json.replace("{Name}", cap_name).replace("{cli_name}", cli_name)
    with open(f"src/services/agent/backends/{name}.rs", "w", encoding="utf-8") as f:
        f.write(code)

with open("src/services/agent/backends/mod.rs", "w", encoding="utf-8") as f:
    for name in files:
        f.write(f"pub mod {name};\n")

print("Done")
