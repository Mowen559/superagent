//! Disk-backed cache for LLM understanding results.
//!
//! One JSON file per project graph DB under
//! `{app_data_root}/cache/understanding/{db_stem}.json`, with an LRU cap so a
//! busy project cannot grow unbounded. Keys embed the output language plus a
//! content signature (source xxh3 for files, DB mtime + node count for the
//! project) so any relevant change naturally invalidates the entry.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use xxhash_rust::xxh3::xxh3_64;
use crate::config::app_paths::AppPaths;

const MAX_ENTRIES: usize = 500;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheData {
    version: u32,
    entries: Vec<CacheEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    key: String,
    value: Value,
    updated_at: String,
    last_used: String,
}

pub struct UnderstandingCache {
    path: PathBuf,
}

impl UnderstandingCache {
    /// Cache file bound to a specific graph DB (one file per project).
    pub fn for_db(app_paths: &AppPaths, db_path: &str) -> Self {
        let stem = Path::new(db_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "default".to_string());
        let dir = app_paths.ensure_data_dir(&["cache", "understanding"]);
        Self { path: dir.join(format!("{}.json", stem)) }
    }

    /// File-level key: language + relative path + source content hash.
    /// When the source is unavailable the caller passes the structural
    /// signature instead, which still tracks the indexed graph state.
    pub fn file_key(lang: &str, rel_path: &str, content: &str) -> String {
        format!("file:{}:{}:{:016x}", lang, rel_path, xxh3_64(content.as_bytes()))
    }

    /// Project-level key: language + graph DB mtime + total node count, so any
    /// re-index (which rewrites the DB) invalidates the overview.
    pub fn project_key(lang: &str, db_path: &str, nodes: i64) -> String {
        let mtime = std::fs::metadata(db_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("project:{}:{}:{}", lang, mtime, nodes)
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let mut data = self.load();
        let value = {
            let entry = data.entries.iter_mut().find(|e| e.key == key)?;
            entry.last_used = chrono::Utc::now().to_rfc3339();
            entry.value.clone()
        };
        self.save(&data);
        Some(value)
    }

    pub fn put(&self, key: &str, value: Value) {
        let mut data = self.load();
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(entry) = data.entries.iter_mut().find(|e| e.key == key) {
            entry.value = value;
            entry.updated_at = now.clone();
            entry.last_used = now;
        } else {
            data.entries.push(CacheEntry {
                key: key.to_string(),
                value,
                updated_at: now.clone(),
                last_used: now,
            });
        }
        // LRU eviction: drop the least recently used entries beyond the cap.
        if data.entries.len() > MAX_ENTRIES {
            data.entries.sort_by(|a, b| b.last_used.cmp(&a.last_used));
            data.entries.truncate(MAX_ENTRIES);
        }
        self.save(&data);
    }

    fn load(&self) -> CacheData {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or(CacheData { version: 1, entries: Vec::new() })
    }

    fn save(&self, data: &CacheData) {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(json) = serde_json::to_string(data) {
            std::fs::write(&self.path, json).ok();
        }
    }
}
