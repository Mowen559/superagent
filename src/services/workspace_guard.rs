use std::path::{Path, PathBuf};

pub struct WorkspaceGuard {
    project_root: PathBuf,
}

impl WorkspaceGuard {
    pub fn new<P: AsRef<Path>>(project_root: P) -> std::io::Result<Self> {
        let canonical_root = std::fs::canonicalize(project_root)?;
        Ok(Self {
            project_root: canonical_root,
        })
    }

    pub fn resolve_safe<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> std::io::Result<PathBuf> {
        let target = if path.as_ref().is_absolute() {
            path.as_ref().to_path_buf()
        } else {
            self.project_root.join(path)
        };

        let canonical_target = std::fs::canonicalize(&target)?;

        if canonical_target.starts_with(&self.project_root) {
            Ok(canonical_target)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Path is outside the project workspace boundary",
            ))
        }
    }
}
