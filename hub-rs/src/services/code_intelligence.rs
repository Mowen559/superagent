use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// Provides unified access to the Code Intelligence graph database
pub struct CodeIntelligenceRuntime {
    db_path: PathBuf,
}

impl CodeIntelligenceRuntime {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
        }
    }

    pub fn is_indexed(&self) -> bool {
        self.db_path.exists()
    }

    pub fn db_path_str(&self) -> String {
        self.db_path.to_string_lossy().to_string()
    }

    pub fn open_ro(&self) -> Result<Connection, String> {
        if !self.is_indexed() {
            return Err("Graph database not found (not indexed)".to_string());
        }
        let conn = Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Failed to open graph DB: {}", e))?;

        // Basic schema validation to ensure the DB is a valid codebase-memory graph
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('nodes', 'edges')",
            )
            .map_err(|e| format!("Schema validation failed: {}", e))?;

        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Schema validation failed: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        if tables.len() != 2 {
            return Err("Graph database schema is invalid or missing required tables".to_string());
        }

        drop(stmt);
        Ok(conn)
    }
}
