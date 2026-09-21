use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpdateAction {
    Skip,
    PartialUpdate,
    ArchitectureUpdate,
    FullUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDecision {
    pub action: UpdateAction,
    pub files_to_reanalyze: Vec<String>,
    pub rerun_architecture: bool,
    pub rerun_tour: bool,
    pub reason: String,
    pub total_structural_changes: usize,
    pub cosmetic_only_count: usize,
}

pub struct ChangeClassifier;

impl ChangeClassifier {
    /// 判断是否属于代码文件（有代码结构），而非纯文档、配置文件或静态资源
    pub fn is_structural_code_file(path: &str) -> bool {
        let p = path.to_lowercase();
        // 排除常见非代码/静态资源/配置文件
        if p.ends_with(".md")
            || p.ends_with(".txt")
            || p.ends_with(".png")
            || p.ends_with(".jpg")
            || p.ends_with(".jpeg")
            || p.ends_with(".gif")
            || p.ends_with(".svg")
            || p.ends_with(".ico")
            || p.ends_with(".lock")
            || p.ends_with(".json")
            || p.ends_with(".toml")
            || p.ends_with(".yaml")
            || p.ends_with(".yml")
            || p.ends_with(".xml")
            || p.ends_with(".gitignore")
            || p.ends_with(".gitattributes")
            || p.contains("/docs/")
            || p.contains("/.github/")
            || p.contains("/target/")
            || p.contains("/node_modules/")
        {
            return false;
        }

        // 识别常见程序语言扩展名
        let extensions = [
            ".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".py", ".go", ".java", ".c",
            ".cpp", ".cc", ".h", ".hpp", ".cs", ".php", ".rb", ".kt", ".swift", ".scala",
            ".dart", ".vue", ".svelte",
        ];
        extensions.iter().any(|ext| p.ends_with(ext))
    }

    /// 提取文件的顶级源代码目录（如 "src", "packages", "crates", "components"）
    pub fn top_level_dir(file_path: &str) -> Option<String> {
        let normalized = file_path.replace('\\', "/");
        let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() > 1 {
            Some(parts[0].to_string())
        } else {
            None
        }
    }

    /// 纯逻辑决策分类器：基于变动文件列表与已知索引文件列表推导更新动作
    pub fn classify_from_file_lists(
        changed_files: &[String],
        all_indexed_files: &[String],
    ) -> UpdateDecision {
        if changed_files.is_empty() {
            return UpdateDecision {
                action: UpdateAction::Skip,
                files_to_reanalyze: Vec::new(),
                rerun_architecture: false,
                rerun_tour: false,
                reason: "No changes detected in workspace".to_string(),
                total_structural_changes: 0,
                cosmetic_only_count: 0,
            };
        }

        let mut structural_files = Vec::new();
        let mut cosmetic_files = Vec::new();

        for file in changed_files {
            if Self::is_structural_code_file(file) {
                structural_files.push(file.clone());
            } else {
                cosmetic_files.push(file.clone());
            }
        }

        let structural_count = structural_files.len();
        let cosmetic_count = cosmetic_files.len();

        // 1. 无任何结构化代码修改 -> SKIP
        if structural_count == 0 {
            return UpdateDecision {
                action: UpdateAction::Skip,
                files_to_reanalyze: Vec::new(),
                rerun_architecture: false,
                rerun_tour: false,
                reason: format!(
                    "{} changed file(s) are documentation/cosmetic with zero structural impact",
                    cosmetic_count
                ),
                total_structural_changes: 0,
                cosmetic_only_count: cosmetic_count,
            };
        }

        let total_known = all_indexed_files.len();
        let triggered_by_count = structural_count > 30;
        let triggered_by_ratio =
            total_known > 0 && (structural_count as f64 / total_known as f64) > 0.5;

        // 2. 变更文件过多（>30 或 >50%）-> FULL_UPDATE
        if triggered_by_count || triggered_by_ratio {
            let reason = if triggered_by_count && triggered_by_ratio {
                format!(
                    "{} structural files changed (>30 files and >50% of project) - full rebuild required",
                    structural_count
                )
            } else if triggered_by_count {
                format!(
                    "{} structural files changed (>30 files threshold) - full rebuild recommended",
                    structural_count
                )
            } else {
                format!(
                    "{} structural files changed (>50% of project) - full rebuild recommended",
                    structural_count
                )
            };

            return UpdateDecision {
                action: UpdateAction::FullUpdate,
                files_to_reanalyze: structural_files,
                rerun_architecture: true,
                rerun_tour: true,
                reason,
                total_structural_changes: structural_count,
                cosmetic_only_count: cosmetic_count,
            };
        }

        // 3. 检查是否有新增或删除的顶级目录
        let existing_dirs: HashSet<String> = all_indexed_files
            .iter()
            .filter_map(|f| Self::top_level_dir(f))
            .collect();

        let changed_dirs: HashSet<String> = structural_files
            .iter()
            .filter_map(|f| Self::top_level_dir(f))
            .collect();

        let has_new_top_dir = changed_dirs.iter().any(|d| !existing_dirs.contains(d));

        // 如果引发生态顶级目录改变，或变更代码文件 > 10 个 -> ARCHITECTURE_UPDATE
        if has_new_top_dir || structural_count > 10 {
            let reason = if has_new_top_dir {
                format!(
                    "New top-level directory detected in changes (affected: {:?}) - architecture re-analysis needed",
                    changed_dirs
                )
            } else {
                format!(
                    "{} structural files changed (>10 files) - architecture and tour re-analysis needed",
                    structural_count
                )
            };

            return UpdateDecision {
                action: UpdateAction::ArchitectureUpdate,
                files_to_reanalyze: structural_files,
                rerun_architecture: true,
                rerun_tour: true,
                reason,
                total_structural_changes: structural_count,
                cosmetic_only_count: cosmetic_count,
            };
        }

        // 4. 局部变更 -> PARTIAL_UPDATE
        UpdateDecision {
            action: UpdateAction::PartialUpdate,
            files_to_reanalyze: structural_files,
            rerun_architecture: false,
            rerun_tour: false,
            reason: format!(
                "Localized structural changes in {} file(s) - incremental update only",
                structural_count
            ),
            total_structural_changes: structural_count,
            cosmetic_only_count: cosmetic_count,
        }
    }

    /// 分析项目工作区并生成更新决策
    pub fn classify_changes(
        project_root: &Path,
        db_path: Option<&str>,
    ) -> Result<UpdateDecision, String> {
        let changed = Self::get_git_changed_files(project_root);
        let all_indexed = if let Some(dbp) = db_path {
            Self::get_indexed_files(dbp).unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(Self::classify_from_file_lists(&changed, &all_indexed))
    }

    fn get_git_changed_files(project_root: &Path) -> Vec<String> {
        let mut files = HashSet::new();

        if let Ok(output) = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(project_root)
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let trimmed = line.trim();
                    if trimmed.len() > 3 {
                        let path_part = &trimmed[3..].trim();
                        let clean = if let Some(idx) = path_part.find("->") {
                            path_part[idx + 2..].trim()
                        } else {
                            path_part
                        };
                        files.insert(clean.trim_matches('"').to_string());
                    }
                }
            }
        }

        files.into_iter().collect()
    }

    fn get_indexed_files(db_path: &str) -> Result<Vec<String>, String> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare("SELECT file_path FROM nodes WHERE label = 'File'")
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_empty_changes_is_skip() {
        let decision = ChangeClassifier::classify_from_file_lists(&[], &[]);
        assert_eq!(decision.action, UpdateAction::Skip);
        assert!(!decision.rerun_architecture);
        assert_eq!(decision.total_structural_changes, 0);
    }

    #[test]
    fn test_classify_cosmetic_only_is_skip() {
        let changes = vec![
            "README.md".to_string(),
            "docs/architecture.md".to_string(),
            "assets/logo.png".to_string(),
            "package-lock.json".to_string(),
        ];
        let indexed = vec!["src/main.rs".to_string(), "src/lib.rs".to_string()];
        let decision = ChangeClassifier::classify_from_file_lists(&changes, &indexed);
        assert_eq!(decision.action, UpdateAction::Skip);
        assert_eq!(decision.cosmetic_only_count, 4);
        assert_eq!(decision.total_structural_changes, 0);
    }

    #[test]
    fn test_classify_localized_changes_is_partial() {
        let changes = vec![
            "src/utils.rs".to_string(),
            "src/services/auth.rs".to_string(),
        ];
        let indexed = vec![
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "src/utils.rs".to_string(),
            "src/services/auth.rs".to_string(),
            "src/services/api.rs".to_string(),
        ];
        let decision = ChangeClassifier::classify_from_file_lists(&changes, &indexed);
        assert_eq!(decision.action, UpdateAction::PartialUpdate);
        assert!(!decision.rerun_architecture);
        assert_eq!(decision.total_structural_changes, 2);
    }

    #[test]
    fn test_classify_new_directory_triggers_architecture_update() {
        let changes = vec!["plugins/new_tool.rs".to_string()];
        let indexed = vec!["src/main.rs".to_string(), "src/lib.rs".to_string()];
        let decision = ChangeClassifier::classify_from_file_lists(&changes, &indexed);
        assert_eq!(decision.action, UpdateAction::ArchitectureUpdate);
        assert!(decision.rerun_architecture);
        assert!(decision.rerun_tour);
    }

    #[test]
    fn test_classify_massive_changes_triggers_full_update() {
        let changes: Vec<String> = (0..35).map(|i| format!("src/file_{}.rs", i)).collect();
        let indexed: Vec<String> = (0..50).map(|i| format!("src/file_{}.rs", i)).collect();
        let decision = ChangeClassifier::classify_from_file_lists(&changes, &indexed);
        assert_eq!(decision.action, UpdateAction::FullUpdate);
        assert!(decision.rerun_architecture);
        assert!(decision.rerun_tour);
    }
}

