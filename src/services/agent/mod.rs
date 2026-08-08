//! Local coding-agent integration layer.
//!
//! Ports the *capability* of multica's 17-CLI agent adapters (see
//! `multica/server/pkg/agent`) into hub as idiomatic Rust: a single
//! [`backend::AgentBackend`] trait, a unified [`types::AgentMessage`] /
//! [`types::AgentResult`] event model, a [`registry`] with a detection table +
//! factory, and shared protocol drivers ([`stream_json`], [`json_lines`]).
//!
//! This round ships the stream-json and json families live (claude, cursor,
//! qwen, codebuddy + copilot, opencode, deveco, openclaw, pi, antigravity); the
//! ACP and codex app-server families are wired as skeletons that detect and
//! list but return NotImplemented at runtime.

pub mod backend;
pub mod models;
pub mod types;

pub mod acp;
pub mod app_server;
pub mod args;
pub mod backends;
pub mod discovery;
pub mod installer;
pub mod json_lines;
pub mod mcp;
pub mod proc;
pub mod registry;
pub mod skills;
pub mod skills_sync;
pub mod stream_json;
pub mod sync_utils;

pub use registry::{detect_all, list_models, new, AgentKind};
pub use types::{AgentMessage, ExecOptions};
