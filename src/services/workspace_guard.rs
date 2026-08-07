use std::path::{Path, PathBuf};

pub struct WorkspaceGuard {
    projects_root: PathBuf,
}

impl WorkspaceGuard {
    pub fn new<P: AsRef<Path>>(projects_root: P) -> std::io::Result<Self> {
        let canonical_root = std::fs::canonicalize(projects_root)?;
        Ok(Self {
            projects_root: canonical_root,
        })
    }

    pub fn resolve_safe<P: AsRef<Path>>(&self, project_name: &str, path: P) -> std::io::Result<PathBuf> {
        // Compute the expected project boundary
        let project_root = self.projects_root.join(project_name);
        
        // If the project doesn't exist, canonicalize will fail, which is safe.
        // We ensure project_root is safely within projects_root
        let canonical_project_root = std::fs::canonicalize(&project_root)?;
        if !canonical_project_root.starts_with(&self.projects_root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Project root is outside the projects directory boundary",
            ));
        }

        // Construct the path relative to project if it's relative
        let target = if path.as_ref().is_absolute() {
            path.as_ref().to_path_buf()
        } else {
            canonical_project_root.join(path)
        };

        // Canonicalize it to resolve any `..` components
        let canonical_target = std::fs::canonicalize(&target)?;

        // Check if it starts with the project root
        if canonical_target.starts_with(&canonical_project_root) {
            Ok(canonical_target)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Path is outside the project workspace boundary",
            ))
        }
    }
}
