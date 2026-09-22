use std::path::PathBuf;

/// Application data paths management.
/// Corresponds to the original `app-paths.ts`.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub bundle_root: PathBuf,
    pub app_data_root: PathBuf,
}

impl AppPaths {
    pub fn new() -> Self {
        let bundle_root = Self::resolve_env_path("HUB_BUNDLE_ROOT").unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("..")
                .join("..")
        });

        let app_data_root =
            Self::resolve_env_path("HUB_APP_DATA_ROOT").unwrap_or_else(Self::default_app_data_root);

        Self {
            bundle_root,
            app_data_root,
        }
    }

    pub fn home_dir() -> Option<PathBuf> {
        let home = std::env::var("HUB_APP_DATA_ROOT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if home.is_some() {
            return home.map(PathBuf::from);
        }

        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }

    /// Stable, cwd-independent app data root: `{HOME|USERPROFILE}/.aindex-hub`.
    /// Falls back to `cwd/.aindex-hub` only when no home directory is available.
    fn default_app_data_root() -> PathBuf {
        match Self::home_dir() {
            Some(h) => h.join(".gemini").join("antigravity"),
            None => std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".gemini").join("antigravity"),
        }
    }

    /// One-time migration: copy settings from the legacy cwd-relative location
    /// (`cwd/.aindex-hub`) into the stable app data root if they only exist there.
    pub fn migrate_legacy_data(&self) {
        let legacy_root = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".gemini").join("antigravity");
        if legacy_root == self.app_data_root {
            return;
        }
        let legacy_llm = legacy_root.join("settings").join("llm.json");
        let new_llm = self.data_path(&["settings", "llm.json"]);
        if legacy_llm.exists() && !new_llm.exists() {
            self.ensure_data_dir(&["settings"]);
            match std::fs::copy(&legacy_llm, &new_llm) {
                Ok(_) => tracing::info!(
                    "Migrated LLM settings from {} to {}",
                    legacy_llm.display(),
                    new_llm.display()
                ),
                Err(e) => tracing::warn!("Failed to migrate LLM settings: {}", e),
            }
        }
    }

    /// Check if running in bundled mode
    pub fn is_bundled_runtime() -> bool {
        std::env::var("HUB_BUNDLE_ROOT").is_ok() || std::env::var("HUB_APP_DATA_ROOT").is_ok()
    }

    /// Get a path under the app data root
    pub fn data_path(&self, segments: &[&str]) -> PathBuf {
        let mut path = self.app_data_root.clone();
        for seg in segments {
            path = path.join(seg);
        }
        path
    }

    /// Ensure a directory exists under app data root
    pub fn ensure_data_dir(&self, segments: &[&str]) -> PathBuf {
        let path = self.data_path(segments);
        if !path.exists() {
            std::fs::create_dir_all(&path).ok();
        }
        path
    }

    /// Get the bundled MCP tools root
    pub fn bundled_mcp_tools_root(&self) -> PathBuf {
        self.bundle_root.join("mcp-tools")
    }

    fn resolve_env_path(key: &str) -> Option<PathBuf> {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
}

impl Default for AppPaths {
    fn default() -> Self {
        Self::new()
    }
}
