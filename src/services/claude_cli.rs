//! Local Claude Code CLI integration.
//!
//! Runs `claude -p` headless with the persisted settings file
//! (`settings/claude.json`, see `config::claude_settings`) so the local proxy
//! env, model, and MCP tool permissions apply to every invocation. Prompts run
//! as background tasks (TaskManager) because agentic runs can take minutes;
//! output is streamed into the task log.

use crate::config::claude_settings;
use crate::models::task::TaskStatus;
use crate::domains::project_management::task_manager::TaskManager;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Hard cap for a single headless run.
const RUN_TIMEOUT_SECS: u64 = 20 * 60;

/// Wire rtk's PreToolUse hook into hub's claude.json and make the rtk binary
/// resolvable from the child's PATH, so the launched Claude CLI auto-routes its
/// Bash tool output through rtk (token compression). No-op when `rtk_bin` is
/// absent or the `rtkHook` toggle is off. Returns whether the hook is active.
fn apply_rtk_hook(cmd: &mut Command, rtk_bin: Option<&Path>) -> bool {
    let Some(rtk_bin) = rtk_bin else {
        return false;
    };
    let active = claude_settings::ensure_rtk_hook(rtk_bin, None);
    if active {
        if let Some(dir) = rtk_bin.parent() {
            let prev = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![dir.to_path_buf()];
            paths.extend(std::env::split_paths(&prev));
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
    }
    active
}

/// Build a command that resolves the CLI through the shell, since npm installs
/// `claude` as a `.cmd`/`.ps1` shim on Windows that CreateProcess cannot spawn
/// directly.
fn claude_command() -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg("claude");
        cmd
    } else {
        Command::new("claude")
    }
}

/// Probe the local CLI: returns the version string when installed.
pub async fn detect_version() -> Option<String> {
    let mut cmd = claude_command();
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let output = tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output())
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

/// Run a headless `claude -p` prompt and return its final text answer
/// synchronously (awaited). Unlike `run_prompt` this does not create a task;
/// it is used by request handlers (e.g. project understanding) that need the
/// answer inline. The prompt is piped through stdin to avoid shell quoting.
pub async fn run_and_capture(
    project: Option<&str>,
    prompt: &str,
    timeout_secs: u64,
    rtk_bin: Option<&Path>,
) -> Result<String, String> {
    let settings_path = claude_settings::ensure_persisted();
    let mut cmd = claude_command();
    apply_rtk_hook(&mut cmd, rtk_bin);
    cmd.arg("-p")
        .arg("--settings")
        .arg(settings_path.to_string_lossy().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    if let Some(dir) = project.filter(|p| std::path::Path::new(p).is_dir()) {
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch claude CLI: {}", e))?;

    // Feed the prompt via stdin, then close it so the CLI starts working.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("Failed to run claude CLI: {}", e)),
        Err(_) => return Err(format!("claude run timed out after {}s", timeout_secs)),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "claude exited with {}: {}",
            output.status,
            tail.trim()
        ));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Err("claude returned empty output".to_string());
    }
    Ok(text)
}

/// Launch a headless `claude -p` run as a background task and return the task
/// id. The prompt is piped through stdin to avoid shell quoting issues.
pub fn run_prompt(
    task_manager: Arc<TaskManager>,
    project: Option<String>,
    prompt: String,
    rtk_bin: Option<PathBuf>,
) -> String {
    let cwd = project.clone().unwrap_or_default();
    let (task, _) = task_manager.create_task(
        "claude-cli",
        &cwd,
        Some(serde_json::json!({ "prompt": prompt })),
    );
    let task_id = task.id.clone();

    let tm = task_manager;
    let tid = task_id.clone();
    tokio::spawn(async move {
        tm.update_status(&tid, TaskStatus::Running, None);

        let settings_path = claude_settings::ensure_persisted();
        let mut cmd = claude_command();
        apply_rtk_hook(&mut cmd, rtk_bin.as_deref());
        cmd.arg("-p")
            .arg("--settings")
            .arg(settings_path.to_string_lossy().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

        if let Some(dir) = project
            .as_deref()
            .filter(|p| std::path::Path::new(p).is_dir())
        {
            cmd.current_dir(dir);
        }

        tm.append_log(
            &tid,
            "[Claude] Starting headless run (claude -p --settings claude.json)",
        );

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                tm.append_log(&tid, &format!("[Claude] Failed to launch CLI: {}", e));
                tm.update_status(
                    &tid,
                    TaskStatus::Failed,
                    Some(format!("Failed to launch claude CLI: {}", e)),
                );
                return;
            }
        };

        // Feed the prompt via stdin, then close it so the CLI starts working.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }

        if let Some(stdout) = child.stdout.take() {
            let tm2 = tm.clone();
            let tid2 = tid.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tm2.append_log(&tid2, &format!("[Claude] {}", line));
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let tm2 = tm.clone();
            let tid2 = tid.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tm2.append_log(&tid2, &format!("[Claude:err] {}", line));
                }
            });
        }

        match tokio::time::timeout(
            std::time::Duration::from_secs(RUN_TIMEOUT_SECS),
            child.wait(),
        )
        .await
        {
            Ok(Ok(status)) if status.success() => {
                tm.append_log(&tid, "[Claude] Run completed.");
                tm.update_status(&tid, TaskStatus::Completed, None);
            }
            Ok(Ok(status)) => {
                let msg = format!("claude exited with status {}", status);
                tm.append_log(&tid, &format!("[Claude] {}", msg));
                tm.update_status(&tid, TaskStatus::Failed, Some(msg));
            }
            Ok(Err(e)) => {
                let msg = format!("Failed to wait for claude: {}", e);
                tm.append_log(&tid, &format!("[Claude] {}", msg));
                tm.update_status(&tid, TaskStatus::Failed, Some(msg));
            }
            Err(_) => {
                let _ = child.kill().await;
                let msg = format!("claude run timed out after {}s", RUN_TIMEOUT_SECS);
                tm.append_log(&tid, &format!("[Claude] {}", msg));
                tm.update_status(&tid, TaskStatus::Failed, Some(msg));
            }
        }
    });

    task_id
}
