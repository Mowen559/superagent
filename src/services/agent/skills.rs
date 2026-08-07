use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisteredService {
    pub name: String,
    pub command: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DetectedSkill {
    pub name: String,
    pub directory: String,
    pub path: String,
    pub description: Option<String>,
    pub source_app: String,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_branch: Option<String>,
    pub readme_url: Option<String>,
    pub commit_hash: Option<String>,
    pub installed_at: Option<String>,
    pub updated_at: Option<String>,
    pub managed_by_agents_lock: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AgentsLockInfo {
    pub source: Option<String>,
    pub skill_path: Option<String>,
    pub branch: Option<String>,
    pub commit_hash: Option<String>,
    pub installed_at: Option<String>,
    pub updated_at: Option<String>,
}

pub struct ScannerCache {
    pub skill_meta: HashMap<PathBuf, (String, Option<String>)>,
    pub agents_lock: HashMap<String, AgentsLockInfo>,
}

impl ScannerCache {
    pub fn new() -> Self {
        Self {
            skill_meta: HashMap::new(),
            agents_lock: parse_agents_lock(),
        }
    }
}

pub fn scan_skills_for_agent(agent_type: &str, skill_paths: &[PathBuf], cache: &mut ScannerCache) -> Vec<DetectedSkill> {
    let mut skills = Vec::new();

    for skill_root in skill_paths {
        scan_skill_directory(agent_type, skill_root, cache, &mut skills);
        scan_skills_json(agent_type, skill_root, cache, &mut skills);
    }

    dedupe_skills(&mut skills);
    skills
}

pub fn parse_registered_services(config_path: &Path, mcp_keys: &[&str]) -> Vec<RegisteredService> {
    let Some(content) = std::fs::read_to_string(config_path).ok() else {
        return Vec::new();
    };

    if config_path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
        return parse_toml_mcp_servers(&content, mcp_keys);
    }

    parse_json_mcp_servers(&content, mcp_keys)
}

fn parse_json_mcp_servers(content: &str, mcp_keys: &[&str]) -> Vec<RegisteredService> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };

    let mut services = Vec::new();
    for key in mcp_keys {
        if let Some(mcp_servers) = json.get(*key).and_then(|value| value.as_object()) {
            for (name, config) in mcp_servers {
                let command = config
                    .get("command")
                    .and_then(|command| command.as_str())
                    .map(String::from);
                push_unique_service(&mut services, name, command);
            }
        }
    }
    services
}

fn parse_toml_mcp_servers(content: &str, mcp_keys: &[&str]) -> Vec<RegisteredService> {
    // Requires the toml crate
    if let Ok(value) = content.parse::<toml::Value>() {
        let mut services = Vec::new();
        for key in mcp_keys {
            if let Some(table) = value.get(*key).and_then(|value| value.as_table()) {
                for (name, config) in table {
                    let command = config
                        .get("command")
                        .and_then(|command| command.as_str())
                        .map(String::from);
                    push_unique_service(&mut services, name, command);
                }
            }
        }
        services
    } else {
        Vec::new()
    }
}

fn scan_skill_directory(
    agent_type: &str,
    skill_root: &Path,
    cache: &mut ScannerCache,
    skills: &mut Vec<DetectedSkill>,
) {
    let Ok(entries) = std::fs::read_dir(skill_root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !path.join("SKILL.md").exists() {
            continue;
        }
        if let Some(skill) = build_detected_skill(agent_type, &path, cache) {
            skills.push(skill);
        }
    }
}

fn scan_skills_json(
    agent_type: &str,
    skill_root: &Path,
    cache: &mut ScannerCache,
    skills: &mut Vec<DetectedSkill>,
) {
    let Some(parent) = skill_root.parent() else {
        return;
    };
    let Some(content) = std::fs::read_to_string(parent.join("skills.json")).ok() else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };

    let entries = json
        .get("entries")
        .and_then(|entries| entries.as_array())
        .cloned()
        .unwrap_or_default();

    for entry in entries {
        let Some(path) = entry.get("path").and_then(|path| path.as_str()) else {
            continue;
        };
        let path = PathBuf::from(path);
        if !path.is_dir() || !path.join("SKILL.md").exists() {
            continue;
        }
        if let Some(skill) = build_detected_skill(agent_type, &path, cache) {
            skills.push(skill);
        }
    }
}

fn build_detected_skill(
    agent_type: &str,
    path: &Path,
    cache: &mut ScannerCache,
) -> Option<DetectedSkill> {
    let directory = path.file_name()?.to_string_lossy().to_string();
    
    let md_path = path.join("SKILL.md");
    let (name, description) = if let Some(meta) = cache.skill_meta.get(&md_path) {
        meta.clone()
    } else {
        let meta = read_skill_name_desc(&md_path, &directory);
        cache.skill_meta.insert(md_path, meta.clone());
        meta
    };
    
    let lock = cache.agents_lock.get(&directory);
    let (repo_owner, repo_name) = lock
        .and_then(|info| info.source.as_deref())
        .and_then(|source| source.split_once('/'))
        .map(|(owner, repo)| (Some(owner.to_string()), Some(repo.to_string())))
        .unwrap_or((None, None));

    Some(DetectedSkill {
        name,
        directory: directory.clone(),
        path: path.to_string_lossy().to_string(),
        description,
        source_app: agent_type.to_string(),
        repo_owner,
        repo_name,
        repo_branch: lock.and_then(|info| info.branch.clone()),
        readme_url: lock.and_then(|info| build_skill_doc_url(info, &directory)),
        commit_hash: lock.and_then(|info| info.commit_hash.clone()),
        installed_at: lock.and_then(|info| info.installed_at.clone()),
        updated_at: lock.and_then(|info| info.updated_at.clone()),
        managed_by_agents_lock: lock.is_some(),
    })
}

fn read_skill_name_desc(skill_md: &Path, fallback_name: &str) -> (String, Option<String>) {
    let Some(content) = std::fs::read_to_string(skill_md).ok() else {
        return (fallback_name.to_string(), None);
    };
    let Some(frontmatter) = extract_frontmatter(&content) else {
        return (fallback_name.to_string(), None);
    };

    let mut name = None;
    let mut description = None;
    for line in frontmatter.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(clean_frontmatter_value(value));
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(clean_frontmatter_value(value));
        }
    }

    (
        name.filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback_name.to_string()),
        description,
    )
}

fn extract_frontmatter(content: &str) -> Option<&str> {
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return None;
    }
    let start = if content.starts_with("---\r\n") { 5 } else { 4 };
    let end = content[start..].find("\n---")?;
    Some(&content[start..start + end])
}

fn clean_frontmatter_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn push_unique_service(services: &mut Vec<RegisteredService>, name: &str, command: Option<String>) {
    if services.iter().any(|service| service.name == name) {
        return;
    }
    services.push(RegisteredService {
        name: name.to_string(),
        command,
    });
}

fn dedupe_skills(skills: &mut Vec<DetectedSkill>) {
    let mut seen = Vec::<String>::new();
    skills.retain(|skill| {
        let key = format!("{}:{}", skill.source_app, skill.path);
        if seen.iter().any(|value| value == &key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

// .agents/.skill-lock.json parsing
fn parse_agents_lock() -> HashMap<String, AgentsLockInfo> {
    let home = dirs::home_dir().unwrap_or_default();
    let lock_path = home.join(".agents").join(".skill-lock.json");
    
    let Some(content) = std::fs::read_to_string(&lock_path).ok() else {
        return HashMap::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return HashMap::new();
    };

    let Some(skills) = json.get("skills").and_then(|skills| skills.as_object()) else {
        return HashMap::new();
    };

    let mut parsed = HashMap::new();
    for (name, value) in skills {
        if value.get("source_type").and_then(|value| value.as_str()) != Some("github") {
            continue;
        }

        let source = value
            .get("source")
            .and_then(|value| value.as_str())
            .map(String::from);
        let source_url = value
            .get("source_url")
            .and_then(|value| value.as_str())
            .map(String::from);
        let branch = value
            .get("branch")
            .or_else(|| value.get("source_branch"))
            .and_then(|value| value.as_str())
            .map(String::from)
            .or_else(|| parse_branch_from_source_url(source_url.as_deref()));

        parsed.insert(
            name.clone(),
            AgentsLockInfo {
                source,
                skill_path: value
                    .get("skill_path")
                    .and_then(|value| value.as_str())
                    .map(String::from),
                branch,
                commit_hash: value
                    .get("commit_hash")
                    .or_else(|| value.get("commitHash"))
                    .and_then(|value| value.as_str())
                    .map(String::from),
                installed_at: value
                    .get("installed_at")
                    .or_else(|| value.get("installedAt"))
                    .and_then(|value| value.as_str())
                    .map(String::from),
                updated_at: value
                    .get("updated_at")
                    .or_else(|| value.get("updatedAt"))
                    .and_then(|value| value.as_str())
                    .map(String::from),
            },
        );
    }
    parsed
}

fn build_skill_doc_url(info: &AgentsLockInfo, directory: &str) -> Option<String> {
    let source = info.source.as_deref()?;
    let (owner, repo) = source.split_once('/')?;
    let branch = info.branch.as_deref().unwrap_or("HEAD");
    let fallback = format!("{directory}/SKILL.md");
    let doc_path = info.skill_path.as_deref().unwrap_or(&fallback);
    Some(format!(
        "https://github.com/{owner}/{repo}/blob/{branch}/{doc_path}"
    ))
}

fn parse_branch_from_source_url(source_url: Option<&str>) -> Option<String> {
    let source_url = source_url?.trim();
    if source_url.is_empty() {
        return None;
    }
    if let Some((_, after_tree)) = source_url.split_once("/tree/") {
        return after_tree
            .split('/')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from);
    }
    if let Some((_, fragment)) = source_url.split_once('#') {
        return fragment
            .split('&')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from);
    }
    None
}

pub fn write_agents_lock_skill(skill_name: &str, source: &AgentsLockInfo) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_default();
    let lock_path = home.join(".agents").join(".skill-lock.json");
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create lock dir: {e}"))?;
    }

    let mut json = std::fs::read_to_string(&lock_path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .unwrap_or_else(|| serde_json::json!({ "skills": {} }));
    
    if !json.is_object() {
        json = serde_json::json!({ "skills": {} });
    }

    if let Some(skills_obj) = json.as_object_mut() {
        if !skills_obj.contains_key("skills") {
            skills_obj.insert("skills".to_string(), serde_json::json!({}));
        }
        
        let skills = skills_obj.get_mut("skills").unwrap();
        if !skills.is_object() {
            *skills = serde_json::json!({});
        }

        if let Some(skills_map) = skills.as_object_mut() {
            let mut skill_info = serde_json::Map::new();
            skill_info.insert("source_type".to_string(), serde_json::json!("github"));
            if let Some(ref source_val) = source.source {
                skill_info.insert("source".to_string(), serde_json::json!(source_val));
            }
            if let Some(ref skill_path) = source.skill_path {
                skill_info.insert("skill_path".to_string(), serde_json::json!(skill_path));
            }
            if let Some(ref branch) = source.branch {
                skill_info.insert("branch".to_string(), serde_json::json!(branch));
            }
            if let Some(ref commit_hash) = source.commit_hash {
                skill_info.insert("commit_hash".to_string(), serde_json::json!(commit_hash));
            }
            if let Some(ref installed_at) = source.installed_at {
                skill_info.insert("installed_at".to_string(), serde_json::json!(installed_at));
            }
            if let Some(ref updated_at) = source.updated_at {
                skill_info.insert("updated_at".to_string(), serde_json::json!(updated_at));
            }

            skills_map.insert(skill_name.to_string(), serde_json::Value::Object(skill_info));
        }
    }

    let content = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(&lock_path, content).map_err(|e| e.to_string())
}

pub fn remove_agents_lock_skill(skill_name: &str) -> Result<bool, String> {
    let home = dirs::home_dir().unwrap_or_default();
    let lock_path = home.join(".agents").join(".skill-lock.json");

    let Some(content) = std::fs::read_to_string(&lock_path).ok() else {
        return Ok(false);
    };

    let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Ok(false);
    };

    let mut removed = false;
    if let Some(skills) = json
        .as_object_mut()
        .and_then(|obj| obj.get_mut("skills"))
        .and_then(|skills| skills.as_object_mut())
    {
        if skills.remove(skill_name).is_some() {
            removed = true;
        }
    }

    if removed {
        let new_content = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        std::fs::write(&lock_path, new_content).map_err(|e| e.to_string())?;
    }

    Ok(removed)
}

