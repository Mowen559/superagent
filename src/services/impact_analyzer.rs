use anyhow::{Context, Result};
use git2::{Repository, StatusOptions};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub struct ImpactAnalyzer {
    llm: crate::domains::model_gateway::providers::LlmProvider,
}

#[derive(Debug)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub status: String,
}

impl ImpactAnalyzer {
    pub fn new(llm: crate::domains::model_gateway::providers::LlmProvider) -> Self {
        Self { llm }
    }

    /// 接管仓库，提取有变更的文件列表（影子 Git 分析基础）
    pub fn extract_changed_files(&self, repo_path: &Path) -> Result<Vec<ChangedFile>> {
        let repo = Repository::open(repo_path)
            .context("Failed to open repository for shadow git analysis")?;

        let mut opts = StatusOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);

        let statuses = repo.statuses(Some(&mut opts))?;
        let mut changed_files = Vec::new();

        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("").to_string();
            let status = entry.status();

            if status.is_empty() || status.is_ignored() || path.is_empty() {
                continue;
            }

            let status_str = if status.is_index_new() || status.is_wt_new() {
                "NEW"
            } else if status.is_index_deleted() || status.is_wt_deleted() {
                "DELETED"
            } else {
                "MODIFIED"
            };

            changed_files.push(ChangedFile {
                path: PathBuf::from(path),
                status: status_str.to_string(),
            });
        }

        Ok(changed_files)
    }

    /// 在 Graph DB 中查找依赖这些变更文件的反向节点（爆炸半径推演）
    pub fn find_impacted_nodes(
        &self,
        db_path: &str,
        changed_files: &[ChangedFile],
    ) -> Result<Vec<String>> {
        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Ok(Vec::new());
        }

        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("Failed to open graph DB for impact analysis")?;

        let mut impacted = HashSet::new();

        for file in changed_files {
            let path_str = file.path.to_string_lossy().replace('\\', "/");
            let like_path = format!("%{}%", path_str);

            // 1. 查找此文件中定义的所有节点 (id)
            let mut stmt = conn.prepare("SELECT id FROM nodes WHERE file_path LIKE ?1")?;
            let node_ids: Vec<i64> = stmt
                .query_map(rusqlite::params![like_path], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();

            if node_ids.is_empty() {
                continue;
            }

            // 2. 查找依赖这些节点的所有 source_id (incoming edges)
            let id_list = node_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let edge_query = format!(
                "SELECT DISTINCT source_id FROM edges WHERE target_id IN ({})",
                id_list
            );
            let mut stmt_edges = conn.prepare(&edge_query)?;

            let source_ids: Vec<i64> = stmt_edges
                .query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();

            if source_ids.is_empty() {
                continue;
            }

            // 3. 将 source_id 转换为具有可读性的节点信息
            let source_id_list = source_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let node_query = format!(
                "SELECT name, file_path FROM nodes WHERE id IN ({})",
                source_id_list
            );
            let mut stmt_nodes = conn.prepare(&node_query)?;

            stmt_nodes
                .query_map([], |row| {
                    let name: String = row.get(0).unwrap_or_default();
                    let file_path: Option<String> = row.get(1).ok().flatten();
                    if let Some(fp) = file_path {
                        Ok(format!("{} (in {})", name, fp))
                    } else {
                        Ok(name)
                    }
                })?
                .for_each(|r| {
                    if let Ok(info) = r {
                        impacted.insert(info);
                    }
                });
        }

        let mut result: Vec<String> = impacted.into_iter().collect();
        result.sort();
        Ok(result)
    }

    pub async fn analyze_impact(
        &self,
        workspace_root: &Path,
        db_path: &str,
        lang: Option<&str>,
    ) -> Result<String> {
        info!(
            "ImpactAnalyzer: Starting diff impact analysis for {:?}",
            workspace_root
        );

        // 1. 获取变更列表 (Shadow Git Interception)
        let changed_files = self.extract_changed_files(workspace_root)?;
        if changed_files.is_empty() {
            info!("No changed files detected. Impact radius is 0.");
            return Ok("No changed files detected. Impact radius is 0.".to_string());
        }

        info!("Detected {} changed files:", changed_files.len());
        let changed_files_str = changed_files
            .iter()
            .map(|f| format!("[{}] {}", f.status, f.path.display()))
            .collect::<Vec<_>>()
            .join("\n");

        // 2 & 3. 映射 Node 并提取爆炸半径
        let impacted_nodes = self.find_impacted_nodes(db_path, &changed_files)?;
        let impacted_nodes_str = if impacted_nodes.is_empty() {
            "No reverse dependencies found. Changes are safe (isolated).".to_string()
        } else {
            impacted_nodes.join("\n")
        };

        let lang_str = lang.unwrap_or("zh-CN");
        let report = self
            .llm
            .explain_impact(&changed_files_str, &impacted_nodes_str, lang_str)
            .await
            .map_err(|e| anyhow::anyhow!("LLM Error: {}", e))?;

        Ok(report)
    }
}
