//! Agent registry: the canonical kind enum, the built-in detection table
//! (command name + minimum version per kind), the `new` factory that maps a
//! kind to a boxed [`AgentBackend`], and `detect_all` which probes every kind's
//! CLI on PATH. Mirrors multica's `New` switch + `MinVersions` + `SupportedTypes`.

use std::str::FromStr;

use serde::Serialize;

use super::app_server::AppServerBackend;
use super::backend::AgentBackend;
use super::backends;
use super::proc;

/// Every agent kind hub knows about. The stream-json and json families are
/// live this round; ACP and codex are wired to skeleton backends (they parse,
/// detect, and appear in listings, but `execute` returns NotImplemented).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    // stream-json family
    Claude,
    Cursor,
    Qwen,
    Codebuddy,
    // json family
    Copilot,
    Opencode,
    Deveco,
    Openclaw,
    Pi,
    Antigravity,
    // ACP family (skeleton)
    Hermes,
    Kimi,
    Kiro,
    Qoder,
    Traecli,
    Grok,
    // app-server (skeleton)
    Codex,
}

/// The protocol family a kind belongs to. Surfaced in detection output so the
/// frontend can badge live vs. not-yet-implemented backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    StreamJson,
    Json,
    Acp,
    AppServer,
}

impl Protocol {
    /// Whether a backend of this family can actually run this round.
    pub fn is_live(self) -> bool {
        matches!(self, Protocol::StreamJson | Protocol::Json | Protocol::Acp | Protocol::AppServer)
    }
}

/// One row of the built-in detection table.
pub struct KindInfo {
    pub kind: AgentKind,
    /// Canonical slug used in the API (`agent` field).
    pub slug: &'static str,
    /// Executable name probed on PATH (also spawned).
    pub command: &'static str,
    /// Minimum acceptable version, or `None` when unconstrained.
    pub min_version: Option<&'static str>,
    pub protocol: Protocol,
    /// User-visible launch skeleton (matches multica launchHeaders).
    pub launch_header: &'static str,
}

/// The canonical table. Command names + min versions are taken from multica
/// `agent.go` launchHeaders and `version.go` MinVersions.
pub const KINDS: &[KindInfo] = &[
    KindInfo { kind: AgentKind::Claude, slug: "claude", command: "claude", min_version: Some("2.0.0"), protocol: Protocol::StreamJson, launch_header: "claude (stream-json)" },
    KindInfo { kind: AgentKind::Cursor, slug: "cursor", command: "cursor-agent", min_version: None, protocol: Protocol::StreamJson, launch_header: "cursor-agent (stream-json)" },
    KindInfo { kind: AgentKind::Qwen, slug: "qwen", command: "qwen", min_version: Some("0.20.0"), protocol: Protocol::StreamJson, launch_header: "qwen -p (stream-json)" },
    KindInfo { kind: AgentKind::Codebuddy, slug: "codebuddy", command: "codebuddy", min_version: None, protocol: Protocol::StreamJson, launch_header: "codebuddy (stream-json)" },
    KindInfo { kind: AgentKind::Copilot, slug: "copilot", command: "copilot", min_version: Some("1.0.0"), protocol: Protocol::Json, launch_header: "copilot (json)" },
    KindInfo { kind: AgentKind::Opencode, slug: "opencode", command: "opencode", min_version: None, protocol: Protocol::Json, launch_header: "opencode run (json)" },
    KindInfo { kind: AgentKind::Deveco, slug: "deveco", command: "deveco", min_version: None, protocol: Protocol::Json, launch_header: "deveco run (json)" },
    KindInfo { kind: AgentKind::Openclaw, slug: "openclaw", command: "openclaw", min_version: None, protocol: Protocol::Json, launch_header: "openclaw agent (json)" },
    KindInfo { kind: AgentKind::Pi, slug: "pi", command: "pi", min_version: None, protocol: Protocol::Json, launch_header: "pi (json mode)" },
    KindInfo { kind: AgentKind::Antigravity, slug: "antigravity", command: "agy", min_version: None, protocol: Protocol::Json, launch_header: "agy -p (non-interactive)" },
    KindInfo { kind: AgentKind::Hermes, slug: "hermes", command: "hermes", min_version: None, protocol: Protocol::Acp, launch_header: "hermes acp" },
    KindInfo { kind: AgentKind::Kimi, slug: "kimi", command: "kimi", min_version: None, protocol: Protocol::Acp, launch_header: "kimi acp" },
    KindInfo { kind: AgentKind::Kiro, slug: "kiro", command: "kiro-cli", min_version: None, protocol: Protocol::Acp, launch_header: "kiro-cli acp" },
    KindInfo { kind: AgentKind::Qoder, slug: "qoder", command: "qodercli", min_version: None, protocol: Protocol::Acp, launch_header: "qodercli --acp" },
    KindInfo { kind: AgentKind::Traecli, slug: "traecli", command: "traecli", min_version: None, protocol: Protocol::Acp, launch_header: "traecli acp serve" },
    KindInfo { kind: AgentKind::Grok, slug: "grok", command: "grok", min_version: Some("0.2.89"), protocol: Protocol::Acp, launch_header: "grok agent stdio" },
    KindInfo { kind: AgentKind::Codex, slug: "codex", command: "codex", min_version: Some("0.100.0"), protocol: Protocol::AppServer, launch_header: "codex app-server" },
];

impl AgentKind {
    pub fn info(self) -> &'static KindInfo {
        KINDS.iter().find(|k| k.kind == self).expect("every kind is in KINDS")
    }
}

impl FromStr for AgentKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        KINDS
            .iter()
            .find(|k| k.slug.eq_ignore_ascii_case(s))
            .map(|k| k.kind)
            .ok_or_else(|| format!("unknown agent kind: {s:?}"))
    }
}

/// Build a backend for `kind`. Never fails: unknown protocols resolve to
/// skeleton backends whose `execute` returns NotImplemented.
pub fn new(kind: AgentKind) -> Box<dyn AgentBackend> {
    match kind {
        AgentKind::Claude => Box::new(backends::claude::ClaudeBackend),
        AgentKind::Cursor => Box::new(backends::cursor::CursorBackend),
        AgentKind::Qwen => Box::new(backends::qwen::QwenBackend),
        AgentKind::Codebuddy => Box::new(backends::codebuddy::CodebuddyBackend),
        AgentKind::Copilot => Box::new(backends::copilot::CopilotBackend),
        AgentKind::Opencode => Box::new(backends::opencode::OpencodeBackend),
        AgentKind::Deveco => Box::new(backends::deveco::DevecoBackend),
        AgentKind::Openclaw => Box::new(backends::openclaw::OpenclawBackend),
        AgentKind::Pi => Box::new(backends::pi::PiBackend),
        AgentKind::Antigravity => Box::new(backends::antigravity::AntigravityBackend),
        AgentKind::Codex => Box::new(AppServerBackend),
        // ACP family
        AgentKind::Hermes => Box::new(backends::hermes::HermesBackend),
        AgentKind::Kimi => Box::new(backends::kimi::KimiBackend),
        AgentKind::Kiro => Box::new(backends::kiro::KiroBackend),
        AgentKind::Qoder => Box::new(backends::qoder::QoderBackend),
        AgentKind::Traecli => Box::new(backends::traecli::TraecliBackend),
        AgentKind::Grok => Box::new(backends::grok::GrokBackend),
    }
}

/// Detection result for a single agent kind.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDetection {
    pub agent: &'static str,
    pub command: &'static str,
    pub protocol: Protocol,
    pub launch_header: &'static str,
    /// Whether the backend can actually run this round (family is live).
    pub supported: bool,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Below the built-in minimum, when a minimum is defined and installed.
    pub version_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_version: Option<&'static str>,
}

/// Probe every kind's CLI concurrently and report install/version status.
pub async fn detect_all() -> Vec<AgentDetection> {
    let futures = KINDS.iter().map(|info| async move {
        let version = proc::detect_version(info.command).await;
        let installed = version.is_some();
        let version_ok = match (&version, info.min_version) {
            (Some(v), Some(min)) => version_at_least(v, min),
            _ => true,
        };
        AgentDetection {
            agent: info.slug,
            command: info.command,
            protocol: info.protocol,
            launch_header: info.launch_header,
            supported: info.protocol.is_live(),
            installed,
            version,
            version_ok,
            min_version: info.min_version,
        }
    });
    futures::future::join_all(futures).await
}

/// Extract (major, minor, patch) from the first `\d+.\d+.\d+` triple in the
/// string (handles prefixes like `v` and suffixes like ` (Claude Code)`).
fn parse_semver(raw: &str) -> Option<(u32, u32, u32)> {
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut nums: Vec<u32> = Vec::new();
            let mut cur = String::new();
            let mut j = i;
            loop {
                if j < chars.len() && chars[j].is_ascii_digit() {
                    cur.push(chars[j]);
                    j += 1;
                } else if j < chars.len() && chars[j] == '.' && !cur.is_empty() && nums.len() < 2 {
                    nums.push(cur.parse().unwrap_or(0));
                    cur.clear();
                    j += 1;
                } else {
                    break;
                }
            }
            if !cur.is_empty() {
                nums.push(cur.parse().unwrap_or(0));
            }
            if nums.len() >= 3 {
                return Some((nums[0], nums[1], nums[2]));
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// True when `detected` >= `min` by semver, or when either cannot be parsed
/// (fail-open: a dev/unusual version string should not block a run).
fn version_at_least(detected: &str, min: &str) -> bool {
    match (parse_semver(detected), parse_semver(min)) {
        (Some(d), Some(m)) => d >= m,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_roundtrip_and_info() {
        assert_eq!(AgentKind::from_str("claude").unwrap(), AgentKind::Claude);
        assert_eq!(AgentKind::from_str("CURSOR").unwrap(), AgentKind::Cursor);
        assert!(AgentKind::from_str("nope").is_err());
        assert_eq!(AgentKind::Antigravity.info().command, "agy");
    }

    #[test]
    fn semver_parse_and_compare() {
        assert_eq!(parse_semver("2.1.100 (Claude Code)"), Some((2, 1, 100)));
        assert_eq!(parse_semver("v0.20.0"), Some((0, 20, 0)));
        assert!(version_at_least("2.0.0", "2.0.0"));
        assert!(version_at_least("2.1.0", "2.0.0"));
        assert!(!version_at_least("1.9.9", "2.0.0"));
        assert!(version_at_least("weird-dev", "2.0.0")); // fail-open
    }

    #[test]
    fn factory_is_total() {
        for info in KINDS {
            let _ = new(info.kind); // must not panic for any kind
        }
    }

    #[test]
    fn live_families_flagged() {
        assert!(Protocol::StreamJson.is_live());
        assert!(Protocol::Json.is_live());
        assert!(Protocol::Acp.is_live());
        assert!(Protocol::AppServer.is_live());
    }
}



/// Resolves the provider from the registry and calls its list_models implementation.
pub async fn list_models(kind: AgentKind, executable_path: Option<&str>) -> super::backend::BackendResult<Vec<super::models::Model>> {
    let backend = new(kind);
    backend.list_models(executable_path).await
}

/// Checks whether setting gent.model has any effect for the given provider.
pub fn model_selection_supported(kind: AgentKind) -> bool {
    let backend = new(kind);
    backend.model_selection_supported()
}

