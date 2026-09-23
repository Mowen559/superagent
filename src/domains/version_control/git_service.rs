use git2::{DiffOptions, Repository, StatusOptions};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepoKind {
    Local,
    Shadow,
}

impl RepoKind {
    pub fn parse(s: Option<&str>) -> RepoKind {
        match s {
            Some("shadow") => RepoKind::Shadow,
            _ => RepoKind::Local,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub is_git_repo: bool,
    pub branch: String,
    pub modified: Vec<String>,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    /// ISO-8601 string; the frontend sorts/renders on this field.
    pub date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffResult {
    pub file_path: String,
    pub diff: String,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowFileChange {
    pub path: String,
    pub status: String,
    pub diff: String,
}

/// Open the local repo, or the shadow (bare) repo with the project dir as worktree.
fn open_repo(project_path: &str, kind: RepoKind) -> Result<Repository, String> {
    match kind {
        RepoKind::Local => {
            let clean = crate::infrastructure::fs_path::strip_unc_prefix(project_path);
            Repository::discover(clean)
                .or_else(|_| Repository::open(clean))
                .or_else(|_| Repository::open(project_path))
                .map_err(|e| format!("Not a git repository: {}", e))
        }
        RepoKind::Shadow => {
            let shadow = Path::new(project_path).join(".shadow-git");
            if !shadow.exists() {
                return Err("Shadow git repository not found".into());
            }
            let repo = Repository::open(&shadow)
                .map_err(|e| format!("Failed to open shadow repo: {}", e))?;
            // Bare repo: attach the project dir as its worktree so workdir diffs work.
            repo.set_workdir(Path::new(project_path), false)
                .map_err(|e| format!("Failed to set shadow worktree: {}", e))?;
            Ok(repo)
        }
    }
}

fn to_iso(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn diff_to_text(diff: &git2::Diff) -> Result<(String, usize, usize), String> {
    let mut diff_text = String::new();
    let mut additions = 0;
    let mut deletions = 0;
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let content = std::str::from_utf8(line.content()).unwrap_or_default();
        match line.origin() {
            '+' => {
                additions += 1;
                diff_text.push_str(&format!("+{}", content));
            }
            '-' => {
                deletions += 1;
                diff_text.push_str(&format!("-{}", content));
            }
            ' ' => {
                diff_text.push_str(&format!(" {}", content));
            }
            _ => {}
        }
        true
    })
    .map_err(|e| e.to_string())?;
    Ok((diff_text, additions, deletions))
}

/// Get git status for a project
pub fn get_status(project_path: &str) -> Result<GitStatus, String> {
    let repo =
        Repository::open(project_path).map_err(|e| format!("Not a git repository: {}", e))?;

    let branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(String::from))
        .unwrap_or_else(|| "detached".into());

    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts)).map_err(|e| e.to_string())?;

    let mut modified = Vec::new();
    let mut added = Vec::new();
    let mut deleted = Vec::new();
    let mut untracked = Vec::new();

    for entry in statuses.iter() {
        let path = entry.path().unwrap_or_default().to_string();
        let status = entry.status();

        if status.contains(git2::Status::WT_MODIFIED)
            || status.contains(git2::Status::INDEX_MODIFIED)
        {
            modified.push(path);
        } else if status.contains(git2::Status::WT_NEW) || status.contains(git2::Status::INDEX_NEW)
        {
            if status.contains(git2::Status::WT_NEW) && !status.contains(git2::Status::INDEX_NEW) {
                untracked.push(path);
            } else {
                added.push(path);
            }
        } else if status.contains(git2::Status::WT_DELETED)
            || status.contains(git2::Status::INDEX_DELETED)
        {
            deleted.push(path);
        }
    }

    Ok(GitStatus {
        is_git_repo: true,
        branch,
        modified,
        added,
        deleted,
        untracked,
    })
}

/// Get file commit history from one repo. An unborn/empty repo yields an empty list.
pub fn get_file_history(
    project_path: &str,
    file_path: &str,
    limit: usize,
    kind: RepoKind,
) -> Result<Vec<GitCommit>, String> {
    let repo = match open_repo(project_path, kind) {
        Ok(r) => r,
        // Shadow repo missing is a normal state, not an error.
        Err(_) if kind == RepoKind::Shadow => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };
    if revwalk.push_head().is_err() {
        // Unborn HEAD (no commits yet)
        return Ok(Vec::new());
    }

    let mut commits = Vec::new();
    for oid in revwalk.take(limit * 3) {
        let oid = match oid {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Check if this commit touched the file
        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(file_path);

        if let Ok(diff) =
            repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut diff_opts))
        {
            if diff.deltas().len() > 0 {
                let author = commit.author();
                let ts = commit.time().seconds();
                commits.push(GitCommit {
                    hash: commit.id().to_string(),
                    short_hash: commit.id().to_string()[..7].to_string(),
                    message: commit.message().unwrap_or_default().trim().to_string(),
                    author: author.name().unwrap_or_default().to_string(),
                    email: author.email().unwrap_or_default().to_string(),
                    timestamp: ts,
                    date: to_iso(ts),
                });
                if commits.len() >= limit {
                    break;
                }
            }
        }
    }

    Ok(commits)
}

/// Get diff for a specific file.
/// - base + target: diff between the two commits
/// - target only:   diff of that commit tree against the current workdir file
/// - neither:       index -> workdir (uncommitted changes)
pub fn get_file_diff(
    project_path: &str,
    file_path: &str,
    base_hash: Option<&str>,
    target_hash: Option<&str>,
    kind: RepoKind,
) -> Result<GitDiffResult, String> {
    let repo = open_repo(project_path, kind)?;

    let mut opts = DiffOptions::new();
    opts.pathspec(file_path);
    // Untracked-in-this-repo files still need workdir diffs (esp. the shadow repo).
    opts.include_untracked(true).recurse_untracked_dirs(true);

    let find_tree = |hash: &str| -> Result<git2::Tree, String> {
        let oid = git2::Oid::from_str(hash).map_err(|e| format!("Invalid hash {}: {}", hash, e))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| format!("Commit not found {}: {}", hash, e))?;
        commit.tree().map_err(|e| e.to_string())
    };

    let diff = match (base_hash, target_hash) {
        (Some(base), Some(target)) => {
            let base_tree = find_tree(base)?;
            let target_tree = find_tree(target)?;
            repo.diff_tree_to_tree(Some(&base_tree), Some(&target_tree), Some(&mut opts))
                .map_err(|e| format!("Failed to compute diff: {}", e))?
        }
        (None, Some(target)) => {
            let target_tree = find_tree(target)?;
            repo.diff_tree_to_workdir(Some(&target_tree), Some(&mut opts))
                .map_err(|e| format!("Failed to compute diff: {}", e))?
        }
        _ => repo
            .diff_index_to_workdir(None, Some(&mut opts))
            .map_err(|e| format!("Failed to compute diff: {}", e))?,
    };

    let (diff_text, additions, deletions) = diff_to_text(&diff)?;

    Ok(GitDiffResult {
        file_path: file_path.to_string(),
        diff: diff_text,
        additions,
        deletions,
    })
}

/// Restore a file's content from a commit (defaults to HEAD) in the given repo.
/// The restored bytes are always written into the project worktree.
pub fn restore_file(
    project_path: &str,
    file_path: &str,
    hash: Option<&str>,
    kind: RepoKind,
) -> Result<(), String> {
    let guard = crate::services::workspace_guard::WorkspaceGuard::new(project_path)
        .map_err(|e| format!("Invalid project path: {}", e))?;
    let full_path = guard
        .resolve_safe(file_path)
        .map_err(|e| format!("Invalid file path (outside workspace): {}", e))?;

    let repo = open_repo(project_path, kind)?;

    let commit = match hash {
        Some(h) => {
            let oid = git2::Oid::from_str(h).map_err(|e| format!("Invalid hash {}: {}", h, e))?;
            repo.find_commit(oid)
                .map_err(|e| format!("Commit not found {}: {}", h, e))?
        }
        None => {
            let head = repo.head().map_err(|e| e.to_string())?;
            repo.find_commit(head.target().ok_or("HEAD has no target")?)
                .map_err(|e| e.to_string())?
        }
    };
    let tree = commit.tree().map_err(|e| e.to_string())?;

    // git uses relative path, ensure we strip prefix if it was made absolute by guard?
    // Actually, `guard.resolve_safe` returns an absolute canonical path.
    // But `tree.get_path` expects a relative path.
    let rel_path = match full_path.strip_prefix(guard.resolve_safe(".").unwrap_or_default()) {
        Ok(p) => p,
        Err(_) => Path::new(file_path),
    };

    let entry = tree
        .get_path(rel_path)
        .map_err(|_| format!("File not in commit: {}", file_path))?;
    let blob = repo.find_blob(entry.id()).map_err(|e| e.to_string())?;

    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&full_path, blob.content())
        .map_err(|e| format!("Failed to write file: {}", e))?;

    Ok(())
}

/// List files changed between the shadow repo's HEAD and the project worktree,
/// each with its textual diff. No shadow repo / no commits => empty list.
pub fn shadow_changed_files(project_path: &str) -> Result<Vec<ShadowFileChange>, String> {
    let repo = match open_repo(project_path, RepoKind::Shadow) {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };

    let head_tree = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok())
        .and_then(|c| c.tree().ok());

    if head_tree.is_none() {
        return Ok(Vec::new());
    }

    let mut opts = DiffOptions::new();
    let diff = repo
        .diff_tree_to_workdir(head_tree.as_ref(), Some(&mut opts))
        .map_err(|e| format!("Failed to compute shadow diff: {}", e))?;

    let mut changed: Vec<(String, String)> = Vec::new();
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        let status = match delta.status() {
            git2::Delta::Added => "added",
            git2::Delta::Deleted => "deleted",
            git2::Delta::Modified => "modified",
            git2::Delta::Renamed => "renamed",
            _ => "changed",
        };
        changed.push((path, status.to_string()));
    }

    // Per-file diff text (bounded to avoid giant payloads)
    let mut files = Vec::new();
    for (path, status) in changed.into_iter().take(100) {
        let mut fopts = DiffOptions::new();
        fopts.pathspec(&path);
        let ftext = repo
            .diff_tree_to_workdir(head_tree.as_ref(), Some(&mut fopts))
            .ok()
            .and_then(|d| diff_to_text(&d).ok())
            .map(|(t, _, _)| t)
            .unwrap_or_default();
        files.push(ShadowFileChange {
            path,
            status,
            diff: ftext,
        });
    }

    Ok(files)
}

/// Apply a unified diff to the project worktree.
pub fn apply_patch(project_path: &str, patch_content: &str) -> Result<(), String> {
    let repo =
        Repository::open(project_path).map_err(|e| format!("Not a git repository: {}", e))?;
    let diff = git2::Diff::from_buffer(patch_content.as_bytes())
        .map_err(|e| format!("Invalid patch: {}", e))?;
    repo.apply(&diff, git2::ApplyLocation::WorkDir, None)
        .map_err(|e| format!("Failed to apply patch: {}", e))
}

/// Clone a git repository
pub fn clone_repo(url: &str, target_path: &str) -> Result<String, String> {
    let _repo = Repository::clone(url, target_path).map_err(|e| format!("Clone failed: {}", e))?;
    Ok(target_path.to_string())
}

/// 为当前工作区生成一个脱离 HEAD 的影子快照
pub fn record_ai_snapshot(project_path: &str, message: &str) -> Result<String, String> {
    // 优先尝试原生 git stash create 极速生成快照，原生 git 严格遵循 .gitignore 且耗时仅毫秒级
    let output = std::process::Command::new("git")
        .args(["stash", "create", message])
        .current_dir(project_path)
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !hash.is_empty() {
                return Ok(hash);
            }
        }
    }

    // 降级使用轻量 git2 获取 HEAD 作为快照基线，杜绝在千万级大仓库中执行同步 add_all 全量扫描
    let repo = open_repo(project_path, RepoKind::Local)?;
    if let Ok(head) = repo.head() {
        if let Ok(commit) = head.peel_to_commit() {
            return Ok(commit.id().to_string());
        }
    }
    Ok("0000000000000000000000000000000000000000".to_string())
}

/// 获取影子快照历史
pub fn get_ai_history(project_path: &str, limit: usize) -> Result<Vec<GitCommit>, String> {
    let repo = open_repo(project_path, RepoKind::Local)?;
    let ref_name = "refs/deepcloud/ai-history";

    let _existing_ref = match repo.find_reference(ref_name) {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()), // No history yet
    };

    let mut revwalk = repo.revwalk().map_err(|e| e.to_string())?;
    revwalk.push_ref(ref_name).map_err(|e| e.to_string())?;

    let mut commits = Vec::new();
    for oid in revwalk.take(limit) {
        let oid = match oid {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let author = commit.author();
        let ts = commit.time().seconds();
        commits.push(GitCommit {
            hash: commit.id().to_string(),
            short_hash: commit.id().to_string()[..7].to_string(),
            message: commit.message().unwrap_or_default().trim().to_string(),
            author: author.name().unwrap_or_default().to_string(),
            email: author.email().unwrap_or_default().to_string(),
            timestamp: ts,
            date: to_iso(ts),
        });
    }

    Ok(commits)
}

/// 回滚工作区到指定快照（仅覆盖工作区文件，保持修改状态）
pub fn rollback_to_snapshot(project_path: &str, snapshot_hash: &str) -> Result<(), String> {
    let repo = open_repo(project_path, RepoKind::Local)?;

    let oid = git2::Oid::from_str(snapshot_hash).map_err(|e| format!("Invalid hash: {}", e))?;
    let commit = repo
        .find_commit(oid)
        .map_err(|e| format!("Snapshot not found: {}", e))?;

    let tree = commit.tree().map_err(|e| e.to_string())?;

    // 使用 checkout_tree 将快照内容直接还原到工作区，不更新 HEAD 也不更新 Index
    let mut checkout_builder = git2::build::CheckoutBuilder::new();
    checkout_builder.force().update_index(false);

    repo.checkout_tree(tree.as_object(), Some(&mut checkout_builder))
        .map_err(|e| format!("Failed to rollback files: {}", e))?;

    Ok(())
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCheckpoint {
    pub id: String,
    pub commit_hash: String,
    pub description: String,
    pub timestamp: String,
}

/// 为当前工作区创建命名检查点
pub fn create_checkpoint(project_path: &str, description: &str) -> Result<SnapshotCheckpoint, String> {
    let id = format!("cp-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let msg = format!("[Checkpoint: {}] {}", id, description);
    let commit_hash = record_ai_snapshot(project_path, &msg)?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    Ok(SnapshotCheckpoint {
        id,
        commit_hash,
        description: description.to_string(),
        timestamp: now,
    })
}

/// 回滚工作区到指定检查点或最新检查点
pub fn revert_to_checkpoint(project_path: &str, checkpoint_target: Option<&str>) -> Result<String, String> {
    let history = get_ai_history(project_path, 30)?;
    if history.is_empty() {
        return Err("No checkpoint history found to revert to.".to_string());
    }

    let target_commit = match checkpoint_target {
        Some(target) => {
            history.iter().find(|c| {
                c.hash.starts_with(target)
                    || c.short_hash == target
                    || c.message.contains(target)
            }).ok_or_else(|| format!("Checkpoint '{}' not found in history.", target))?
        }
        None => {
            history.iter().find(|c| c.message.contains("[Checkpoint:"))
                .unwrap_or(&history[0])
        }
    };

    rollback_to_snapshot(project_path, &target_commit.hash)?;
    Ok(format!(
        "Successfully reverted workspace to checkpoint '{}' ({})",
        target_commit.short_hash, target_commit.message
    ))
}

/// 列出当前工作区历史上的所有命名检查点
pub fn list_checkpoints(project_path: &str) -> Result<Vec<SnapshotCheckpoint>, String> {
    let history = get_ai_history(project_path, 50)?;
    let mut checkpoints = Vec::new();

    for commit in history {
        if commit.message.contains("[Checkpoint:") {
            let id = if let Some(start) = commit.message.find("[Checkpoint: ") {
                let rest = &commit.message[start + 13..];
                if let Some(end) = rest.find(']') {
                    rest[..end].to_string()
                } else {
                    commit.short_hash.clone()
                }
            } else {
                commit.short_hash.clone()
            };

            let description = if let Some(pos) = commit.message.find("] ") {
                commit.message[pos + 2..].to_string()
            } else {
                commit.message.clone()
            };

            let time_str = chrono::DateTime::from_timestamp(commit.timestamp, 0)
                .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| commit.timestamp.to_string());

            checkpoints.push(SnapshotCheckpoint {
                id,
                commit_hash: commit.hash,
                description,
                timestamp: time_str,
            });
        }
    }

    Ok(checkpoints)
}

