use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::discovery::get_agent_discovery_spec;
use super::skills::{AgentsLockInfo, DetectedSkill};
use super::AgentKind;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSkillSourceType {
    GitRepo,
    LocalDir,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallLocalSkillRequest {
    pub source_type: LocalSkillSourceType,
    pub git_url: Option<String>,
    pub git_ref: Option<String>,
    pub skill_path: Option<String>,
    pub local_dir: Option<String>,
    pub target_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateLocalSkillRequest {
    pub git_url: Option<String>,
    pub git_ref: Option<String>,
    pub skill_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UninstallSkillRequest {
    pub skill_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalSkillInstallResult {
    pub agent_type: String,
    pub success: bool,
    pub message: String,
    pub target_path: String,
    pub updated: bool,
    pub skill: Option<DetectedSkill>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalSkillUninstallResult {
    pub agent_type: String,
    pub success: bool,
    pub message: String,
}

pub async fn install_or_update_skill_for_agent(
    kind: AgentKind,
    req: InstallLocalSkillRequest,
    update: bool,
) -> Result<LocalSkillInstallResult, String> {
    let spec = get_agent_discovery_spec(kind);
    // Use the first skill path as the target install directory
    let skills_root = spec
        .skill_paths
        .first()
        .ok_or("No skill path found for this agent")?
        .clone();

    std::fs::create_dir_all(&skills_root)
        .map_err(|e| format!("failed to create skills dir: {e}"))?;

    let resolved = resolve_install_source(&req).await?;
    let target_name = req
        .target_name
        .as_deref()
        .map(normalize_skill_dir_name)
        .or_else(|| {
            resolved
                .skill_path
                .as_deref()
                .and_then(last_path_component)
                .map(normalize_skill_dir_name)
        })
        .or_else(|| {
            last_path_component(&resolved.source_dir.to_string_lossy())
                .map(normalize_skill_dir_name)
        })
        .ok_or_else(|| "unable to resolve target skill name".to_string())?;

    let target_dir = skills_root.join(&target_name);
    let source_dir = resolved
        .skill_path
        .as_deref()
        .map(|skill_path| resolved.source_dir.join(skill_path))
        .unwrap_or_else(|| resolved.source_dir.clone());

    if !source_dir.exists() || !source_dir.join("SKILL.md").exists() {
        return Err(format!(
            "source skill directory `{}` does not contain SKILL.md",
            source_dir.display()
        ));
    }

    replace_directory_with_backup(&source_dir, &target_dir)?;

    if let Some(source_info) = resolved.lock_source {
        super::skills::write_agents_lock_skill(&target_name, &source_info)?;
    }

    // Attempt to parse the skill we just installed
    let mut cache = super::skills::ScannerCache::new();
    let detected_skills =
        super::skills::scan_skills_for_agent(kind.info().slug, &spec.skill_paths, &mut cache);

    let skill = detected_skills.into_iter().find(|skill| {
        skill.directory.eq_ignore_ascii_case(&target_name)
            || skill.name.eq_ignore_ascii_case(&target_name)
    });

    Ok(LocalSkillInstallResult {
        agent_type: kind.info().slug.to_string(),
        success: true,
        message: format!(
            "Skill `{}` {} for {}.",
            target_name,
            if update { "updated" } else { "installed" },
            kind.info().slug
        ),
        target_path: target_dir.to_string_lossy().to_string(),
        updated: update,
        skill,
    })
}

pub async fn uninstall_skill(
    kind: AgentKind,
    req: UninstallSkillRequest,
) -> Result<LocalSkillUninstallResult, String> {
    let spec = get_agent_discovery_spec(kind);
    let skills_root = spec
        .skill_paths
        .first()
        .ok_or("No skill path found for this agent")?;
    let target_dir = skills_root.join(&req.skill_name);

    let mut removed = false;
    if target_dir.exists() && target_dir.is_dir() {
        std::fs::remove_dir_all(&target_dir)
            .map_err(|e| format!("failed to remove skill directory: {}", e))?;
        removed = true;
    }

    if remove_skill_from_json(skills_root, &req.skill_name) {
        removed = true;
    }

    if !removed {
        return Err("skill not found in local configuration".to_string());
    }

    let _ = super::skills::remove_agents_lock_skill(&req.skill_name);

    Ok(LocalSkillUninstallResult {
        agent_type: kind.info().slug.to_string(),
        success: true,
        message: format!("Skill {} uninstalled successfully.", req.skill_name),
    })
}

struct ResolvedSkillSource {
    source_dir: PathBuf,
    skill_path: Option<String>,
    lock_source: Option<AgentsLockInfo>,
}

async fn resolve_install_source(
    req: &InstallLocalSkillRequest,
) -> Result<ResolvedSkillSource, String> {
    match req.source_type {
        LocalSkillSourceType::GitRepo => {
            let git_url = req
                .git_url
                .as_deref()
                .ok_or_else(|| "git_repo source requires git_url".to_string())?;
            let git_ref = req.git_ref.as_deref().unwrap_or("main");
            let cache_dir = git_cache_dir(git_url);
            sync_git_repo(git_url, git_ref, &cache_dir).await?;
            let commit_hash = git_commit_hash(&cache_dir).await.ok();

            let source = source_slug_from_git_url(git_url)
                .ok_or_else(|| "git_url must be a GitHub URL or owner/repo slug".to_string())?;
            let _source_url = canonical_git_url(git_url);
            let skill_path = req.skill_path.clone().unwrap_or_default();
            let now = chrono::Utc::now().to_rfc3339();

            Ok(ResolvedSkillSource {
                source_dir: cache_dir,
                skill_path: if skill_path.is_empty() {
                    None
                } else {
                    validate_relative_path(&skill_path)?;
                    Some(skill_path.clone())
                },
                lock_source: Some(AgentsLockInfo {
                    source: Some(source),
                    skill_path: Some(skill_path),
                    branch: Some(git_ref.to_string()),
                    commit_hash,
                    installed_at: Some(now.clone()),
                    updated_at: Some(now),
                }),
            })
        }
        LocalSkillSourceType::LocalDir => {
            let local_dir = req
                .local_dir
                .as_deref()
                .ok_or_else(|| "local_dir source requires local_dir".to_string())?;
            Ok(ResolvedSkillSource {
                source_dir: PathBuf::from(local_dir),
                skill_path: req.skill_path.clone(),
                lock_source: None,
            })
        }
    }
}

pub fn resolve_update_source(
    detected: &DetectedSkill,
    req: UpdateLocalSkillRequest,
) -> Result<(String, String, Option<String>), String> {
    let git_url = req.git_url.or_else(|| {
        let owner = detected.repo_owner.as_deref()?;
        let repo = detected.repo_name.as_deref()?;
        Some(format!("https://github.com/{owner}/{repo}.git"))
    });
    let git_url = git_url.ok_or_else(|| {
        "update requires git_url because this skill has no tracked repo metadata".to_string()
    })?;

    let skill_path = req.skill_path.unwrap_or_else(|| detected.directory.clone());
    validate_relative_path(&skill_path)?;

    let git_ref = req.git_ref.or_else(|| detected.repo_branch.clone());
    Ok((git_url, skill_path, git_ref))
}

async fn sync_git_repo(git_url: &str, git_ref: &str, cache_dir: &Path) -> Result<(), String> {
    if cache_dir.join(".git").exists() {
        run_git(cache_dir, &["fetch", "--all", "--prune"]).await?;
        run_git(cache_dir, &["checkout", git_ref]).await?;
        run_git(cache_dir, &["pull", "--ff-only"]).await?;
        return Ok(());
    }

    if cache_dir.exists() {
        let is_empty = std::fs::read_dir(cache_dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true);
        if !is_empty {
            return Err(format!(
                "cache dir `{}` exists but is not a git repository",
                cache_dir.display()
            ));
        }
    } else {
        std::fs::create_dir_all(cache_dir)
            .map_err(|e| format!("failed to create cache dir: {e}"))?;
    }

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", "--branch", git_ref, git_url])
        .arg(cache_dir)
        .output()
        .await
        .map_err(|e| format!("failed to run git clone: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<(), String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| format!("failed to run git {}: {e}", args.join(" ")))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

async fn git_commit_hash(cwd: &Path) -> Result<String, String> {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| format!("failed to read git commit: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn replace_directory_with_backup(source: &Path, target: &Path) -> Result<(), String> {
    let backup = target.with_extension(format!("bak-{}", chrono::Utc::now().timestamp_millis()));
    let mut backup_created = false;

    if target.exists() {
        std::fs::rename(target, &backup)
            .map_err(|e| format!("failed to backup existing skill: {e}"))?;
        backup_created = true;
    }

    if let Err(error) = copy_dir_recursive(source, target) {
        if backup_created {
            let _ = std::fs::remove_dir_all(target);
            let _ = std::fs::rename(&backup, target);
        }
        return Err(error);
    }

    if backup_created {
        let _ = std::fs::remove_dir_all(backup);
    }

    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("failed to create target dir: {e}"))?;

    for entry in std::fs::read_dir(source).map_err(|e| format!("failed to read source dir: {e}"))? {
        let entry = entry.map_err(|e| format!("failed to read entry: {e}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("failed to read file type: {e}"))?;

        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &target_path).map_err(|e| {
                format!(
                    "failed to copy `{}` to `{}`: {e}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn remove_skill_from_json(skills_dir: &Path, skill_name: &str) -> bool {
    let Some(parent) = skills_dir.parent() else {
        return false;
    };
    let skills_json = parent.join("skills.json");
    let Ok(content) = std::fs::read_to_string(&skills_json) else {
        return false;
    };
    let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };

    let mut removed = false;
    if let Some(entries) = json
        .as_object_mut()
        .and_then(|obj| obj.get_mut("entries"))
        .and_then(|entries| entries.as_array_mut())
    {
        let original_len = entries.len();
        entries.retain(|entry| {
            let Some(path) = entry.get("path").and_then(|path| path.as_str()) else {
                return true;
            };
            let name = PathBuf::from(path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string());
            name.as_deref() != Some(skill_name)
        });
        removed = entries.len() < original_len;
    }

    if removed {
        if let Ok(new_content) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(&skills_json, new_content);
        }
    }

    removed
}

fn git_cache_dir(git_url: &str) -> PathBuf {
    let mut cache_dir =
        crate::config::app_paths::AppPaths::home_dir().unwrap_or_else(|| PathBuf::from("."));
    cache_dir.push(".ai-service-repository");
    cache_dir.push("skill-cache");
    cache_dir.push(format!(
        "{:x}",
        xxhash_rust::xxh3::xxh3_64(git_url.as_bytes())
    ));
    cache_dir
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("skill_path must be a relative path inside the source repository".to_string());
    }
    Ok(())
}

fn normalize_skill_dir_name(value: &str) -> String {
    value
        .trim()
        .trim_matches('/')
        .trim_matches('\\')
        .replace(['/', '\\', ':'], "-")
}

fn last_path_component(value: &str) -> Option<&str> {
    value
        .trim()
        .trim_matches('/')
        .trim_matches('\\')
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
}

fn source_slug_from_git_url(git_url: &str) -> Option<String> {
    let trimmed = git_url.trim().trim_end_matches(".git");
    if !trimmed.contains('/') {
        return None;
    }

    if let Some(after_github) = trimmed.split("git@github.com:").nth(1) {
        let mut parts = after_github.split('/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim();
        if !owner.is_empty() && !repo.is_empty() {
            return Some(format!("{owner}/{repo}"));
        }
    }

    if let Some(after_github) = trimmed.split("github.com/").nth(1) {
        let mut parts = after_github.split('/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim();
        if !owner.is_empty() && !repo.is_empty() {
            return Some(format!("{owner}/{repo}"));
        }
    }

    let mut parts: Vec<&str> = trimmed.split('/').collect();
    let repo = parts.pop()?.trim();
    let owner = parts.pop()?.trim();
    if !owner.is_empty() && !repo.is_empty() {
        Some(format!("{owner}/{repo}"))
    } else {
        None
    }
}

fn canonical_git_url(git_url: &str) -> String {
    if git_url.starts_with("http://")
        || git_url.starts_with("https://")
        || git_url.starts_with("git@")
    {
        git_url.to_string()
    } else {
        format!(
            "https://github.com/{}.git",
            git_url.trim().trim_end_matches(".git")
        )
    }
}
