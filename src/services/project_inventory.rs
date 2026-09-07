use ignore::WalkBuilder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_REPORTED_PATHS: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct InventoryLimits {
    pub max_files: usize,
    pub max_fingerprint_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInventory {
    pub source_fingerprint: String,
    pub fingerprint_algorithm: &'static str,
    pub fingerprint_bytes: u64,
    pub files_scanned: usize,
    pub total_bytes: u64,
    pub latest_modified_ms: Option<u128>,
    pub truncated: bool,
    pub traversal_errors: usize,
    pub languages: BTreeMap<String, usize>,
    pub top_level_modules: BTreeMap<String, usize>,
    pub build_systems: Vec<String>,
    pub manifests: Vec<String>,
    pub entry_points: Vec<String>,
    pub test_files: Vec<String>,
    pub documentation: Vec<String>,
    pub ci_files: Vec<String>,
}

#[derive(Debug)]
struct FileFact {
    absolute: PathBuf,
    relative: String,
    size: u64,
    modified_ms: Option<u128>,
}

pub fn inspect_project(root: &Path, limits: InventoryLimits) -> Result<ProjectInventory, String> {
    if !root.is_dir() {
        return Err(format!(
            "Project path is not a directory: {}",
            root.display()
        ));
    }

    let mut facts = Vec::new();
    let mut traversal_errors = 0usize;
    let mut hit_file_limit = false;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .ignore(true)
        .sort_by_file_path(|left, right| left.cmp(right))
        .filter_entry(|entry| !is_generated_directory(entry.path()));

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                traversal_errors += 1;
                continue;
            }
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if facts.len() >= limits.max_files {
            hit_file_limit = true;
            break;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                traversal_errors += 1;
                continue;
            }
        };
        let relative = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis());
        facts.push(FileFact {
            absolute: entry.path().to_path_buf(),
            relative,
            size: metadata.len(),
            modified_ms,
        });
    }

    facts.sort_by(|left, right| left.relative.cmp(&right.relative));
    let mut hasher = Sha256::new();
    let mut fingerprint_bytes = 0u64;
    let mut fingerprint_truncated = false;
    let mut total_bytes = 0u64;
    let mut latest_modified_ms: Option<u128> = None;
    let mut languages = BTreeMap::new();
    let mut top_level_modules = BTreeMap::new();
    let mut build_systems = BTreeSet::new();
    let mut manifests = Vec::new();
    let mut entry_points = Vec::new();
    let mut test_files = Vec::new();
    let mut documentation = Vec::new();
    let mut ci_files = Vec::new();

    for fact in &facts {
        total_bytes = total_bytes.saturating_add(fact.size);
        latest_modified_ms = match (latest_modified_ms, fact.modified_ms) {
            (Some(current), Some(candidate)) => Some(current.max(candidate)),
            (None, value) => value,
            (value, None) => value,
        };
        hasher.update(fact.relative.as_bytes());
        hasher.update([0]);
        hasher.update(fact.size.to_le_bytes());
        hasher.update(fact.modified_ms.unwrap_or_default().to_le_bytes());

        if fingerprint_bytes < limits.max_fingerprint_bytes {
            match File::open(&fact.absolute) {
                Ok(mut file) => {
                    let mut buffer = [0u8; 32 * 1024];
                    loop {
                        let allowed = limits
                            .max_fingerprint_bytes
                            .saturating_sub(fingerprint_bytes)
                            .min(buffer.len() as u64)
                            as usize;
                        if allowed == 0 {
                            fingerprint_truncated = true;
                            break;
                        }
                        match file.read(&mut buffer[..allowed]) {
                            Ok(0) => break,
                            Ok(read) => {
                                hasher.update(&buffer[..read]);
                                fingerprint_bytes = fingerprint_bytes.saturating_add(read as u64);
                            }
                            Err(_) => {
                                traversal_errors += 1;
                                break;
                            }
                        }
                    }
                    if fingerprint_bytes >= limits.max_fingerprint_bytes && fact.size > 0 {
                        fingerprint_truncated = true;
                    }
                }
                Err(_) => traversal_errors += 1,
            }
        } else {
            fingerprint_truncated = true;
        }

        let lower = fact.relative.to_ascii_lowercase();
        if let Some(extension) = Path::new(&lower)
            .extension()
            .and_then(|value| value.to_str())
        {
            *languages
                .entry(language_for_extension(extension).to_string())
                .or_insert(0) += 1;
        }
        let top = fact.relative.split('/').next().unwrap_or("root");
        let top = if fact.relative.contains('/') {
            top
        } else {
            "root"
        };
        *top_level_modules.entry(top.to_string()).or_insert(0) += 1;

        let file_name = Path::new(&lower)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if let Some(build_system) = build_system_for_manifest(file_name) {
            build_systems.insert(build_system.to_string());
            push_bounded(&mut manifests, &fact.relative);
        }
        if is_entry_point(&lower, file_name) {
            push_bounded(&mut entry_points, &fact.relative);
        }
        if is_test_file(&lower, file_name) {
            push_bounded(&mut test_files, &fact.relative);
        }
        if is_documentation(file_name, &lower) {
            push_bounded(&mut documentation, &fact.relative);
        }
        if is_ci_file(&lower) {
            push_bounded(&mut ci_files, &fact.relative);
        }
    }

    let source_fingerprint = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(ProjectInventory {
        source_fingerprint,
        fingerprint_algorithm: "sha256-path-metadata-content-bounded-v1",
        fingerprint_bytes,
        files_scanned: facts.len(),
        total_bytes,
        latest_modified_ms,
        truncated: hit_file_limit || fingerprint_truncated,
        traversal_errors,
        languages,
        top_level_modules,
        build_systems: build_systems.into_iter().collect(),
        manifests,
        entry_points,
        test_files,
        documentation,
        ci_files,
    })
}

fn push_bounded(values: &mut Vec<String>, value: &str) {
    if values.len() < MAX_REPORTED_PATHS {
        values.push(value.to_string());
    }
}

fn is_generated_directory(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(
            ".git"
                | ".next"
                | ".deepcloud"
                | ".shadow-git"
                | ".supermemory"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | "vendor"
                | "coverage"
        )
    )
}

fn build_system_for_manifest(file_name: &str) -> Option<&'static str> {
    match file_name {
        "cargo.toml" => Some("Cargo"),
        "package.json" => Some("Node.js"),
        "pnpm-lock.yaml" => Some("pnpm"),
        "yarn.lock" => Some("Yarn"),
        "pom.xml" => Some("Maven"),
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => {
            Some("Gradle")
        }
        "go.mod" => Some("Go Modules"),
        "pyproject.toml" | "requirements.txt" | "setup.py" => Some("Python"),
        "composer.json" => Some("Composer"),
        "gemfile" => Some("Bundler"),
        "cmakelists.txt" => Some("CMake"),
        "makefile" => Some("Make"),
        name if name.ends_with(".sln") || name.ends_with(".csproj") => Some(".NET"),
        _ => None,
    }
}

fn is_entry_point(relative: &str, file_name: &str) -> bool {
    matches!(
        file_name,
        "main.rs"
            | "lib.rs"
            | "main.go"
            | "main.py"
            | "app.py"
            | "manage.py"
            | "index.ts"
            | "index.tsx"
            | "index.js"
            | "index.jsx"
            | "main.ts"
            | "main.tsx"
            | "main.js"
            | "main.jsx"
    ) || relative.ends_with("application.java")
        || relative.ends_with("application.kt")
        || relative.ends_with("/pages/index.tsx")
        || relative.ends_with("/pages/index.jsx")
        || relative.ends_with("/app/page.tsx")
        || relative.ends_with("/app/page.jsx")
}

fn is_test_file(relative: &str, file_name: &str) -> bool {
    relative.contains("/tests/")
        || relative.contains("/test/")
        || relative.contains("/__tests__/")
        || file_name.starts_with("test_")
        || file_name.ends_with("_test.rs")
        || file_name.ends_with("_test.go")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
}

fn is_documentation(file_name: &str, relative: &str) -> bool {
    matches!(
        Path::new(file_name)
            .extension()
            .and_then(|value| value.to_str()),
        Some("md" | "mdx" | "rst" | "adoc")
    ) || relative.ends_with("readme")
        || relative.ends_with("license")
}

fn is_ci_file(relative: &str) -> bool {
    relative.starts_with(".github/workflows/")
        || relative.starts_with(".gitlab-ci")
        || relative == "jenkinsfile"
        || relative.starts_with(".circleci/")
        || relative.starts_with("azure-pipelines")
}

fn language_for_extension(extension: &str) -> &'static str {
    match extension {
        "rs" => "Rust",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "go" => "Go",
        "py" => "Python",
        "ts" | "mts" | "cts" => "TypeScript",
        "tsx" => "TSX",
        "js" | "mjs" | "cjs" => "JavaScript",
        "jsx" => "JSX",
        "vue" => "Vue",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" => "C++",
        "cs" => "C#",
        "php" => "PHP",
        "rb" => "Ruby",
        "swift" => "Swift",
        "sql" => "SQL",
        "md" | "mdx" | "rst" | "adoc" => "Documentation",
        "json" | "yaml" | "yml" | "toml" | "xml" => "Configuration",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn inventory_finds_project_operational_surfaces() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("src")).unwrap();
        fs::create_dir_all(directory.path().join("tests")).unwrap();
        fs::create_dir_all(directory.path().join("docs")).unwrap();
        fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname='demo'",
        )
        .unwrap();
        fs::write(directory.path().join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(
            directory.path().join("tests/app_test.rs"),
            "#[test] fn works() {}",
        )
        .unwrap();
        fs::write(directory.path().join("docs/README.md"), "# Demo").unwrap();

        let result = inspect_project(
            directory.path(),
            InventoryLimits {
                max_files: 100,
                max_fingerprint_bytes: 1024 * 1024,
            },
        )
        .unwrap();

        assert_eq!(result.files_scanned, 4);
        assert!(result.build_systems.contains(&"Cargo".to_string()));
        assert!(result.entry_points.contains(&"src/main.rs".to_string()));
        assert!(result.test_files.contains(&"tests/app_test.rs".to_string()));
        assert!(result.documentation.contains(&"docs/README.md".to_string()));
        assert_eq!(result.source_fingerprint.len(), 64);
        assert!(!result.truncated);
    }

    #[test]
    fn fingerprint_changes_when_source_changes() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("main.py");
        fs::write(&source, "print('one')").unwrap();
        let limits = InventoryLimits {
            max_files: 100,
            max_fingerprint_bytes: 1024 * 1024,
        };
        let first = inspect_project(directory.path(), limits).unwrap();
        fs::write(&source, "print('two')").unwrap();
        let second = inspect_project(directory.path(), limits).unwrap();
        assert_ne!(first.source_fingerprint, second.source_fingerprint);
    }
}
