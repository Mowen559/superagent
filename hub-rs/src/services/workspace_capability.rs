use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Process-scoped registry of workspaces explicitly opened by the local UI.
/// Managed projects remain available for CLI/backwards-compatible callers.
pub struct WorkspaceCapabilityRegistry {
    managed_root: PathBuf,
    opened: Mutex<HashSet<PathBuf>>,
}

impl WorkspaceCapabilityRegistry {
    pub fn new(managed_root: impl AsRef<Path>) -> Self {
        let requested = managed_root.as_ref();
        let _ = std::fs::create_dir_all(requested);
        let managed_root = requested
            .canonicalize()
            .unwrap_or_else(|_| requested.to_path_buf());
        Self {
            managed_root,
            opened: Mutex::new(HashSet::new()),
        }
    }

    pub fn open(&self, path: impl AsRef<Path>) -> Result<PathBuf, String> {
        let canonical = canonical_directory(path.as_ref())?;
        self.opened
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(canonical.clone());
        Ok(canonical)
    }

    pub fn resolve(&self, path: impl AsRef<Path>) -> Result<PathBuf, String> {
        let canonical = canonical_directory(path.as_ref())?;
        let explicitly_opened = self
            .opened
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&canonical);
        if explicitly_opened || canonical.starts_with(&self.managed_root) {
            Ok(canonical)
        } else {
            Err(format!(
                "Workspace has not been opened by the local control plane: {}",
                canonical.display()
            ))
        }
    }

    #[cfg(test)]
    fn is_open(&self, path: impl AsRef<Path>) -> bool {
        self.resolve(path).is_ok()
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Cannot open workspace '{}': {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "Workspace is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_paths_require_explicit_open() {
        let managed = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let registry = WorkspaceCapabilityRegistry::new(managed.path());

        assert!(!registry.is_open(external.path()));
        registry.open(external.path()).unwrap();
        assert!(registry.is_open(external.path()));
    }

    #[test]
    fn managed_projects_are_available_without_registration() {
        let managed = tempfile::tempdir().unwrap();
        let project = managed.path().join("demo");
        std::fs::create_dir_all(&project).unwrap();
        let registry = WorkspaceCapabilityRegistry::new(managed.path());

        assert!(registry.is_open(&project));
    }
}
