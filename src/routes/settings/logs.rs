use axum::{routing::{get, post}, Json, Router, extract::State};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use walkdir::WalkDir;
use crate::AppState;

#[derive(Serialize)]
pub struct DiskUsage {
    pub size_bytes: u64,
    pub threshold_bytes: u64,
    pub exceeds_threshold: bool,
    pub file_count: usize,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/system/logs/disk_usage", get(get_disk_usage))
        .route("/api/system/logs/clear", post(clear_logs))
}

fn get_tasks_dir(state: &AppState) -> PathBuf {
    state.app_paths.ensure_data_dir(&["tasks"])
}

async fn get_disk_usage(State(state): State<Arc<AppState>>) -> Json<DiskUsage> {
    let dir = get_tasks_dir(&state);
    let mut total_size = 0;
    let mut file_count = 0;

    if dir.exists() {
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Ok(metadata) = entry.metadata() {
                    total_size += metadata.len();
                    file_count += 1;
                }
            }
        }
    }

    let threshold_bytes = 500 * 1024 * 1024; // 500 MB threshold
    Json(DiskUsage {
        size_bytes: total_size,
        threshold_bytes,
        exceeds_threshold: total_size > threshold_bytes,
        file_count,
    })
}

#[derive(Deserialize)]
pub struct ClearLogsPayload {
    pub max_age_days: Option<u64>,
}

async fn clear_logs(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ClearLogsPayload>,
) -> Json<serde_json::Value> {
    let dir = get_tasks_dir(&state);
    if !dir.exists() {
        return Json(serde_json::json!({ "success": true, "deleted": 0 }));
    }

    let max_age = payload.max_age_days.unwrap_or(0);
    let now = std::time::SystemTime::now();
    let mut deleted_count = 0;

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(metadata) = entry.metadata() {
                let should_delete = if max_age > 0 {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(duration) = now.duration_since(modified) {
                            duration.as_secs() > max_age * 24 * 3600
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    true // Delete all if max_age is 0 or missing
                };

                if should_delete {
                    if std::fs::remove_file(entry.path()).is_ok() {
                        deleted_count += 1;
                    }
                }
            }
        }
    }

    Json(serde_json::json!({ "success": true, "deleted_files": deleted_count }))
}
