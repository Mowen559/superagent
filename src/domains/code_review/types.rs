use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCategory {
    Bug,
    Security,
    Performance,
    Concurrency,
    ResourceLeak,
    Style,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub file_path: String,
    pub line_number: usize,
    pub severity: ReviewSeverity,
    pub category: ReviewCategory,
    pub rule_id: Option<String>,
    pub title: String,
    pub detail: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRule {
    pub id: String,
    pub name: String,
    pub language: String,
    pub severity: ReviewSeverity,
    pub category: ReviewCategory,
    pub description: String,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewStats {
    pub total_comments: usize,
    pub critical_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
}

impl ReviewStats {
    pub fn calculate(comments: &[ReviewComment]) -> Self {
        let mut stats = Self::default();
        stats.total_comments = comments.len();
        for comment in comments {
            match comment.severity {
                ReviewSeverity::Critical => stats.critical_count += 1,
                ReviewSeverity::Warning => stats.warning_count += 1,
                ReviewSeverity::Info => stats.info_count += 1,
            }
        }
        stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewReport {
    pub mode: String,
    pub total_files_analyzed: usize,
    pub filtered_files_count: usize,
    pub groups_count: usize,
    pub risk_level: String,
    pub summary: String,
    pub comments: Vec<ReviewComment>,
    pub stats: ReviewStats,
}
