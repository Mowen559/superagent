use std::path::Path;
use regex::Regex;
use crate::domains::version_control::diff_grouping::{DiffEntry, TopologyDiffGrouper};
use crate::domains::version_control::git_service::{self, RepoKind};
use super::rules::RuleRegistry;
use super::types::{ReviewCategory, ReviewComment, ReviewReport, ReviewSeverity, ReviewStats};

/// 五重门文件选择与审查引擎
pub struct CodeReviewEngine {
    workspace_root: std::path::PathBuf,
    rules: RuleRegistry,
    grouper: TopologyDiffGrouper,
}

impl CodeReviewEngine {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        let rules = RuleRegistry::load_workspace_rules(&workspace_root);
        Self {
            workspace_root,
            rules,
            grouper: TopologyDiffGrouper::default(),
        }
    }

    /// 五重门文件过滤判定
    pub fn is_file_eligible_for_review(path_str: &str) -> bool {
        let p = path_str.replace('\\', "/").to_lowercase();

        // 0. 目录级大噪声过滤 (Provider Ignore Dirs)
        if p.contains("/target/")
            || p.starts_with("target/")
            || p.contains("/node_modules/")
            || p.starts_with("node_modules/")
            || p.contains("/vendor/")
            || p.starts_with("vendor/")
            || p.contains("/.git/")
            || p.starts_with(".git/")
        {
            return false;
        }

        // 1. 二进制与静态资源过滤
        if p.ends_with(".png")
            || p.ends_with(".jpg")
            || p.ends_with(".jpeg")
            || p.ends_with(".gif")
            || p.ends_with(".ico")
            || p.ends_with(".svg")
            || p.ends_with(".exe")
            || p.ends_with(".dll")
            || p.ends_with(".so")
            || p.ends_with(".dylib")
            || p.ends_with(".bin")
            || p.ends_with(".pdf")
            || p.ends_with(".lock")
        {
            return false;
        }

        // 2. 内置测试文件排除 (Gate 5: default_test_path)
        if p.ends_with("_test.go")
            || p.ends_with("_test.rs")
            || p.ends_with(".test.ts")
            || p.ends_with(".test.tsx")
            || p.ends_with(".test.js")
            || p.ends_with(".test.jsx")
            || p.ends_with(".spec.ts")
            || p.ends_with(".spec.js")
            || p.contains("/__tests__/")
            || p.contains("/tests/")
        {
            return false;
        }

        // 3. 语言扩展名白名单 (Gate 4: supported_ext)
        let supported_extensions = [
            ".rs", ".go", ".java", ".py", ".ts", ".tsx", ".js", ".jsx", ".c", ".cpp", ".cc",
            ".h", ".hpp", ".cs", ".kt", ".swift", ".sql", ".sh", ".bash",
        ];

        supported_extensions.iter().any(|ext| p.ends_with(ext))
    }

    /// 执行代码审查核心流水线
    pub fn review_workspace(&self, target_files: Option<&[String]>) -> Result<ReviewReport, String> {
        let ws_str = self.workspace_root.to_string_lossy().to_string();
        let status = git_service::get_status(&ws_str).unwrap_or(git_service::GitStatus {
            is_git_repo: false,
            branch: "".to_string(),
            modified: vec![],
            added: vec![],
            deleted: vec![],
            untracked: vec![],
        });

        let mut changed_files = Vec::new();
        changed_files.extend(status.modified);
        changed_files.extend(status.added);
        changed_files.extend(status.untracked);

        if let Some(targets) = target_files {
            if changed_files.is_empty() {
                changed_files.extend(targets.iter().cloned());
            }
        }

        let mut eligible_entries = Vec::new();
        let mut filtered_count = 0;

        for file in &changed_files {
            if let Some(targets) = target_files {
                if !targets.iter().any(|t| t == file) {
                    continue;
                }
            }

            if Self::is_file_eligible_for_review(file) {
                let diff_res = git_service::get_file_diff(&ws_str, file, None, None, RepoKind::Local);
                let (additions, deletions) = match diff_res {
                    Ok(r) => (r.additions, r.deletions),
                    Err(_) => (10, 0),
                };
                eligible_entries.push(DiffEntry::new(file.clone(), additions, deletions, "modified"));
            } else {
                filtered_count += 1;
            }
        }

        let groups = self.grouper.group_diffs(&eligible_entries);
        let mut comments = Vec::new();

        // 针对每个文件扫描规则缺陷
        for entry in &eligible_entries {
            let full_path = self.workspace_root.join(&entry.path);
            if full_path.exists() && full_path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&full_path) {
                    let mut file_comments = self.scan_file_content(&entry.path, &content);
                    comments.append(&mut file_comments);
                }
            }
        }

        // 终态审查净化器 (Review Filter): 剔除重复和低置信度警告
        let comments = self.filter_and_purify_comments(comments);
        let stats = ReviewStats::calculate(&comments);

        let risk_level = if stats.critical_count > 0 {
            "CRITICAL".to_string()
        } else if stats.warning_count > 2 {
            "HIGH".to_string()
        } else if stats.warning_count > 0 {
            "MEDIUM".to_string()
        } else {
            "LOW".to_string()
        };

        let summary = format!(
            "Analyzed {} files across {} topological groups (filtered {} noise files). Found {} potential issues ({} critical, {} warnings).",
            eligible_entries.len(),
            groups.len(),
            filtered_count,
            stats.total_comments,
            stats.critical_count,
            stats.warning_count
        );

        Ok(ReviewReport {
            mode: "workspace".to_string(),
            total_files_analyzed: eligible_entries.len(),
            filtered_files_count: filtered_count,
            groups_count: groups.len(),
            risk_level,
            summary,
            comments,
            stats,
        })
    }

    /// 扫描单个文件内容，匹配规则库
    pub fn scan_file_content(&self, file_path: &str, content: &str) -> Vec<ReviewComment> {
        let mut comments = Vec::new();
        let applicable_rules = self.rules.find_rules_for_file(file_path);

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx + 1;
            let trimmed = line.trim();
            // 跳过纯注释行
            if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
                continue;
            }

            for rule in &applicable_rules {
                for pattern_str in &rule.patterns {
                    if let Ok(re) = Regex::new(pattern_str) {
                        if re.is_match(line) {
                            comments.push(ReviewComment {
                                file_path: file_path.to_string(),
                                line_number: line_num,
                                severity: rule.severity,
                                category: rule.category,
                                rule_id: Some(rule.id.clone()),
                                title: rule.name.clone(),
                                detail: format!("Rule violation detected: {}. Matching line: `{}`", rule.description, trimmed),
                                suggestion: Some("Inspect and refactor to eliminate potential risk.".to_string()),
                            });
                            break;
                        }
                    }
                }
            }
        }

        comments
    }

    /// 终态净化过滤：去重与误报消除
    fn filter_and_purify_comments(&self, comments: Vec<ReviewComment>) -> Vec<ReviewComment> {
        let mut purified = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for comment in comments {
            let key = format!("{}:{}:{:?}", comment.file_path, comment.line_number, comment.rule_id);
            if seen.insert(key) {
                purified.push(comment);
            }
        }

        purified
    }
}
