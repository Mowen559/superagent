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

/// Owns the worker and every descendant on Windows. Handles are not thread-affine.
pub struct RunProcessTree {
    #[cfg(windows)]
    handle: usize,
}

impl RunProcessTree {
    pub fn attach(child: &tokio::process::Child) -> Result<Self, String> {
        #[cfg(windows)] unsafe {
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let raw = child.raw_handle().ok_or("worker has no process handle")?;
            if SetInformationJobObject(job, JobObjectExtendedLimitInformation,
                &info as *const _ as *const _, std::mem::size_of_val(&info) as u32) == 0
                || AssignProcessToJobObject(job, raw as _) == 0 {
                let error = std::io::Error::last_os_error().to_string(); CloseHandle(job); return Err(error);
            }
            Ok(Self { handle: job as usize })
        }
        #[cfg(not(windows))] { let _ = child; Ok(Self {}) }
    }

    pub fn terminate(&self) {
        #[cfg(windows)] unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle as _, 1);
        }
    }
}

impl Drop for RunProcessTree {
    fn drop(&mut self) {
        #[cfg(windows)] unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle as _); }
    }
}

#[cfg(all(test, windows))]
mod process_tree_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn cancellation_reaps_worker_and_real_grandchild_within_contract() {
        let script = "import subprocess,sys,time\ntime.sleep(0.3)\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'])\nprint(p.pid,flush=True)\ntime.sleep(30)";
        let interpreter = std::process::Command::new("python").args(["-X", "utf8", "-c", "import sys;print(sys.executable)"]).output().unwrap();
        let interpreter = String::from_utf8(interpreter.stdout).unwrap();
        let mut command = agent_command(interpreter.trim());
        command.args(["-u", "-c", script]).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        let tree = RunProcessTree::attach(&child).unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let pid: u32 = tokio::time::timeout(Duration::from_secs(5), output.next_line()).await.unwrap().unwrap().unwrap().parse().unwrap();
        use windows_sys::Win32::{Foundation::{CloseHandle, WAIT_OBJECT_0}, System::Threading::{OpenProcess, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject}};
        let descendant = unsafe { OpenProcess(SYNCHRONIZATION_SYNCHRONIZE, 0, pid) };
        assert!(!descendant.is_null(), "fixture grandchild must exist before cancellation");
        let started = std::time::Instant::now();
        tree.terminate();
        let exited = unsafe { WaitForSingleObject(descendant, 200) };
        unsafe { CloseHandle(descendant); }
        assert_eq!(exited, WAIT_OBJECT_0, "grandchild survived cancellation");
        assert!(started.elapsed() < Duration::from_millis(200));
        child.wait().await.unwrap();
    }
}

/// Build a [`Command`] that resolves `name` through the shell on Windows (to
/// handle `.cmd`/`.ps1` shims) and directly on Unix. On Windows the console
/// window is suppressed.
pub fn agent_command(default_name: &str) -> Command {
    let resolved_name = resolve_agent_path(default_name);

    #[allow(unused_mut)]
    let mut cmd = if cfg!(windows) {
        let is_exe = resolved_name.ends_with(".exe")
            || std::path::Path::new(&resolved_name)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
        if is_exe {
            Command::new(resolved_name)
        } else {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(resolved_name);
            c
        }
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
