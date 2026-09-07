use super::app_paths::AppPaths;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Runtime configuration - manages all subsystem paths and health discovery.
/// Corresponds to the original `runtime-config.ts`.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub workspace_root: PathBuf,
    pub understand_anything_root: PathBuf,
    pub shadow_git_root: PathBuf,
    pub project_root: PathBuf,
    pub projects_root: PathBuf,
    pub codebase_memory_cache_root: PathBuf,
    pub codebase_memory_bin: PathBuf,
    /// Path to the rtk (Rust Token Killer) binary, invoked as a subprocess for
    /// command-output filtering and agent-hook installation.
    pub rtk_bin: PathBuf,
    /// Base URL for SSO Token Introspection
    pub sso_api_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub key: String,
    pub label: String,
    pub state: HealthState,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Ok,
    Warning,
    Error,
}

impl RuntimeConfig {
    pub fn from_env(app_paths: &AppPaths) -> Self {
        let workspace_root = Self::env_path("HUB_WORKSPACE_ROOT").unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("..")
        });

        let understand_anything_root = Self::env_path("HUB_UNDERSTAND_ANYTHING_ROOT")
            .unwrap_or_else(|| {
                workspace_root
                    .join("Understand-Anything")
                    .join("understand-anything-plugin")
            });

        let shadow_git_root = Self::env_path("HUB_SHADOW_GIT_ROOT").unwrap_or_else(|| {
            if AppPaths::is_bundled_runtime() {
                app_paths.data_path(&["shadow-git"])
            } else {
                workspace_root.join(".shadow-git")
            }
        });

        let project_root =
            Self::env_path("HUB_PROJECT_ROOT").unwrap_or_else(|| workspace_root.clone());

        let projects_root = Self::env_path("HUB_PROJECTS_ROOT")
            .unwrap_or_else(|| app_paths.data_path(&["projects"]));

        let codebase_memory_cache_root = Self::env_path("HUB_CODEBASE_MEMORY_CACHE_ROOT")
            .unwrap_or_else(|| {
                if AppPaths::is_bundled_runtime() {
                    app_paths.data_path(&["cache", "codebase-memory-mcp"])
                } else {
                    let home = std::env::var("USERPROFILE")
                        .or_else(|_| std::env::var("HOME"))
                        .unwrap_or_default();
                    PathBuf::from(home)
                        .join(".cache")
                        .join("codebase-memory-mcp")
                }
            });

        let codebase_memory_bin = Self::resolve_cbm_binary(app_paths);
        let rtk_bin = Self::resolve_rtk_binary(app_paths, &workspace_root);

        let sso_api_url = std::env::var("DEEPCLOUD_API_URL")
            .unwrap_or_else(|_| "https://api.supermemory.ai".to_string());

        Self {
            workspace_root,
            understand_anything_root,
            shadow_git_root,
            project_root,
            projects_root,
            codebase_memory_cache_root,
            codebase_memory_bin,
            rtk_bin,
            sso_api_url,
        }
    }

    /// Get the codebase-memory DB path for a given project
    pub fn codebase_memory_db_path(&self, project_path: Option<&str>) -> PathBuf {
        if let Some(explicit) = Self::env_path("HUB_CODEBASE_MEMORY_DB_PATH") {
            return explicit;
        }
        let repo_root = project_path
            .map(PathBuf::from)
            .unwrap_or_else(|| self.project_root.clone());

        let slug = Self::project_cache_slug(&repo_root);

        self.codebase_memory_cache_root.join(format!("{}.db", slug))
    }

    /// Get shadow git root for a specific project
    pub fn shadow_git_root_for(&self, project_path: Option<&str>) -> PathBuf {
        if let Some(p) = project_path {
            return PathBuf::from(p).join(".shadow-git");
        }
        self.shadow_git_root.clone()
    }

    /// Get supermemory store path
    pub fn supermemory_store_path(&self, project_path: Option<&str>) -> PathBuf {
        if let Some(p) = project_path {
            return PathBuf::from(p)
                .join(".supermemory")
                .join("supermemory.json");
        }
        self.shadow_git_root.join("supermemory_store.json")
    }

    /// Get knowledge graph file candidates
    pub fn knowledge_graph_candidates(&self, project_path: Option<&str>) -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        if let Some(explicit) = Self::env_path("HUB_KNOWLEDGE_GRAPH_PATH") {
            candidates.push(explicit);
        }

        if let Some(p) = project_path {
            let base = PathBuf::from(p);
            candidates.push(
                base.join(".understand-anything")
                    .join("knowledge-graph.json"),
            );
            candidates.push(base.join(".supermemory").join("knowledge-graph.json"));
        }

        candidates.push(
            self.understand_anything_root
                .join("packages")
                .join("dashboard")
                .join("public")
                .join("knowledge-graph.json"),
        );

        candidates
    }

    /// Find the first existing knowledge graph file
    pub fn knowledge_graph_path(&self, project_path: Option<&str>) -> Option<PathBuf> {
        self.knowledge_graph_candidates(project_path)
            .into_iter()
            .find(|p| p.exists())
    }

    /// Compute health statuses for all subsystems
    pub fn runtime_statuses(&self, project_path: Option<&str>) -> Vec<RuntimeStatus> {
        let cbm_db = self.codebase_memory_db_path(project_path);
        let knowledge_graph = self.knowledge_graph_path(project_path);
        let shadow_git = self.shadow_git_root_for(project_path);
        let supermemory_store = self.supermemory_store_path(project_path);

        let llm_settings = super::llm_settings::read_llm_settings();
        let llm_ready = super::llm_settings::has_usable_settings(&llm_settings);

        let statuses = vec![
            RuntimeStatus {
                key: "llm-provider".into(),
                label: "LLM Provider".into(),
                state: if llm_ready {
                    HealthState::Ok
                } else {
                    HealthState::Warning
                },
                message: if llm_ready {
                    format!(
                        "Configured for model {} via {}.",
                        llm_settings.model, llm_settings.base_url
                    )
                } else if llm_settings.enabled {
                    "LLM is enabled but base URL, API key, or model is incomplete.".into()
                } else {
                    "LLM is not configured yet.".into()
                },
                path: None,
            },
            RuntimeStatus {
                key: "projects-root".into(),
                label: "Projects Root".into(),
                state: HealthState::Ok,
                message: "Git repositories are cloned here before analysis.".into(),
                path: Some(self.projects_root.to_string_lossy().into()),
            },
            RuntimeStatus {
                key: "codebase-memory-db".into(),
                label: "Codebase Memory DB".into(),
                state: if cbm_db.exists() {
                    HealthState::Ok
                } else {
                    HealthState::Warning
                },
                message: if cbm_db.exists() {
                    "Indexed graph database detected.".into()
                } else {
                    "Graph database not found. Run project analysis first.".into()
                },
                path: Some(cbm_db.to_string_lossy().into()),
            },
            RuntimeStatus {
                key: "understand-anything-metadata".into(),
                label: "Understand-Anything Metadata".into(),
                state: if knowledge_graph.is_some() {
                    HealthState::Ok
                } else {
                    HealthState::Warning
                },
                message: if let Some(ref kg) = knowledge_graph {
                    format!("Knowledge graph metadata detected from {}.", kg.display())
                } else {
                    "Metadata file not found. Summary/tags/complexity will be unavailable.".into()
                },
                path: knowledge_graph.as_ref().map(|p| p.to_string_lossy().into()),
            },
            RuntimeStatus {
                key: "shadow-git".into(),
                label: "Shadow Git Workspace".into(),
                state: if shadow_git.exists() {
                    HealthState::Ok
                } else {
                    HealthState::Warning
                },
                message: if shadow_git.exists() {
                    "Shadow Git workspace detected.".into()
                } else {
                    "Shadow Git workspace not found. AI history views may degrade.".into()
                },
                path: Some(shadow_git.to_string_lossy().into()),
            },
            RuntimeStatus {
                key: "supermemory-chat".into(),
                label: "Supermemory Chat".into(),
                state: if supermemory_store.exists() || knowledge_graph.is_some() {
                    HealthState::Ok
                } else {
                    HealthState::Warning
                },
                message: if supermemory_store.exists() {
                    "Local supermemory sync is available.".into()
                } else {
                    "Supermemory store not initialized yet.".into()
                },
                path: if supermemory_store.exists() {
                    Some(supermemory_store.to_string_lossy().into())
                } else {
                    None
                },
            },
        ];

        statuses
    }

    fn resolve_cbm_binary(app_paths: &AppPaths) -> PathBuf {
        let ext = if cfg!(windows) { ".exe" } else { "" };
        let bin_name = format!("codebase-memory-mcp{}", ext);

        // Check explicit env
        if let Some(explicit) = Self::env_path("HUB_CODEBASE_MEMORY_BIN") {
            if explicit.exists() {
                return explicit;
            }
        }

        // Check bundled location
        let bundled = app_paths
            .bundled_mcp_tools_root()
            .join("codebase-memory-mcp")
            .join(&bin_name);
        if bundled.exists() {
            return bundled;
        }

        // Fallback to PATH
        PathBuf::from(&bin_name)
    }

    /// Resolve the rtk binary. `hub` is itself the rtk hook, so this prefers a
    /// self-hosted shim (`{app_data}/bin/rtk`, a copy of the running `hub`).
    /// The explicit `HUB_RTK_BIN` override still wins for advanced setups, and
    /// legacy bundled / workspace builds remain as fallbacks.
    fn resolve_rtk_binary(app_paths: &AppPaths, workspace_root: &Path) -> PathBuf {
        let ext = if cfg!(windows) { ".exe" } else { "" };
        let bin_name = format!("rtk{}", ext);

        if let Some(explicit) = Self::env_path("HUB_RTK_BIN") {
            if explicit.exists() {
                return explicit;
            }
        }

        // Primary path: hub hosts its own rtk shim (no separate binary shipped).
        let shim = super::rtk_shim::ensure_shim(app_paths);
        if shim.exists() {
            return shim;
        }

        let bundled = app_paths
            .bundled_mcp_tools_root()
            .join("rtk")
            .join(&bin_name);
        if bundled.exists() {
            return bundled;
        }

        // Workspace build output (dev): super-agent/rtk/target/release/rtk.exe
        let workspace_build = workspace_root
            .join("rtk")
            .join("target")
            .join("release")
            .join(&bin_name);
        if workspace_build.exists() {
            return workspace_build;
        }

        // Fallback to PATH
        PathBuf::from(&bin_name)
    }

    fn env_path(key: &str) -> Option<PathBuf> {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }

    fn project_cache_slug(project_path: &Path) -> String {
        let raw = project_path.to_string_lossy();
        // `std::fs::canonicalize` returns verbatim paths on Windows. Strip the
        // transport prefix so the same workspace maps to the same cache DB,
        // whether the caller supplied `C:\\repo` or `\\?\\C:\\repo`.
        let normalized = raw
            .strip_prefix(r"\\?\UNC\")
            .map(|rest| format!(r"\\{rest}"))
            .or_else(|| raw.strip_prefix(r"\\?\").map(str::to_string))
            .unwrap_or_else(|| raw.to_string());
        normalized
            .replace(":\\", "-")
            .replace(['\\', '/', ':'], "-")
            .chars()
            .map(|character| {
                if character.is_alphanumeric() || matches!(character, '-' | '_' | '.') {
                    character
                } else {
                    '-'
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_slug_is_stable_for_windows_verbatim_paths() {
        assert_eq!(
            RuntimeConfig::project_cache_slug(Path::new(r"C:\work\demo")),
            RuntimeConfig::project_cache_slug(Path::new(r"\\?\C:\work\demo"))
        );
        let slug = RuntimeConfig::project_cache_slug(Path::new(r"\\?\C:\work\demo"));
        assert!(!slug.contains('?'));
        assert!(!slug.contains(':'));
        assert!(!slug.contains('\\'));
    }
}
