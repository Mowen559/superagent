use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

const MAX_CHANGESET_FILES: usize = 200_000;
const MAX_HASHED_BYTES: u64 = 2048 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileFingerprint {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineManifest {
    pub schema_version: u8,
    pub project_path: String,
    pub created_at: String,
    pub files: BTreeMap<String, FileFingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub kind: String,
    pub original_bytes: Option<u64>,
    pub proposed_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSetReceipt {
    pub schema_version: u8,
    pub files: Vec<ChangedFile>,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReceipt {
    pub schema_version: u8,
    pub applied_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub forced: bool,
}

pub fn capture_baseline(project: &Path, manifest_path: &Path) -> Result<BaselineManifest, String> {
    let manifest = BaselineManifest {
        schema_version: 1,
        project_path: project.to_string_lossy().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        files: scan_files(project)?,
    };
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let payload = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    let temp = manifest_path.with_extension("json.tmp");
    std::fs::write(&temp, payload).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, manifest_path).map_err(|error| error.to_string())?;
    Ok(manifest)
}

pub fn copy_workspace(source: &Path, destination: &Path) -> Result<(), String> {
    let canonical_source = source
        .canonicalize()
        .map_err(|error| format!("Cannot resolve source workspace: {error}"))?;
    let canonical_destination = if destination.exists() {
        destination
            .canonicalize()
            .map_err(|error| format!("Cannot resolve isolated workspace: {error}"))?
    } else {
        let parent = destination
            .parent()
            .ok_or_else(|| "Isolated workspace has no parent directory".to_string())?
            .canonicalize()
            .map_err(|error| format!("Cannot resolve isolated workspace parent: {error}"))?;
        let name = destination
            .file_name()
            .ok_or_else(|| "Isolated workspace has no directory name".to_string())?;
        parent.join(name)
    };
    if canonical_destination.starts_with(&canonical_source) {
        return Err("Isolated workspace cannot be nested inside the source workspace".to_string());
    }
    std::fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let mut builder = WalkBuilder::new(source);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .ignore(true)
        .filter_entry(|entry| !is_generated_directory(entry.path()));
    for entry in builder.build() {
        let entry = entry.map_err(|error| error.to_string())?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|error| error.to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let relative_str = relative.to_string_lossy().replace('\\', "/");
        if relative_str.split('/').any(|seg| matches!(
            seg,
            ".git"
                | ".deepcloud"
                | ".shadow-git"
                | ".supermemory"
                | "node_modules"
                | "target"
                | ".next"
                | "dist"
                | "build"
                | "coverage"
                | "release"
                | "mpv"
                | ".cargo"
                | ".agents"
                | ".idea"
                | ".vscode"
                | "out"
                | "bin"
                | "obj"
                | "vendor"
                | "venv"
                | ".venv"
                | "__pycache__"
                | ".pytest_cache"
                | ".turbo"
                | ".aindex-hub"
                | ".cache"
                | "openclaw"
                | "hermes-agent"
                | "deepseek-harness"
                | "Ghosten-Player"
                | "data"
                | "gh_cli"
                | ".codegraph"
                | ".cursor"
                | ".qoder"
                | "runs"
                | "scratch"
                | ".system_generated"
                | ".gemini"
        )) {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
        } else if entry.file_type().is_some_and(|kind| kind.is_file()) {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

pub fn load_baseline(path: &Path) -> Result<BaselineManifest, String> {
    let payload = std::fs::read(path).map_err(|error| {
        format!(
            "Cannot read workspace baseline '{}': {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("Invalid workspace baseline '{}': {error}", path.display()))
}

pub fn workspace_fingerprint(root: &Path) -> Result<String, String> {
    let files = scan_files(root)?;
    let mut hasher = Sha256::new();
    for (path, fingerprint) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(fingerprint.bytes.to_le_bytes());
        hasher.update(fingerprint.sha256.as_bytes());
    }
    Ok(Sha256::digest(hasher.finalize())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn inspect_changes(
    baseline: &BaselineManifest,
    original: &Path,
    proposed: &Path,
) -> Result<ChangeSetReceipt, String> {
    let current = scan_files(original)?;
    let proposed_files = scan_files(proposed)?;
    let mut paths = BTreeSet::new();
    paths.extend(baseline.files.keys().cloned());
    paths.extend(proposed_files.keys().cloned());
    let mut files = Vec::new();
    let mut conflicts = Vec::new();

    for path in paths {
        let before = baseline.files.get(&path);
        let after = proposed_files.get(&path);
        let kind = match (before, after) {
            (None, Some(_)) => Some("added"),
            (Some(_), None) => Some("deleted"),
            (Some(left), Some(right)) if left != right => Some("modified"),
            _ => None,
        };
        let Some(kind) = kind else { continue };
        if current.get(&path) != before {
            conflicts.push(path.clone());
        }
        files.push(ChangedFile {
            path,
            kind: kind.to_string(),
            original_bytes: before.map(|value| value.bytes),
            proposed_bytes: after.map(|value| value.bytes),
        });
    }

    Ok(ChangeSetReceipt {
        schema_version: 1,
        files,
        conflicts,
    })
}

pub fn apply_changes(
    receipt: &ChangeSetReceipt,
    original: &Path,
    proposed: &Path,
    selected: Option<&[String]>,
    forced: bool,
) -> Result<ApplyReceipt, String> {
    if !receipt.conflicts.is_empty() {
        return Err(format!(
            "Original workspace changed after the Agent run started: {}",
            receipt.conflicts.join(", ")
        ));
    }
    let known = receipt
        .files
        .iter()
        .map(|change| change.path.as_str())
        .collect::<BTreeSet<_>>();
    let selected = selected
        .map(|paths| paths.iter().map(String::as_str).collect::<BTreeSet<_>>())
        .unwrap_or_else(|| known.clone());
    if let Some(unknown) = selected.iter().find(|path| !known.contains(**path)) {
        return Err(format!(
            "File is not part of the Change Set Receipt: {unknown}"
        ));
    }

    let backup = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut applied = Vec::<(String, bool)>::new();
    let mut applied_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut skipped_files = Vec::new();

    for change in &receipt.files {
        if !selected.contains(change.path.as_str()) {
            skipped_files.push(change.path.clone());
            continue;
        }
        let relative = safe_relative(&change.path)?;
        let destination = original.join(&relative);
        let existed = destination.is_file();
        if existed {
            let backup_path = backup.path().join(&relative);
            if let Some(parent) = backup_path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::copy(&destination, backup_path).map_err(|error| error.to_string())?;
        }

        let result = if change.kind == "deleted" {
            if destination.exists() {
                std::fs::remove_file(&destination).map_err(|error| error.to_string())
            } else {
                Ok(())
            }
        } else {
            let source = proposed.join(&relative);
            if !source.is_file() {
                Err(format!("Proposed file is missing: {}", change.path))
            } else {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                std::fs::copy(source, &destination)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
        };

        if let Err(error) = result {
            applied.push((change.path.clone(), existed));
            rollback(original, backup.path(), &applied);
            return Err(format!("Apply failed at '{}': {error}", change.path));
        }
        applied.push((change.path.clone(), existed));
        if change.kind == "deleted" {
            deleted_files.push(change.path.clone());
        } else {
            applied_files.push(change.path.clone());
        }
    }

    Ok(ApplyReceipt {
        schema_version: 1,
        applied_files,
        deleted_files,
        skipped_files,
        forced,
    })
}

fn rollback(original: &Path, backup: &Path, applied: &[(String, bool)]) {
    for (path, existed) in applied.iter().rev() {
        let Ok(relative) = safe_relative(path) else {
            continue;
        };
        let destination = original.join(&relative);
        if *existed {
            let source = backup.join(&relative);
            if let Some(parent) = destination.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::copy(source, destination);
        } else if destination.is_file() {
            let _ = std::fs::remove_file(destination);
        }
    }
}

fn scan_files(root: &Path) -> Result<BTreeMap<String, FileFingerprint>, String> {
    let mut files = BTreeMap::new();
    let mut hashed_bytes = 0u64;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .ignore(true)
        .filter_entry(|entry| !is_generated_directory(entry.path()));
    for entry in builder.build() {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        safe_relative(&relative)?;
        if relative.split('/').any(|seg| matches!(
            seg,
            ".git"
                | ".deepcloud"
                | ".shadow-git"
                | ".supermemory"
                | "node_modules"
                | "target"
                | ".next"
                | "dist"
                | "build"
                | "coverage"
                | "release"
                | "mpv"
                | ".cargo"
                | ".agents"
                | ".idea"
                | ".vscode"
                | "out"
                | "bin"
                | "obj"
                | "vendor"
                | "venv"
                | ".venv"
                | "__pycache__"
                | ".pytest_cache"
                | ".turbo"
                | ".aindex-hub"
                | ".cache"
                | "openclaw"
                | "hermes-agent"
                | "deepseek-harness"
                | "Ghosten-Player"
                | "data"
                | "gh_cli"
                | ".codegraph"
                | ".cursor"
                | ".qoder"
                | "runs"
                | "scratch"
                | ".system_generated"
                | ".gemini"
        )) {
            continue;
        }
        let metadata = std::fs::metadata(entry.path()).map_err(|error| error.to_string())?;
        if is_binary_or_large_asset(entry.path(), metadata.len()) {
            continue;
        }
        let payload = std::fs::read(entry.path()).map_err(|error| error.to_string())?;
        hashed_bytes = hashed_bytes.saturating_add(payload.len() as u64);
        if hashed_bytes > MAX_HASHED_BYTES {
            return Err(format!(
                "Change set hashing exceeds {} bytes",
                MAX_HASHED_BYTES
            ));
        }
        let sha256 = Sha256::digest(&payload)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if files.len() >= MAX_CHANGESET_FILES {
            return Err(format!("Change set exceeds {MAX_CHANGESET_FILES} files"));
        }
        files.insert(
            relative,
            FileFingerprint {
                bytes: payload.len() as u64,
                sha256,
            },
        );
    }
    Ok(files)
}

fn safe_relative(path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("Unsafe relative path in change set: {path}"));
    }
    Ok(relative.to_path_buf())
}

fn is_generated_directory(path: &Path) -> bool {
    let name = match path.file_name().and_then(|value| value.to_str()) {
        Some(name) => name,
        None => return false,
    };
    matches!(
        name,
        ".git"
            | ".deepcloud"
            | ".shadow-git"
            | ".supermemory"
            | "node_modules"
            | "target"
            | ".next"
            | "dist"
            | "build"
            | "coverage"
            | "release"
            | "mpv"
            | ".cargo"
            | ".agents"
            | ".idea"
            | ".vscode"
            | "out"
            | "bin"
            | "obj"
            | "vendor"
            | "venv"
            | ".venv"
            | "__pycache__"
            | ".pytest_cache"
            | ".turbo"
            | ".aindex-hub"
            | ".cache"
            | "openclaw"
            | "hermes-agent"
            | "deepseek-harness"
            | "Ghosten-Player"
            | "data"
            | "gh_cli"
            | ".codegraph"
            | ".cursor"
            | ".qoder"
            | "runs"
            | "scratch"
            | ".system_generated"
            | ".gemini"
    )
}

fn is_binary_or_large_asset(path: &Path, file_len: u64) -> bool {
    if file_len > 10 * 1024 * 1024 {
        return true;
    }
    let ext = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => ext.to_ascii_lowercase(),
        None => return false,
    };
    matches!(
        ext.as_str(),
        "exe"
            | "dll"
            | "so"
            | "dylib"
            | "pdb"
            | "lib"
            | "rlib"
            | "jar"
            | "war"
            | "ear"
            | "zip"
            | "tar"
            | "gz"
            | "tgz"
            | "7z"
            | "rar"
            | "iso"
            | "bin"
            | "mp4"
            | "mkv"
            | "avi"
            | "mp3"
            | "wav"
            | "flac"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "ico"
            | "pdf"
            | "db"
            | "sqlite"
            | "sqlite3"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline(project: &Path, root: &Path) -> (BaselineManifest, PathBuf) {
        let path = root.join("baseline.json");
        (capture_baseline(project, &path).unwrap(), path)
    }

    #[test]
    fn detects_add_modify_delete_and_applies_only_selection() {
        let original = tempfile::tempdir().unwrap();
        let proposed = tempfile::tempdir().unwrap();
        std::fs::write(original.path().join("keep.txt"), "old").unwrap();
        std::fs::write(original.path().join("delete.txt"), "delete").unwrap();
        std::fs::write(proposed.path().join("keep.txt"), "new").unwrap();
        std::fs::write(proposed.path().join("add.txt"), "added").unwrap();
        let metadata = tempfile::tempdir().unwrap();
        let (baseline, _) = baseline(original.path(), metadata.path());

        let receipt = inspect_changes(&baseline, original.path(), proposed.path()).unwrap();
        assert_eq!(receipt.files.len(), 3);
        let applied = apply_changes(
            &receipt,
            original.path(),
            proposed.path(),
            Some(&["keep.txt".to_string()]),
            false,
        )
        .unwrap();
        assert_eq!(applied.applied_files, vec!["keep.txt"]);
        assert_eq!(
            std::fs::read_to_string(original.path().join("keep.txt")).unwrap(),
            "new"
        );
        assert!(original.path().join("delete.txt").exists());
        assert!(!original.path().join("add.txt").exists());
    }

    #[test]
    fn rejects_original_workspace_conflicts() {
        let original = tempfile::tempdir().unwrap();
        let proposed = tempfile::tempdir().unwrap();
        std::fs::write(original.path().join("app.rs"), "before").unwrap();
        std::fs::write(proposed.path().join("app.rs"), "agent").unwrap();
        let metadata = tempfile::tempdir().unwrap();
        let (baseline, _) = baseline(original.path(), metadata.path());
        std::fs::write(original.path().join("app.rs"), "user edit").unwrap();

        let receipt = inspect_changes(&baseline, original.path(), proposed.path()).unwrap();
        assert_eq!(receipt.conflicts, vec!["app.rs"]);
        assert!(apply_changes(&receipt, original.path(), proposed.path(), None, false).is_err());
    }

    #[test]
    fn rejects_an_isolated_copy_nested_inside_its_source() {
        let source = tempfile::tempdir().unwrap();
        let nested = source.path().join(".deepcloud").join("run");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();

        let error = copy_workspace(source.path(), &nested).unwrap_err();

        assert!(error.contains("cannot be nested"));
        assert!(!nested.exists());
    }
}
