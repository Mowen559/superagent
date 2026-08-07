//! Self-hosted `rtk` binary shim.
//!
//! The `hub` executable doubles as the rtk hook (see [`crate::cli`]), so there
//! is no separate `rtk` binary to ship. Two things still expect a program
//! literally named `rtk` on `PATH`:
//!
//! * rtk's command rewriter emits the hard-coded prefix `rtk` (e.g. it turns
//!   `cat foo` into `rtk read foo`), which Claude then spawns as a fresh
//!   process.
//!
//! To satisfy that without a second build artifact, we copy the running `hub`
//! executable to `{app_data}/bin/rtk{.exe}` once per version and prepend that
//! directory to the Claude child's `PATH`. Because the copy is the same binary,
//! it dispatches through [`crate::cli`] and — invoked as `rtk read ...` — takes
//! the hook branch just like `hub read ...` would.

use std::path::PathBuf;

use super::app_paths::AppPaths;

/// File name of the shim (`rtk.exe` on Windows, `rtk` elsewhere).
fn shim_file_name() -> String {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    format!("rtk{ext}")
}

/// Absolute path where the shim lives: `{app_data}/bin/rtk{.exe}`.
pub fn shim_path(app_paths: &AppPaths) -> PathBuf {
    app_paths.data_path(&["bin", &shim_file_name()])
}

/// Ensure `{app_data}/bin/rtk{.exe}` exists and matches the current `hub`
/// executable, returning its path. Falls back to the current executable path
/// itself when the copy cannot be produced (e.g. `current_exe` unavailable or
/// the destination is locked), so callers always get a usable rtk-role binary.
pub fn ensure_shim(app_paths: &AppPaths) -> PathBuf {
    let Ok(current) = std::env::current_exe() else {
        return PathBuf::from(shim_file_name());
    };

    let dst = shim_path(app_paths);

    // Skip the copy when the shim is already up to date (same size). This is a
    // cheap heuristic that avoids clobbering a running copy on every launch.
    if is_up_to_date(&current, &dst) {
        return dst;
    }

    app_paths.ensure_data_dir(&["bin"]);
    match std::fs::copy(&current, &dst) {
        Ok(_) => {
            tracing::info!("rtk shim refreshed at {}", dst.display());
            dst
        }
        Err(e) => {
            // A locked destination usually means a previous copy is fine to use.
            if dst.exists() {
                tracing::warn!("rtk shim copy failed ({e}); reusing existing {}", dst.display());
                dst
            } else {
                tracing::warn!("rtk shim copy failed ({e}); falling back to current exe");
                current
            }
        }
    }
}

/// True when `dst` exists and has the same byte length as `src`.
fn is_up_to_date(src: &std::path::Path, dst: &std::path::Path) -> bool {
    let (Ok(sm), Ok(dm)) = (std::fs::metadata(src), std::fs::metadata(dst)) else {
        return false;
    };
    sm.len() == dm.len()
}
