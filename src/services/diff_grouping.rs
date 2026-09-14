//! 大规模变更拓扑分组与 Token 预算控制 (Diff Grouping & Budget Estimation)
//!
//! 借鉴 Alibaba open-code-review 生产级架构设计：
//! 1. 当代码变更涉及大量文件或数千行代码时，按目录边界与拓扑关联将大 Diff
//!    智能拆解为高内聚、低耦合的独立处理单元（Review/Change Group）；
//! 2. 对每个 Group 实行精准 Token 预算评估，超限时触发自适应切分与降级保护，
//!    防止大模型因 Context 膨胀发生注意力涣散（Lost in the Middle）或超时崩溃。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const DEFAULT_MAX_FILES_PER_GROUP: usize = 5;
pub const DEFAULT_MAX_LINES_PER_GROUP: usize = 600;
pub const DEFAULT_TOKEN_BUDGET_PER_GROUP: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffEntry {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub status: String,
}

impl DiffEntry {
    pub fn new(path: impl Into<String>, additions: usize, deletions: usize, status: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            additions,
            deletions,
            status: status.into(),
        }
    }

    pub fn total_changed_lines(&self) -> usize {
        self.additions + self.deletions
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeGroup {
    pub group_id: usize,
    pub title: String,
    pub directory_scope: String,
    pub files: Vec<DiffEntry>,
    pub total_lines_changed: usize,
    pub estimated_tokens: usize,
    pub requires_sub_chunking: bool,
}

pub struct TopologyDiffGrouper {
    pub max_files_per_group: usize,
    pub max_lines_per_group: usize,
    pub token_budget_per_group: usize,
}

impl Default for TopologyDiffGrouper {
    fn default() -> Self {
        Self {
            max_files_per_group: DEFAULT_MAX_FILES_PER_GROUP,
            max_lines_per_group: DEFAULT_MAX_LINES_PER_GROUP,
            token_budget_per_group: DEFAULT_TOKEN_BUDGET_PER_GROUP,
        }
    }
}

impl TopologyDiffGrouper {
    pub fn new(max_files: usize, max_lines: usize, token_budget: usize) -> Self {
        Self {
            max_files_per_group: max_files,
            max_lines_per_group: max_lines,
            token_budget_per_group: token_budget,
        }
    }

    /// 将所有变更文件根据目录作用域与拓扑亲和度聚类分组，并评估预算
    pub fn group_diffs(&self, entries: &[DiffEntry]) -> Vec<ChangeGroup> {
        if entries.is_empty() {
            return Vec::new();
        }

        // 1. 按顶层/次级目录提取模块范围进行初步聚类
        let mut cluster_map: BTreeMap<String, Vec<DiffEntry>> = BTreeMap::new();
        for entry in entries {
            let scope = extract_directory_scope(&entry.path);
            cluster_map.entry(scope).or_default().push(entry.clone());
        }

        let mut groups = Vec::new();
        let mut next_group_id = 1;

        // 2. 遍历各目录聚类，若超出单组最大文件数或最大行数，则线性拆解
        for (scope, cluster_files) in cluster_map {
            let mut current_batch: Vec<DiffEntry> = Vec::new();
            let mut current_lines = 0;

            for file in cluster_files {
                let file_lines = file.total_changed_lines().max(1);

                let will_exceed_files = current_batch.len() >= self.max_files_per_group;
                let will_exceed_lines = current_lines + file_lines > self.max_lines_per_group;

                if !current_batch.is_empty() && (will_exceed_files || will_exceed_lines) {
                    groups.push(self.create_group(next_group_id, &scope, std::mem::take(&mut current_batch)));
                    next_group_id += 1;
                    current_lines = 0;
                }

                current_lines += file_lines;
                current_batch.push(file);
            }

            if !current_batch.is_empty() {
                groups.push(self.create_group(next_group_id, &scope, current_batch));
                next_group_id += 1;
            }
        }

        groups
    }

    fn create_group(&self, group_id: usize, scope: &str, files: Vec<DiffEntry>) -> ChangeGroup {
        let total_lines_changed: usize = files.iter().map(|f| f.total_changed_lines()).sum();
        let estimated_tokens = estimate_group_tokens(&files);
        let requires_sub_chunking = estimated_tokens > self.token_budget_per_group;

        let title = if files.len() == 1 {
            format!("Group {}: {}", group_id, files[0].path)
        } else {
            format!("Group {}: {} ({} files)", group_id, scope, files.len())
        };

        ChangeGroup {
            group_id,
            title,
            directory_scope: scope.to_string(),
            files,
            total_lines_changed,
            estimated_tokens,
            requires_sub_chunking,
        }
    }
}

fn extract_directory_scope(path_str: &str) -> String {
    let p = Path::new(path_str);
    if let Some(parent) = p.parent() {
        let parent_str = parent.to_string_lossy().replace('\\', "/");
        if parent_str.is_empty() {
            "root".to_string()
        } else {
            // 取前两级目录作为范围，例如 src/services
            let parts: Vec<&str> = parent_str.split('/').filter(|s| !s.is_empty()).collect();
            if parts.len() <= 2 {
                parent_str
            } else {
                format!("{}/{}", parts[0], parts[1])
            }
        }
    } else {
        "root".to_string()
    }
}

/// 针对变更组评估 Token 开销：
/// 基础提示词开销 (~1200) + 每行变更约 4 个 Token + 上下文代码窥视估算
fn estimate_group_tokens(files: &[DiffEntry]) -> usize {
    const PROMPT_PRELUDE_TOKENS: usize = 1200;
    const TOKENS_PER_DIFF_LINE: usize = 5;
    const CONTEXT_OVERHEAD_PER_FILE: usize = 350;

    let diff_lines: usize = files.iter().map(|f| f.total_changed_lines()).sum();
    let file_count = files.len();

    PROMPT_PRELUDE_TOKENS + (diff_lines * TOKENS_PER_DIFF_LINE) + (file_count * CONTEXT_OVERHEAD_PER_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_files_by_directory_scope() {
        let grouper = TopologyDiffGrouper::default();
        let entries = vec![
            DiffEntry::new("src/services/a.rs", 20, 5, "MODIFIED"),
            DiffEntry::new("src/services/b.rs", 15, 2, "MODIFIED"),
            DiffEntry::new("src/config/c.rs", 50, 10, "MODIFIED"),
            DiffEntry::new("tests/t1.rs", 80, 0, "NEW"),
        ];

        let groups = grouper.group_diffs(&entries);
        // Should produce clusters based on directory scope
        assert!(groups.len() >= 3);
        assert!(groups.iter().any(|g| g.directory_scope.contains("src/services")));
        assert!(groups.iter().any(|g| g.directory_scope.contains("src/config")));
        assert!(groups.iter().any(|g| g.directory_scope.contains("tests")));
    }

    #[test]
    fn splits_large_clusters_exceeding_file_count_limit() {
        let grouper = TopologyDiffGrouper::new(2, 1000, 10_000);
        let entries = vec![
            DiffEntry::new("src/services/a.rs", 10, 0, "MODIFIED"),
            DiffEntry::new("src/services/b.rs", 10, 0, "MODIFIED"),
            DiffEntry::new("src/services/c.rs", 10, 0, "MODIFIED"),
        ];

        let groups = grouper.group_diffs(&entries);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].files.len(), 2);
        assert_eq!(groups[1].files.len(), 1);
    }

    #[test]
    fn flags_sub_chunking_when_budget_exceeded() {
        let grouper = TopologyDiffGrouper::new(5, 5000, 2000);
        let entries = vec![
            // 600 lines diff -> 1200 + 600*5 + 350 = 4550 > budget 2000
            DiffEntry::new("src/big.rs", 500, 100, "MODIFIED"),
        ];

        let groups = grouper.group_diffs(&entries);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].requires_sub_chunking);
    }
}
