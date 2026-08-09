use std::path::PathBuf;
use std::process::Command;
use thiserror::Error;

use crate::services::agent::AgentKind;
use super::registry::InstallStrategy;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("Installation is unsupported for this agent")]
    Unsupported,
    #[error("Failed to execute install command: {0}")]
    CommandFailed(String),
    #[error("Installation failed with exit code: {0}")]
    ProcessFailed(String),
}

pub fn get_agents_install_dir() -> PathBuf {
    let app_paths = crate::config::app_paths::AppPaths::new();
    app_paths.app_data_root.join("agents")
}

pub fn install_agent(kind: AgentKind) -> Result<(), InstallError> {
    let info = kind.info();
    match info.install_strategy {
        InstallStrategy::Npm(package_name) => install_npm(package_name),
        InstallStrategy::Pip(package_name) => install_pip(kind, package_name),
        InstallStrategy::BinaryDownload(url) => install_binary(kind, url),
        InstallStrategy::Unsupported => Err(InstallError::Unsupported),
    }
}

fn install_npm(package_name: &str) -> Result<(), InstallError> {
    let install_dir = get_agents_install_dir();
    if !install_dir.exists() {
        std::fs::create_dir_all(&install_dir)
            .map_err(|e| InstallError::CommandFailed(format!("Failed to create install dir: {}", e)))?;
    }

    let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
    
    let mut cmd = Command::new(npm_cmd);
    cmd.arg("install")
        .arg("--prefix")
        .arg(&install_dir)
        .arg(package_name)
        .arg("--no-save");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    let status = cmd.status()
        .map_err(|e| InstallError::CommandFailed(e.to_string()))?;

    if !status.success() {
        return Err(InstallError::ProcessFailed(status.to_string()));
    }

    Ok(())
}

fn install_pip(kind: AgentKind, package_name: &str) -> Result<(), InstallError> {
    let install_dir = get_agents_install_dir().join("venvs").join(kind.info().slug);
    if !install_dir.exists() {
        std::fs::create_dir_all(&install_dir)
            .map_err(|e| InstallError::CommandFailed(format!("Failed to create venv dir: {}", e)))?;
    }

    // Create venv
    let python_cmd = if cfg!(windows) { "python" } else { "python3" };
    let mut venv_cmd = Command::new(python_cmd);
    venv_cmd.arg("-m").arg("venv").arg(&install_dir);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        venv_cmd.creation_flags(0x0800_0000);
    }

    let venv_status = venv_cmd.status()
        .map_err(|e| InstallError::CommandFailed(format!("Failed to run venv: {}", e)))?;

    if !venv_status.success() {
        return Err(InstallError::ProcessFailed(format!("venv creation failed: {}", venv_status)));
    }

    // Run pip install
    let pip_cmd = if cfg!(windows) {
        install_dir.join("Scripts").join("pip")
    } else {
        install_dir.join("bin").join("pip")
    };

    let mut cmd = Command::new(pip_cmd);
    cmd.arg("install").arg(package_name);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    let status = cmd.status()
        .map_err(|e| InstallError::CommandFailed(format!("Failed to run pip: {}", e)))?;

    if !status.success() {
        return Err(InstallError::ProcessFailed(format!("pip install failed: {}", status)));
    }

    Ok(())
}

fn install_binary(_kind: AgentKind, _url: &str) -> Result<(), InstallError> {
    // Stub for now. We don't have binary downloads defined yet.
    Err(InstallError::Unsupported)
}

pub fn update_agent(kind: AgentKind) -> Result<(), InstallError> {
    let info = kind.info();
    match info.install_strategy {
        InstallStrategy::Npm(package_name) => update_npm(package_name),
        InstallStrategy::Pip(package_name) => update_pip(kind, package_name),
        InstallStrategy::BinaryDownload(_url) => Err(InstallError::Unsupported),
        InstallStrategy::Unsupported => Err(InstallError::Unsupported),
    }
}

fn update_npm(package_name: &str) -> Result<(), InstallError> {
    let install_dir = get_agents_install_dir();
    if !install_dir.exists() {
        return Err(InstallError::CommandFailed("Agent is not installed yet.".into()));
    }

    let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let mut cmd = Command::new(npm_cmd);
    cmd.arg("install")
        .arg("--prefix")
        .arg(&install_dir)
        .arg(format!("{}@latest", package_name))
        .arg("--no-save");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    let status = cmd.status()
        .map_err(|e| InstallError::CommandFailed(e.to_string()))?;

    if !status.success() {
        return Err(InstallError::ProcessFailed(status.to_string()));
    }
    Ok(())
}

fn update_pip(kind: AgentKind, package_name: &str) -> Result<(), InstallError> {
    let install_dir = get_agents_install_dir().join("venvs").join(kind.info().slug);
    if !install_dir.exists() {
        return Err(InstallError::CommandFailed("Agent is not installed yet.".into()));
    }

    let pip_cmd = if cfg!(windows) {
        install_dir.join("Scripts").join("pip")
    } else {
        install_dir.join("bin").join("pip")
    };

    let mut cmd = Command::new(pip_cmd);
    cmd.arg("install").arg("--upgrade").arg(package_name);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    let status = cmd.status()
        .map_err(|e| InstallError::CommandFailed(format!("Failed to run pip: {}", e)))?;

    if !status.success() {
        return Err(InstallError::ProcessFailed(format!("pip install failed: {}", status)));
    }
    Ok(())
}
