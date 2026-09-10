use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileEntry>>,
}

const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".next",
    ".turbo",
    ".understand-anything",
    ".supermemory",
    ".shadow-git",
    ".aindex-hub",
    ".idea",
    ".vscode",
    "node_modules",
    "dist",
    "build",
    "target",
    "__pycache__",
    ".cache",
];

const MAX_DEPTH: usize = 6;

/// Build a file tree for the given project path
pub fn build_file_tree(project_path: &str, max_depth: Option<usize>) -> Vec<FileEntry> {
    let root = Path::new(project_path);
    if !root.exists() {
        return Vec::new();
    }
    let depth = max_depth.unwrap_or(MAX_DEPTH);
    build_tree_recursive(root, root, depth)
}

fn build_tree_recursive(root: &Path, dir: &Path, remaining_depth: usize) -> Vec<FileEntry> {
    if remaining_depth == 0 {
        return Vec::new();
    }

    let mut entries: Vec<FileEntry> = Vec::new();
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    let mut files: Vec<FileEntry> = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();

        if name.starts_with('.')
            && name != ".env"
            && path.is_dir()
            && EXCLUDED_DIRS.contains(&name.as_str())
        {
            continue;
        }

        if path.is_dir() {
            if EXCLUDED_DIRS.contains(&name.as_str()) {
                continue;
            }
            dirs.push((name, path));
        } else {
            let size = entry.metadata().map(|m| m.len()).ok();
            let rel_path = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.push(FileEntry {
                name,
                path: rel_path,
                entry_type: "file".into(),
                size,
                children: None,
            });
        }
    }

    // Sort and add directories first
    dirs.sort_by_key(|a| a.0.to_lowercase());
    for (name, path) in dirs {
        let children = build_tree_recursive(root, &path, remaining_depth - 1);
        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(FileEntry {
            name,
            path: rel_path,
            entry_type: "directory".into(),
            size: None,
            children: Some(children),
        });
    }

    // Then files
    files.sort_by_key(|a| a.name.to_lowercase());
    entries.extend(files);
    entries
}

/// Read file content with size limit
pub fn read_file_content(project_path: &str, file_path: &str) -> Result<String, String> {
    let guard = crate::services::workspace_guard::WorkspaceGuard::new(project_path)
        .map_err(|e| format!("Invalid project path: {}", e))?;

    let full_path = guard
        .resolve_safe(file_path)
        .map_err(|e| format!("Invalid file path (outside workspace): {}", e))?;

    if !full_path.exists() {
        return Err(format!("File not found: {}", file_path));
    }

    let metadata = std::fs::metadata(&full_path).map_err(|e| e.to_string())?;
    if metadata.len() > 1024 * 1024 {
        return Err("File too large (>1MB)".into());
    }

    // Read as bytes first so that non-UTF-8 files (e.g. GBK encoded sources) degrade
    // gracefully instead of failing outright.
    let bytes = std::fs::read(&full_path).map_err(|e| format!("Failed to read file: {}", e))?;
    match String::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(e) => Ok(String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}
