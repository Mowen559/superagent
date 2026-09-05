//! Cross-platform agent subprocess helpers, distilled from `claude_cli.rs`.
//!
//! npm/npx install most CLIs as a `.cmd`/`.ps1` shim on Windows that
//! `CreateProcess` cannot spawn directly, so on Windows we route through
//! `cmd /C <name>` and set `CREATE_NO_WINDOW` to avoid a console-window storm.
//! On Unix we spawn the binary directly. These helpers are shared by every
//! backend and by the registry's `--version` probe.

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Build a [`Command`] that resolves `name` through the shell on Windows (to
/// handle `.cmd`/`.ps1` shims) and directly on Unix. On Windows the console
/// window is suppressed.
pub fn agent_command(default_name: &str) -> Command {
    let resolved_name = resolve_agent_path(default_name);

    #[allow(unused_mut)]
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(resolved_name);
        c
    } else {
        Command::new(resolved_name)
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Probe a CLI: run `<name> --version` and return the trimmed first useful
/// line when it exits 0, else `None`. Bounded to 20s so a hung shim cannot
/// stall detection.
pub async fn detect_version(name: &str) -> Option<String> {
    let mut cmd = agent_command(name);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let output = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

pub fn resolve_agent_path(name: &str) -> String {
    let install_dir = super::installer::get_agents_install_dir();

    let p_ext = install_dir
        .join("node_modules")
        .join(".bin")
        .join(format!("{name}.cmd"));
    if p_ext.exists() {
        return p_ext.to_string_lossy().into_owned();
    }
    let p = install_dir.join("node_modules").join(".bin").join(name);
    if p.exists() {
        return p.to_string_lossy().into_owned();
    }

    for info in super::registry::KINDS {
        if info.command == name {
            let p = if cfg!(windows) {
                install_dir
                    .join("venvs")
                    .join(info.slug)
                    .join("Scripts")
                    .join(format!("{name}.exe"))
            } else {
                install_dir
                    .join("venvs")
                    .join(info.slug)
                    .join("bin")
                    .join(name)
            };
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }

    name.to_string()
}
