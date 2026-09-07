use std::fs;
use std::path::{Path, PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::adapter::SyncError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub transaction_id: String,
    pub timestamp: String,
    pub agent_kind: String,
    pub files: Vec<String>,
}

pub fn get_history_dir(agent_kind: &str) -> PathBuf {
    let home = crate::config::app_paths::AppPaths::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".deepcloud").join("history").join(agent_kind)
}

pub fn save_history(
    agent_kind: &str,
    transaction_id: &str,
    snapshots: &std::collections::HashMap<PathBuf, Option<Vec<u8>>>,
) -> Result<(), SyncError> {
    let history_dir = get_history_dir(agent_kind).join(transaction_id);
    if let Err(e) = fs::create_dir_all(&history_dir) {
        return Err(SyncError::AtomicWriteFailed(format!("Failed to create history dir: {}", e)));
    }

    let mut record = HistoryRecord {
        transaction_id: transaction_id.to_string(),
        timestamp: Utc::now().to_rfc3339(),
        agent_kind: agent_kind.to_string(),
        files: Vec::new(),
    };

    for (idx, (path, content)) in snapshots.iter().enumerate() {
        let abs_path = path.to_string_lossy().to_string();
        record.files.push(abs_path.clone());

        let file_name = format!("{}.backup", idx);
        let backup_path = history_dir.join(&file_name);

        if let Some(bytes) = content {
            if let Err(e) = fs::write(&backup_path, bytes) {
                return Err(SyncError::AtomicWriteFailed(format!("Failed to write backup {}: {}", backup_path.display(), e)));
            }
        } else {
            let _ = fs::write(history_dir.join(format!("{}.not_existed", idx)), b"");
        }
    }

    let metadata_path = history_dir.join("metadata.json");
    let metadata_json = serde_json::to_string(&record).unwrap_or_default();
    if let Err(e) = fs::write(&metadata_path, metadata_json) {
        return Err(SyncError::AtomicWriteFailed(format!("Failed to write history metadata: {}", e)));
    }

    Ok(())
}

pub fn list_history(agent_kind: &str) -> Vec<HistoryRecord> {
    let mut records = Vec::new();
    let history_dir = get_history_dir(agent_kind);

    if let Ok(entries) = fs::read_dir(history_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                let metadata_path = entry.path().join("metadata.json");
                if let Ok(content) = fs::read_to_string(&metadata_path) {
                    if let Ok(record) = serde_json::from_str::<HistoryRecord>(&content) {
                        records.push(record);
                    }
                }
            }
        }
    }

    records.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    records
}

pub fn restore_history(agent_kind: &str, transaction_id: &str) -> Result<(), SyncError> {
    let history_dir = get_history_dir(agent_kind).join(transaction_id);
    let metadata_path = history_dir.join("metadata.json");

    let content = fs::read_to_string(&metadata_path)
        .map_err(|e| SyncError::RollbackFailed(format!("Cannot read history metadata: {}", e)))?;

    let record: HistoryRecord = serde_json::from_str(&content)
        .map_err(|e| SyncError::RollbackFailed(format!("Invalid history metadata: {}", e)))?;

    let mut errs = Vec::new();

    for (idx, file_path_str) in record.files.iter().enumerate() {
        let target_path = PathBuf::from(file_path_str);
        let backup_path = history_dir.join(format!("{}.backup", idx));
        let not_existed_marker = history_dir.join(format!("{}.not_existed", idx));

        if not_existed_marker.exists() {
            if target_path.exists() {
                if let Err(e) = fs::remove_file(&target_path) {
                    errs.push(format!("Failed to delete {}: {}", target_path.display(), e));
                }
            }
        } else if backup_path.exists() {
            if let Ok(parent) = target_path.parent().ok_or(()) {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::copy(&backup_path, &target_path) {
                errs.push(format!("Failed to copy {} to {}: {}", backup_path.display(), target_path.display(), e));
            }
        } else {
            errs.push(format!("Missing backup file for {}", target_path.display()));
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(SyncError::RollbackFailed(errs.join("; ")))
    }
}
