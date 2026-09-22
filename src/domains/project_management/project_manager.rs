//! Multi-project index and repository management service.
//!
//! Inspired by codebase-memory-mcp's `list_projects`, `index_status`, and `delete_project`.
//! Discovers, queries health metadata, and manages the lifecycle of indexed graph databases.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use rusqlite::Connection;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexedProjectInfo {
    pub name: String,
    pub path: String,
    pub db_file: String,
    pub db_size_bytes: u64,
    pub node_count: usize,
    pub edge_count: usize,
    pub last_modified: String,
    pub is_ready: bool,
}

pub struct ProjectIndexManager;

impl ProjectIndexManager {
    /// Discovers all SQLite knowledge graph databases under search roots
    pub fn discover_projects<P: AsRef<Path>>(root: P) -> Vec<IndexedProjectInfo> {
        let root = root.as_ref();
        let mut projects = Vec::new();

        // Common candidate locations for graph databases
        let candidate_patterns = [
            ".codebase-memory/graph.db",
            ".codebase-memory/index.db",
            "codebase-memory.db",
            ".superagent/index.db",
            ".supermemory/supermemory.db",
        ];

        for pat in candidate_patterns {
            let candidate = root.join(pat);
            if candidate.exists() && candidate.is_file() {
                if let Some(info) = Self::inspect_project_db(&candidate, root) {
                    projects.push(info);
                }
            }
        }

        projects
    }

    /// Inspects an individual database file and extracts node/edge counts and metadata
    pub fn inspect_project_db(db_path: &Path, base_dir: &Path) -> Option<IndexedProjectInfo> {
        if !db_path.exists() {
            return None;
        }

        let metadata = std::fs::metadata(db_path).ok()?;
        let db_size_bytes = metadata.len();
        let last_modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ).ok()?;

        let node_count: usize = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
            .unwrap_or(0);

        let edge_count: usize = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap_or(0);

        let name = db_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("default-project")
            .to_string();

        Some(IndexedProjectInfo {
            name,
            path: base_dir.to_string_lossy().to_string(),
            db_file: db_path.to_string_lossy().to_string(),
            db_size_bytes,
            node_count,
            edge_count,
            last_modified,
            is_ready: node_count > 0,
        })
    }

    /// Deletes or resets an existing graph index safely
    pub fn delete_project_index(db_path: &Path) -> Result<bool, String> {
        if !db_path.exists() {
            return Ok(false);
        }

        std::fs::remove_file(db_path).map_err(|e| format!("Failed to delete index: {}", e))?;
        
        // Also remove wal and shm files if present
        let wal = db_path.with_extension("db-wal");
        if wal.exists() {
            let _ = std::fs::remove_file(wal);
        }
        let shm = db_path.with_extension("db-shm");
        if shm.exists() {
            let _ = std::fs::remove_file(shm);
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inspect_non_existent_returns_none() {
        let info = ProjectIndexManager::inspect_project_db(Path::new("missing.db"), Path::new("."));
        assert!(info.is_none());
    }
}
