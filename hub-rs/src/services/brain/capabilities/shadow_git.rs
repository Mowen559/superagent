use std::path::Path;
use std::process::Command;

/// Create a lightweight snapshot of a file using git (if the directory is a git repo).
pub fn snapshot_file(filepath: &str) {
    let path = Path::new(filepath);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    // Quick check if git exists
    if Command::new("git")
        .arg("status")
        .current_dir(&cwd)
        .output()
        .is_err()
    {
        return;
    }

    // Add and commit specifically the file to a shadow stash/branch or just normal commit?
    // A better approach for an agent is to `git diff` and store the patch, or simply use `git add` + `git commit -m "[ShadowGit] Snapshot before AI edit"`.
    // However, we don`t want to mess up the user`s actual git history if they are working!
    // Instead, we just copy the file to a hidden directory: `.shadow_git/`
    let shadow_dir = cwd.join(".shadow_git");
    if !shadow_dir.exists() {
        let _ = std::fs::create_dir_all(&shadow_dir);
    }

    if let Some(filename) = path.file_name() {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let backup_name = format!("{}_{}", timestamp, filename.to_string_lossy());
        let backup_path = shadow_dir.join(backup_name);

        let _ = std::fs::copy(path, backup_path);
    }
}
