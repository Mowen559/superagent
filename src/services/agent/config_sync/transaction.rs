use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::adapter::SyncError;

pub struct ConfigTransactionManager {
    snapshots: HashMap<PathBuf, Option<Vec<u8>>>,
    locks: Vec<(PathBuf, std::fs::File)>,
}

impl Drop for ConfigTransactionManager {
    fn drop(&mut self) {
        // Unlock files by deleting the .lock file
        for (path, _) in self.locks.drain(..) {
            let _ = fs::remove_file(path);
        }
    }
}

impl ConfigTransactionManager {
    pub fn new() -> Self {
        Self {
            snapshots: HashMap::new(),
            locks: Vec::new(),
        }
    }

    /// Acquire locks for the given files using atomic file creation.
    pub fn lock(&mut self, files: &[String]) -> Result<(), SyncError> {
        for file in files {
            let path = PathBuf::from(file);
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Err(SyncError::AtomicWriteFailed(format!("Failed to create parent dir: {}", e)));
                }
            }

            let lock_name = format!(
                ".{}.lock",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            let lock_path = parent.join(lock_name);

            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(file) => {
                    self.locks.push((lock_path, file));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(SyncError::ConfigLocked);
                }
                Err(e) => {
                    return Err(SyncError::AtomicWriteFailed(format!(
                        "Failed to create lock file {}: {}",
                        lock_path.display(),
                        e
                    )));
                }
            }
        }
        Ok(())
    }

    /// Snapshot files before modifying
    pub fn snapshot(&mut self, files: &[String]) -> Result<(), SyncError> {
        for file in files {
            let path = PathBuf::from(file);
            let content = if path.exists() {
                match fs::read(&path) {
                    Ok(c) => Some(c),
                    Err(e) => return Err(SyncError::ConfigNotReadable(e.to_string())),
                }
            } else {
                None
            };
            self.snapshots.insert(path, content);
        }
        Ok(())
    }

    /// Commit the mutation plan. Writes to temp files, then renames.
    pub fn commit(&self, files_to_write: &HashMap<String, String>) -> Result<(), SyncError> {
        // Prepare phase: Write all to temp files
        let mut temp_files = HashMap::new();

        for (file, content) in files_to_write {
            let target_path = PathBuf::from(file);
            let parent = target_path.parent().unwrap_or_else(|| Path::new("."));

            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Err(SyncError::AtomicWriteFailed(format!(
                        "Failed to create parent dir: {}",
                        e
                    )));
                }
            }

            let tmp_name = format!(
                ".{}.tmp",
                target_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
            let tmp_path = parent.join(tmp_name);

            // Write and fsync
            let mut file = match fs::File::create(&tmp_path) {
                Ok(f) => f,
                Err(e) => {
                    return Err(SyncError::AtomicWriteFailed(format!(
                        "Failed to create tmp file {}: {}",
                        tmp_path.display(),
                        e
                    )));
                }
            };
            use std::io::Write;
            if let Err(e) = file.write_all(content.as_bytes()) {
                return Err(SyncError::AtomicWriteFailed(format!("Failed to write tmp file: {}", e)));
            }
            if let Err(e) = file.sync_all() {
                return Err(SyncError::AtomicWriteFailed(format!("Failed to fsync tmp file: {}", e)));
            }

            temp_files.insert(target_path, tmp_path);
        }

        // Commit phase: Rename temp files to target
        for (target, tmp) in temp_files {
            if let Err(e) = fs::rename(&tmp, &target) {
                // Try to clean up temp file if rename failed
                let _ = fs::remove_file(&tmp);
                return Err(SyncError::AtomicWriteFailed(format!(
                    "Failed to rename {} to {}: {}",
                    tmp.display(),
                    target.display(),
                    e
                )));
            }
        }

        Ok(())
    }

    /// Rollback changes using snapshots.
    pub fn rollback(&self) -> Result<(), SyncError> {
        let mut errs = Vec::new();
        for (path, original_content) in &self.snapshots {
            if let Some(content) = original_content {
                // Restore file
                let tmp_name = format!(
                    ".{}.rollback.tmp",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                let tmp_path = path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(tmp_name);

                // Write and fsync rollback
                if let Ok(mut file) = fs::File::create(&tmp_path) {
                    use std::io::Write;
                    let _ = file.write_all(content);
                    let _ = file.sync_all();
                } else {
                    errs.push(format!("Write tmp {}", tmp_path.display()));
                    continue;
                }

                if let Err(e) = fs::rename(&tmp_path, path) {
                    errs.push(format!(
                        "Rename {} to {}: {}",
                        tmp_path.display(),
                        path.display(),
                        e
                    ));
                }
            } else {
                // File didn't exist originally, delete it if it exists now
                if path.exists() {
                    if let Err(e) = fs::remove_file(path) {
                        errs.push(format!("Delete {}: {}", path.display(), e));
                    }
                }
            }
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(SyncError::RollbackFailed(errs.join("; ")))
        }
    }

    /// Save the snapshot into persistent history
    pub fn save_history(&self, agent_kind: &str, transaction_id: &str) -> Result<(), SyncError> {
        crate::services::agent::config_sync::history::save_history(
            agent_kind,
            transaction_id,
            &self.snapshots,
        )
    }
}
