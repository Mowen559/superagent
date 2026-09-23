use crate::infrastructure::fs_path::{normalize_path, resolve_safe_path};
use std::path::{Path, PathBuf};

pub struct WorkspaceGuard {
    project_root: PathBuf,
}

impl WorkspaceGuard {
    pub fn new<P: AsRef<Path>>(project_root: P) -> std::io::Result<Self> {
        let canonical_root = std::fs::canonicalize(project_root)?;
        Ok(Self {
            project_root: normalize_path(&canonical_root),
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 校验并解析工作区内的安全路径（自动剥离 Windows `\\?\` 前缀并支持待新建文件的父目录安全校验）
    pub fn resolve_safe<P: AsRef<Path>>(&self, path: P) -> std::io::Result<PathBuf> {
        let raw_str = path.as_ref().to_string_lossy();
        resolve_safe_path(&raw_str, &self.project_root).map_err(|msg| {
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, msg)
        })
    }

    /// 便捷辅助方法：从根工作区目录直接校验并解析目标项目路径字符串
    pub fn resolve_project_str<R: AsRef<Path>>(
        projects_root: R,
        project: &str,
    ) -> Result<String, String> {
        let guard = Self::new(projects_root).map_err(|e| format!("Invalid root: {}", e))?;
        let resolved = guard
            .resolve_safe(project)
            .map_err(|e| format!("Invalid project path: {}", e))?;
        Ok(resolved.to_string_lossy().to_string())
    }
}
