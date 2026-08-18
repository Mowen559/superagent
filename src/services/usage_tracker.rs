use rusqlite::{Connection, OpenFlags};
use std::sync::Once;
use std::path::PathBuf;
use crate::config::app_paths::AppPaths;

static INIT: Once = Once::new();

pub fn init_db() {
    INIT.call_once(|| {
        let db_path = get_db_path();
        
        // Ensure data directory exists
        let paths = AppPaths::new();
        paths.ensure_data_dir(&["data"]);
        
        if let Ok(conn) = Connection::open(&db_path) {
            let _ = conn.execute(
                "CREATE TABLE IF NOT EXISTS api_usage_logs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                    account_id TEXT NOT NULL,
                    model TEXT NOT NULL,
                    prompt_tokens INTEGER NOT NULL,
                    completion_tokens INTEGER NOT NULL,
                    total_cost REAL NOT NULL
                )",
                [],
            );
        } else {
            tracing::error!("Failed to create usage.db at {:?}", db_path);
        }
    });
}

fn get_db_path() -> PathBuf {
    AppPaths::new().data_path(&["data", "usage.db"])
}

pub fn record_usage(account_id: String, model: String, prompt_tokens: u32, completion_tokens: u32, total_cost: f64) {
    if prompt_tokens == 0 && completion_tokens == 0 {
        return; // Nothing to record
    }
    
    // Fire and forget, don't block the async runtime
    tokio::task::spawn_blocking(move || {
        let db_path = get_db_path();
        match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE) {
            Ok(conn) => {
                let res = conn.execute(
                    "INSERT INTO api_usage_logs (account_id, model, prompt_tokens, completion_tokens, total_cost) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![account_id, model, prompt_tokens, completion_tokens, total_cost],
                );
                if let Err(e) = res {
                    tracing::error!("Failed to insert usage log: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to open usage.db for recording: {}", e);
            }
        }
    });
}
