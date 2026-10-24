//! Unified event + option model shared by every agent backend.
//!
//! This mirrors multica's `agent.Message` / `agent.Result` / `agent.ExecOptions`
//! (see `multica/server/pkg/agent/agent.go`) translated to idiomatic Rust: a
//! single [`AgentMessage`] enum streamed over an `mpsc` channel while a run is
//! in flight, plus a single [`AgentResult`] delivered over a `oneshot` channel
//! once the run finishes. Every backend — regardless of underlying CLI protocol
//! (stream-json, json, ACP, app-server) — speaks this vocabulary so the routes,
//! event store, and SSE layer never learn per-CLI details.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single event emitted by an agent while it works. Serialized (tagged by
/// `kind`) into `{task_id}.events.jsonl` and pushed over SSE unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AgentMessage {
    /// Real-time token context usage broadcast.
    ContextUsage { percentage: f32, prompt_tokens: u32, max_tokens: u32 },
    /// Assistant answer text (may arrive as many fragments).
    Text { content: String },
    /// Reasoning / thinking content, kept distinct from user-facing text.
    Thinking { content: String },
    /// The agent invoked a tool. `input` is the raw tool arguments object.
    ToolUse {
        tool: String,
        #[serde(skip_serializing_if = "String::is_empty", default)]
        call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
    /// A tool finished; `output` is its (possibly truncated) result text.
    ToolResult {
        #[serde(skip_serializing_if = "String::is_empty", default)]
        call_id: String,
        #[serde(skip_serializing_if = "String::is_empty", default)]
        tool: String,
        output: String,
    },
    /// Coarse status transition (e.g. "running"); carries an early
    /// session-id pointer when the CLI announces one before completion.
    Status {
        status: String,
        #[serde(skip_serializing_if = "String::is_empty", default)]
        session_id: String,
    },
    /// A terminal-ish error surfaced mid-stream (not necessarily fatal).
    Error { content: String },
    /// Diagnostic log line with a severity level (info / warn / error).
    Log { level: String, content: String },
    /// A correlated request that pauses a managed Agent until the user answers.
    InteractionRequest {
        request_id: String,
        question: String,
        #[serde(default)]
        choices: Vec<String>,
        /// Structured choice metadata (ZCode AskUserQuestion / DeepSeek
        /// AskUserQuestionItem contract): label + description + recommended.
        #[serde(
            default,
            skip_serializing_if = "Vec::is_empty",
            deserialize_with = "crate::services::agent::types::deserialize_interaction_choices"
        )]
        choice_details: Vec<InteractionChoiceView>,
    },
}

/// Wire view of a structured interaction choice, shared by SSE and event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionChoiceView {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recommended: bool,
}

fn deserialize_interaction_choices<'de, D>(deserializer: D) -> Result<Vec<InteractionChoiceView>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = raw else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

/// Per-model token accounting, mirrors multica `agent.TokenUsage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    #[serde(default)]
    pub cost: f64,
}

/// Final outcome of an agent run, mirrors multica `agent.Result`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentResult {
    /// "completed" | "failed" | "aborted" | "timeout" | "cancelled".
    pub status: String,
    /// Final user-facing output selected by the backend.
    pub output: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub error: String,
    pub duration_ms: i64,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub session_id: String,
    /// Keyed by model name.
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub usage: HashMap<String, TokenUsage>,
}

impl AgentResult {
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            status: "failed".to_string(),
            error: error.into(),
            ..Default::default()
        }
    }
}

/// Configures a single execution. A mirror of the commonly-used subset of
/// multica `agent.ExecOptions`; every field is optional so a caller only sets
/// what it needs and backends ignore fields they do not consume (matching
/// multica's fall-through convention for `ThinkingLevel`, etc.).
#[derive(Debug, Clone, Default)]
pub struct ExecOptions {
    /// Working directory for the CLI subprocess.
    pub cwd: Option<String>,
    /// Model identifier passed through the CLI's `--model` flag.
    pub model: Option<String>,
    /// SystemPrompt carries the Multica runtime brief.
    pub system_prompt: Option<String>,
    /// Hard wall-clock cap; `None` leaves liveness to the caller.
    pub timeout: Option<std::time::Duration>,
    /// When set, resume a previous agent session (`--resume`/`--session`).
    pub resume_session_id: Option<String>,
    /// Environment variables injected into the agent subprocess.
    pub env: Option<HashMap<String, String>>,
    /// Serialized MCP config to be written to a temporary file.
    pub mcp_config: Option<serde_json::Value>,
    /// Per-agent extra CLI arguments appended after the managed argv.
    pub custom_args: Vec<String>,
    /// Additional daemon-managed CLI arguments appended after the managed argv.
    pub extra_args: Vec<String>,
    /// Runtime-native reasoning/effort value (`--effort`, `--variant`, ...).
    pub thinking_level: Option<String>,
    /// Runtime-native execution tier (Codex).
    pub service_tier: Option<String>,
    /// Openclaw routing mode ("local" vs "gateway").
    pub openclaw_mode: Option<String>,
    /// Daemon-owned, task-local settings file passed through Claude Code's `--settings` flag.
    pub claude_settings_path: Option<String>,
    /// Superagent work mode ("plan" | "build" | "edit" | "yolo" | "guarded");
    /// forwarded as `--tool-policy` to the superagent worker.
    pub tool_policy: Option<String>,
    /// Image attachment paths (hub-side staging files) forwarded as repeatable
    /// `--image` flags to the superagent worker.
    pub image_paths: Vec<String>,
}

/// A running agent execution: read [`Self::messages`] for streaming events and
/// await [`Self::result`] for the final outcome. Mirrors multica `agent.Session`.
pub struct AgentSession {
    pub messages: tokio::sync::mpsc::Receiver<AgentMessage>,
    pub result: tokio::sync::oneshot::Receiver<AgentResult>,
}

/// Payload sent by the frontend to synchronize parameter configurations
/// for a specific agent (or multiple agents).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigPayload {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
}
