use crate::services::git_service::{self, RepoKind};
use crate::services::workspace_guard::WorkspaceGuard;

pub struct DestructiveOps<'a> {
    guard: &'a WorkspaceGuard,
}

impl<'a> DestructiveOps<'a> {
    pub fn new(guard: &'a WorkspaceGuard) -> Self {
        Self { guard }
    }

    pub fn git_restore(
        &self,
        project: &str,
        file_path: &str,
        hash: Option<&str>,
        kind: RepoKind,
    ) -> Result<(), String> {
        let safe_project = self
            .guard
            .resolve_safe(project, "")
            .map_err(|e| format!("Security violation (project): {}", e))?;

        let safe_file = self
            .guard
            .resolve_safe(project, file_path)
            .map_err(|e| format!("Security violation (file): {}", e))?;

        tracing::info!(
            "DESTRUCTIVE OP: git restore in {} for file {} (hash: {:?})",
            safe_project.display(),
            safe_file.display(),
            hash
        );
        git_service::restore_file(project, file_path, hash, kind)
    }

    pub fn shadow_rollback(&self, project: &str, hash: &str) -> Result<(), String> {
        let safe_project = self
            .guard
            .resolve_safe(project, "")
            .map_err(|e| format!("Security violation (project): {}", e))?;

        tracing::info!(
            "DESTRUCTIVE OP: shadow rollback in {} to hash {}",
            safe_project.display(),
            hash
        );
        git_service::rollback_to_snapshot(project, hash)
    }

    pub fn apply_patch(&self, project: &str, patch_content: &str) -> Result<(), String> {
        let safe_project = self
            .guard
            .resolve_safe(project, "")
            .map_err(|e| format!("Security violation (project): {}", e))?;

        tracing::info!(
            "DESTRUCTIVE OP: git apply patch in {}",
            safe_project.display()
        );
        git_service::apply_patch(project, patch_content)
    }
}
