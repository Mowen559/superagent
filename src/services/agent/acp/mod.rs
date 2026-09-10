//! ACP (Agent Client Protocol) driver.
//!
//! The ACP family (hermes / kimi / kiro / qoder / traecli / grok) speaks
//! JSON-RPC 2.0 over stdio as a bidirectional long-lived connection:
//! `initialize` -> `session/new` | `session/load` (resume) -> `session/set_model`
//! -> `session/prompt`, while concurrently handling agent->client notifications
//! (`session/update` streaming chunks, `session/request_permission` which must
//! be auto-answered allow_once/reject_once) and matching pending request ids.

mod discovery;
mod runner;

pub use discovery::{discover_acp_models, AcpDiscoveryProvider};
pub use runner::{run, AcpSpec};
