//! Graph Freshness and Git drift detection service.
//!
//! Inspired by UnderstandAnything's `staleness.ts`. Detects whether the
//! knowledge graph database is synchronized with the physical Git repository,
//! computing drift metrics (behind, ahead, diverged, dirty files).

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use rusqlite::Connection;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FreshnessStatus {
    Fresh,
    Dirty,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DriftRelation {
    None,
    Behind,
    Ahead,
    Diverged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphFreshnessResult {
    pub status: FreshnessStatus,
    pub relation: DriftRelation,
    pub graph_commit: Option<String>,
    pub head_commit: Option<String>,
    pub commits_behind: usize,
    pub commits_ahead: usize,
    pub changed_files: Vec<String>,
    pub last_analyzed_at: Option<String>,
}

pub struct GraphFreshnessDetector;

impl GraphFreshnessDetector {
    /// Inspects the given database and repository path to assess freshness
    pub fn check_freshness<P: AsRef<Path>>(db_path: P, repo_path: P) -> GraphFreshnessResult {
        let db = db_path.as_ref();
        let repo = repo_path.as_ref();

        if !db.exists() {
            return GraphFreshnessResult {
                status: FreshnessStatus::Unknown,
                relation: DriftRelation::None,
                graph_commit: None,
                head_commit: None,
                commits_behind: 0,
                commits_ahead: 0,
                changed_files: Vec::new(),
                last_analyzed_at: None,
            };
        }

        let (graph_commit, last_analyzed_at) = Self::read_graph_metadata(db);

        let head_commit = Self::get_git_head(repo);
        let dirty_files = Self::get_git_dirty_files(repo);

        match (graph_commit.as_ref(), head_commit.as_ref()) {
            (Some(gc), Some(hc)) if gc == hc => {
                if dirty_files.is_empty() {
                    GraphFreshnessResult {
                        status: FreshnessStatus::Fresh,
                        relation: DriftRelation::None,
                        graph_commit,
                        head_commit,
                        commits_behind: 0,
                        commits_ahead: 0,
                        changed_files: Vec::new(),
                        last_analyzed_at,
                    }
                } else {
                    GraphFreshnessResult {
                        status: FreshnessStatus::Dirty,
                        relation: DriftRelation::None,
                        graph_commit,
                        head_commit,
                        commits_behind: 0,
                        commits_ahead: 0,
                        changed_files: dirty_files,
                        last_analyzed_at,
                    }
                }
            }
            (Some(gc), Some(hc)) => {
                let (behind, ahead, relation) = Self::compute_git_drift(repo, gc, hc);
                GraphFreshnessResult {
                    status: FreshnessStatus::Stale,
                    relation,
                    graph_commit,
                    head_commit,
                    commits_behind: behind,
                    commits_ahead: ahead,
                    changed_files: dirty_files,
                    last_analyzed_at,
                }
            }
            _ => GraphFreshnessResult {
                status: FreshnessStatus::Unknown,
                relation: DriftRelation::None,
                graph_commit,
                head_commit,
                commits_behind: 0,
                commits_ahead: 0,
                changed_files: dirty_files,
                last_analyzed_at,
            },
        }
    }

    fn read_graph_metadata(db_path: &Path) -> (Option<String>, Option<String>) {
        let conn = match Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(_) => return (None, None),
        };

        // Attempt reading from project_summaries or metadata table
        let mut commit = None;
        let mut analyzed_at = None;

        if let Ok(mut stmt) = conn.prepare("SELECT summary, created_at FROM project_summaries LIMIT 1") {
            if let Ok(row) = stmt.query_row([], |r| {
                let summary: String = r.get(0)?;
                let created: Option<String> = r.get(1).ok();
                Ok((summary, created))
            }) {
                analyzed_at = row.1;
                // Check if commit hash is embedded in summary or fallback to table modification
                if let Some(pos) = row.0.find("commit:") {
                    let part = &row.0[pos + 7..];
                    commit = part.split_whitespace().next().map(|s| s.to_string());
                }
            }
        }

        (commit, analyzed_at)
    }

    fn get_git_head(repo_path: &Path) -> Option<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_path)
            .output()
            .ok()?;

        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }

    fn get_git_dirty_files(repo_path: &Path) -> Vec<String> {
        let output = match Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(repo_path)
            .output()
        {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        if !output.status.success() {
            return Vec::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.len() > 3 {
                    Some(line[3..].trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    fn compute_git_drift(
        repo_path: &Path,
        graph_commit: &str,
        head_commit: &str,
    ) -> (usize, usize, DriftRelation) {
        // Count commits ahead: graph_commit..head_commit
        let ahead_out = Command::new("git")
            .args(["rev-list", "--count", &format!("{}..{}", graph_commit, head_commit)])
            .current_dir(repo_path)
            .output();

        let commits_ahead = ahead_out
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<usize>().ok())
            .unwrap_or(0);

        // Count commits behind: head_commit..graph_commit
        let behind_out = Command::new("git")
            .args(["rev-list", "--count", &format!("{}..{}", head_commit, graph_commit)])
            .current_dir(repo_path)
            .output();

        let commits_behind = behind_out
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<usize>().ok())
            .unwrap_or(0);

        let relation = match (commits_behind > 0, commits_ahead > 0) {
            (true, true) => DriftRelation::Diverged,
            (true, false) => DriftRelation::Behind,
            (false, true) => DriftRelation::Ahead,
            (false, false) => DriftRelation::None,
        };

        (commits_behind, commits_ahead, relation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freshness_non_existent_db() {
        let res = GraphFreshnessDetector::check_freshness("non_existent.db", ".");
        assert_eq!(res.status, FreshnessStatus::Unknown);
        assert_eq!(res.relation, DriftRelation::None);
    }
}
