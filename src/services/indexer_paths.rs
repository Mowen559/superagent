//! Adapt only the native indexer's transport paths. Project ownership and
//! receipts continue to use the original workspace, never the temporary alias.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub struct IndexerPaths {
    pub project: PathBuf,
    pub cache: PathBuf,
    staging: Option<tempfile::TempDir>,
    junction: Option<PathBuf>,
}

impl IndexerPaths {
    pub fn prepare(project: &Path, cache: &Path, executable: &Path) -> Result<Self, String> {
        let transport_project = PathBuf::from(crate::infrastructure::fs_path::strip_unc_prefix(
            &project.to_string_lossy(),
        ));
        let project = transport_project.as_path();
        let native_indexer=executable.file_name().is_some_and(|name|name.to_string_lossy().eq_ignore_ascii_case("codebase-memory-mcp.exe"));
        if !cfg!(windows) || !native_indexer
            || (project.to_string_lossy().is_ascii() && cache.to_string_lossy().is_ascii())
        {
            return Ok(Self {
                project: project.to_path_buf(),
                cache: cache.to_path_buf(),
                staging: None,
                junction: None,
            });
        }
        let binary = if executable.is_file() {
            executable.to_path_buf()
        } else {
            std::env::var_os("PATH")
                .and_then(|paths| {
                    std::env::split_paths(&paths)
                        .map(|dir| dir.join(executable))
                        .find(|p| p.is_file())
                })
                .ok_or("Native indexer executable was not found")?
        };
        let base = std::env::var_os("HUB_INDEX_STAGING_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                binary
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(".hub-index-staging")
            });
        if !base.is_absolute() || !base.to_string_lossy().is_ascii() {
            return Err("Windows native indexer needs a writable ASCII staging path; configure HUB_INDEX_STAGING_ROOT".into());
        }
        std::fs::create_dir_all(&base).map_err(
            |_| "Cannot create index staging directory; configure HUB_INDEX_STAGING_ROOT",
        )?;
        let stage = tempfile::Builder::new()
            .prefix("run-")
            .tempdir_in(&base)
            .map_err(|_| "Cannot allocate index staging directory")?;
        // Validate the final cleanup target before any recursive TempDir drop.
        let checked = stage
            .path()
            .canonicalize()
            .map_err(|_| "Cannot resolve staging path")?;
        let checked_base = base
            .canonicalize()
            .map_err(|_| "Cannot resolve staging root")?;
        if checked.parent() != Some(checked_base.as_path()) {
            let _ = stage.keep();
            return Err("Index staging escaped its configured root".into());
        }
        let alias = stage.path().join("source");
        let cache = stage.path().join("cache");
        let paths = Self {
            project: alias.clone(),
            cache,
            staging: Some(stage),
            junction: Some(alias.clone()),
        };
        let quote = |p: &Path| p.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference='Stop'; $null=New-Item -ItemType Junction -Path '{}' -Target '{}'",
            quote(&alias),
            quote(project)
        );
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "Cannot launch Windows junction adapter")?;
        if !status.success() {
            return Err("Cannot create source junction for native indexer".into());
        }
        std::fs::create_dir_all(&paths.cache).map_err(|_| "Cannot create index staging cache")?;
        Ok(paths)
    }

    pub fn publish(&self, destination: &Path) -> Result<(), String> {
        let source = if self.staging.is_some() {
            let files = std::fs::read_dir(&self.cache)
                .map_err(|_| "Indexer produced no cache")?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "db"))
                .take(3)
                .collect::<Vec<_>>();
            if files.len() != 1 {
                return Err("Indexer did not produce exactly one project graph database".into());
            }
            files[0].clone()
        } else {
            destination.to_path_buf()
        };
        validate_database(&source)?;
        if source != destination {
            let parent = destination
                .parent()
                .ok_or("Graph cache destination has no parent")?;
            std::fs::create_dir_all(parent).map_err(|_| "Cannot create project graph cache")?;
            let temporary = tempfile::NamedTempFile::new_in(parent)
                .map_err(|_| "Cannot stage graph publication")?;
            std::fs::copy(&source, temporary.path()).map_err(|_| "Cannot copy validated graph")?;
            temporary
                .persist(destination)
                .map_err(|_| "Cannot atomically publish validated graph")?;
        }
        Ok(())
    }
}

impl Drop for IndexerPaths {
    fn drop(&mut self) {
        if let Some(link) = &self.junction {
            // remove_dir removes the junction itself. Never recursively visit
            // its target; if unlink fails, retain the staging tree for audit.
            if link.exists() && std::fs::remove_dir(link).is_err() {
                if let Some(stage) = self.staging.take() {
                    let _ = stage.keep();
                }
            }
        }
    }
}

pub fn validate_database(path: &Path) -> Result<i64, String> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "Graph database is missing or unreadable")?;
    conn.prepare("SELECT source_id,target_id,type FROM edges LIMIT 0")
        .map_err(|_| "Graph edges schema is invalid")?;
    let count: i64 = conn
        .query_row("SELECT count(*) FROM nodes", [], |row| row.get(0))
        .map_err(|_| "Graph nodes schema is invalid")?;
    if count <= 0 {
        return Err("Graph database contains no nodes".into());
    }
    Ok(count)
}

pub fn index_project(
    config: &crate::config::runtime::RuntimeConfig,
    project: &Path,
) -> Result<serde_json::Value, String> {
    let project = project
        .canonicalize()
        .map_err(|_| "Project directory cannot be opened")?;
    let paths = IndexerPaths::prepare(
        &project,
        &config.codebase_memory_cache_root,
        &config.codebase_memory_bin,
    )?;
    let stdout = tempfile::tempfile().map_err(|_| "Cannot capture indexer stdout")?;
    let stderr = tempfile::tempfile().map_err(|_| "Cannot capture indexer stderr")?;
    let payload = serde_json::json!({"repo_path":paths.project,"mode":"fast"}).to_string();
    let mut child = Command::new(&config.codebase_memory_bin)
        .args(["cli", "index_repository", &payload])
        .current_dir(&paths.project)
        .env("CBM_CACHE_DIR", &paths.cache)
        .stdin(Stdio::null())
        .stdout(
            stdout
                .try_clone()
                .map_err(|_| "Cannot capture index output")?,
        )
        .stderr(
            stderr
                .try_clone()
                .map_err(|_| "Cannot capture index errors")?,
        )
        .spawn()
        .map_err(|_| "Native indexer could not be started")?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_secs(120) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Native indexer timed out or wait failed".into());
            }
        }
    };
    if !status.success() {
        return Err("Native indexer failed; existing project graph was preserved".into());
    }
    let db = config.codebase_memory_db_path(Some(&project.to_string_lossy()));
    paths.publish(&db)?;
    Ok(
        serde_json::json!({"status":"indexed","project":project,"database_path":db,"nodes":validate_database(&db)?,"ascii_path_adapter":paths.staging.is_some(),"duration_ms":started.elapsed().as_millis()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_staged_graph_never_overwrites_existing_cache() {
        let stage=tempfile::tempdir().unwrap();
        let destination=tempfile::tempdir().unwrap();
        std::fs::write(stage.path().join("candidate.db"),"broken SQLite").unwrap();
        let target=destination.path().join("project.db");
        std::fs::write(&target,"last known graph bytes").unwrap();
        let paths=IndexerPaths{project:stage.path().join("unused"),cache:stage.path().to_path_buf(),staging:Some(stage),junction:None};
        assert!(paths.publish(&target).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(),"last known graph bytes");
    }
}
