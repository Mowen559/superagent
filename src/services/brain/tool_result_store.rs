use crate::config::app_paths::AppPaths;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use xxhash_rust::xxh3::xxh3_64;

const REFERENCE_PREFIX: &str = "tool-result://";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredToolResult {
    pub reference: String,
    pub original_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultSlice {
    pub reference: String,
    pub content: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub total_chars: usize,
}

#[derive(Debug, Clone)]
pub struct ToolResultStore {
    root: PathBuf,
    max_entries: usize,
    max_bytes: u64,
}

impl ToolResultStore {
    pub fn default_location() -> Self {
        Self::new(AppPaths::new().data_path(&["tool-results"]))
    }

    pub fn new(root: PathBuf) -> Self {
        Self::with_limits(root, 512, 256 * 1024 * 1024)
    }

    pub fn with_limits(root: PathBuf, max_entries: usize, max_bytes: u64) -> Self {
        Self {
            root,
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
        }
    }

    pub fn persist(&self, content: &str) -> Result<StoredToolResult, String> {
        fs::create_dir_all(&self.root)
            .map_err(|error| format!("failed to create tool result store: {error}"))?;
        let id = format!("{:016x}", xxh3_64(content.as_bytes()));
        let path = self.path_for_id(&id);
        if !path.exists() {
            let temporary = self.root.join(format!(".{}.tmp", Uuid::new_v4()));
            fs::write(&temporary, content)
                .map_err(|error| format!("failed to write tool result: {error}"))?;
            fs::rename(&temporary, &path)
                .map_err(|error| format!("failed to commit tool result: {error}"))?;
        }
        self.prune(&path)?;
        Ok(StoredToolResult {
            reference: format!("{REFERENCE_PREFIX}{id}"),
            original_chars: content.chars().count(),
        })
    }

    pub fn read_slice(
        &self,
        reference: &str,
        offset: usize,
        max_chars: usize,
    ) -> Result<ToolResultSlice, String> {
        let id = parse_reference(reference)?;
        let content = fs::read_to_string(self.path_for_id(id))
            .map_err(|error| format!("failed to read stored tool result: {error}"))?;
        let chars: Vec<char> = content.chars().collect();
        let total_chars = chars.len();
        let offset = offset.min(total_chars);
        let end = offset.saturating_add(max_chars).min(total_chars);
        let slice: String = chars[offset..end].iter().collect();

        Ok(ToolResultSlice {
            reference: reference.to_string(),
            content: slice,
            offset,
            next_offset: (end < total_chars).then_some(end),
            total_chars,
        })
    }

    fn path_for_id(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn prune(&self, preserve: &Path) -> Result<(), String> {
        let mut entries = fs::read_dir(&self.root)
            .map_err(|error| format!("failed to inspect tool result store: {error}"))?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                let modified = metadata.modified().ok();
                Some((path, modified, metadata.len()))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(_, modified, _)| *modified);
        let mut total_bytes = entries.iter().map(|(_, _, bytes)| *bytes).sum::<u64>();

        while entries.len() > 1
            && (entries.len() > self.max_entries || total_bytes > self.max_bytes)
        {
            let removable = entries
                .iter()
                .position(|(path, _, _)| path != preserve)
                .unwrap_or(0);
            let (path, _, bytes) = entries.remove(removable);
            fs::remove_file(&path).map_err(|error| {
                format!("failed to prune tool result {}: {error}", path.display())
            })?;
            total_bytes = total_bytes.saturating_sub(bytes);
        }
        Ok(())
    }
}

fn parse_reference(reference: &str) -> Result<&str, String> {
    let id = reference
        .strip_prefix(REFERENCE_PREFIX)
        .ok_or_else(|| "invalid tool result reference".to_string())?;
    if id.len() != 16 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid tool result reference".to_string());
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn persists_content_and_reads_bounded_slices() {
        let dir = tempdir().unwrap();
        let store = ToolResultStore::new(dir.path().to_path_buf());
        let stored = store.persist("0123456789abcdef").unwrap();

        let first = store.read_slice(&stored.reference, 0, 6).unwrap();
        assert_eq!(first.content, "012345");
        assert_eq!(first.next_offset, Some(6));

        let second = store.read_slice(&stored.reference, 6, 20).unwrap();
        assert_eq!(second.content, "6789abcdef");
        assert_eq!(second.next_offset, None);
    }

    #[test]
    fn rejects_path_traversal_references() {
        let dir = tempdir().unwrap();
        let store = ToolResultStore::new(dir.path().to_path_buf());
        assert!(store
            .read_slice("tool-result://../../secret", 0, 10)
            .is_err());
    }

    #[test]
    fn prunes_old_results_to_configured_entry_limit() {
        let dir = tempdir().unwrap();
        let store = ToolResultStore::with_limits(dir.path().to_path_buf(), 2, u64::MAX);
        store.persist("first result").unwrap();
        store.persist("second result").unwrap();
        let third = store.persist("third result").unwrap();

        assert!(store.read_slice(&third.reference, 0, 100).is_ok());
        assert_eq!(fs::read_dir(store.root()).unwrap().count(), 2);
    }
}
