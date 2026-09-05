use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use super::types::VisualSettings;

pub fn read(path: &Path) -> VisualSettings {
    let Ok(content) = std::fs::read_to_string(path) else {
        return VisualSettings::default();
    };
    serde_json::from_str(content.trim_start_matches('\u{feff}')).unwrap_or_else(|error| {
        tracing::warn!("failed to read visual settings {}: {error}", path.display());
        VisualSettings::default()
    })
}

pub fn write_atomic(path: &Path, settings: &VisualSettings) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "visual settings path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("failed to serialize visual settings: {error}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("failed to open {}: {error}", temporary.display()))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("failed to persist {}: {error}", temporary.display()))?;
    if !path.exists() {
        return std::fs::rename(&temporary, path)
            .map_err(|error| format!("failed to commit {}: {error}", path.display()));
    }

    let backup = path.with_extension("json.bak");
    if backup.exists() {
        std::fs::remove_file(&backup)
            .map_err(|error| format!("failed to rotate {}: {error}", backup.display()))?;
    }
    std::fs::rename(path, &backup)
        .map_err(|error| format!("failed to stage {}: {error}", path.display()))?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::rename(&backup, path);
            Err(format!("failed to commit {}: {error}", path.display()))
        }
    }
}
