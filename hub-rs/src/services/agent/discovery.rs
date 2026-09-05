use super::AgentKind;
use std::path::PathBuf;

pub struct AgentDiscoverySpec {
    pub skill_paths: Vec<PathBuf>,
    pub config_path: PathBuf,
    pub mcp_keys: Vec<&'static str>,
}

pub fn get_agent_discovery_spec(kind: AgentKind) -> AgentDiscoverySpec {
    let home = crate::config::app_paths::AppPaths::home_dir().unwrap_or_default();
    match kind {
        AgentKind::Claude => AgentDiscoverySpec {
            skill_paths: vec![home.join(".claude").join("skills")],
            config_path: home.join(".claude").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Cursor => AgentDiscoverySpec {
            skill_paths: vec![home.join(".cursor").join("skills")],
            config_path: home.join(".cursor").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Codebuddy => AgentDiscoverySpec {
            skill_paths: vec![home.join(".codebuddy").join("skills")],
            config_path: home.join(".codebuddy").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Opencode => AgentDiscoverySpec {
            skill_paths: vec![home.join(".opencode").join("skills")],
            config_path: home.join(".opencode").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Openclaw => AgentDiscoverySpec {
            skill_paths: vec![home.join(".openclaw").join("skills")],
            config_path: home.join(".openclaw").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Copilot => AgentDiscoverySpec {
            skill_paths: vec![home.join(".copilot").join("skills")],
            config_path: home.join(".copilot").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Qwen => AgentDiscoverySpec {
            skill_paths: vec![home.join(".qwen").join("skills")],
            config_path: home.join(".qwen").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Deveco => AgentDiscoverySpec {
            skill_paths: vec![home.join(".deveco").join("skills")],
            config_path: home.join(".deveco").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Pi => AgentDiscoverySpec {
            skill_paths: vec![home.join(".pi").join("skills")],
            config_path: home.join(".pi").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Antigravity => AgentDiscoverySpec {
            skill_paths: vec![home.join(".antigravity").join("skills")],
            config_path: home.join(".antigravity").join("settings.json"),
            mcp_keys: vec!["mcpServers"],
        },
        AgentKind::Codex => AgentDiscoverySpec {
            skill_paths: vec![home.join(".codex").join("skills")],
            config_path: home.join(".codex").join("config.toml"),
            mcp_keys: vec!["mcpServers", "mcp_servers", "servers"],
        },
        // ACP family uses settings.json by default for now
        _ => {
            let name = kind.info().slug;
            AgentDiscoverySpec {
                skill_paths: vec![home.join(format!(".{}", name)).join("skills")],
                config_path: home.join(format!(".{}", name)).join("settings.json"),
                mcp_keys: vec!["mcpServers"],
            }
        }
    }
}
