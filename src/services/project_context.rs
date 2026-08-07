use std::path::Path;

/// Loads project-specific instructions from a `.deepcloud.md` file in the given project directory.
/// If no file is found, it traverses upwards to find one in parent directories, stopping at the file system root.
pub fn load_project_instructions(project_path: Option<&Path>) -> Option<String> {
    let mut current_dir = match project_path {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().unwrap_or_default(),
    };

    loop {
        let candidate = current_dir.join(".deepcloud.md");
        if candidate.exists() {
            if let Ok(content) = std::fs::read_to_string(&candidate) {
                return Some(content);
            }
        }
        if !current_dir.pop() {
            break;
        }
    }

    None
}
