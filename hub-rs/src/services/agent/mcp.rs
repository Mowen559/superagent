//! MCP Config Temp file generation and lifecycle management.
//!
//! Mirrors multica's `writeMcpConfigToTemp` and `hasManagedMcpConfig`.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// An RAII guard for an MCP config file written to a temporary directory.
/// The file is removed when this guard is dropped.
pub struct McpConfigTempFile {
    path: PathBuf,
}

impl McpConfigTempFile {
    /// Writes the given MCP config JSON to a temporary file. Returns the guard.
    pub fn new(config: &Value) -> Result<Self, std::io::Error> {
        // Use tempfile crate or just a random file in std::env::temp_dir()
        use std::io::Write;

        let mut temp_file = tempfile::NamedTempFile::new()?;
        let json_bytes = serde_json::to_vec(config)?;
        temp_file.write_all(&json_bytes)?;
        temp_file.flush()?;

        let (_, path) = temp_file.keep()?;
        Ok(Self { path })
    }

    /// Returns the absolute path to the generated config file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for McpConfigTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Helper to determine if an MCP config is "managed" (not just null).
pub fn has_managed_mcp_config(config: Option<&Value>) -> bool {
    match config {
        Some(Value::Null) | None => false,
        _ => true,
    }
}
