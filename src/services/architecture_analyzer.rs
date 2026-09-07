use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use tracing::{info, warn};

pub struct ArchitectureAnalyzer {
    llm: crate::domains::model_gateway::providers::LlmProvider,
}

impl ArchitectureAnalyzer {
    pub fn new(llm: crate::domains::model_gateway::providers::LlmProvider) -> Self {
        Self { llm }
    }

    /// 从 Graph DB 提取节点集合，分组并发给大模型进行架构分层（如 Service, Repository, UI）
    pub async fn analyze_layers(&self, db_path: &str, lang: Option<&str>) -> Result<String> {
        info!("ArchitectureAnalyzer: Starting layer analysis");

        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Err(anyhow::anyhow!("Graph DB not found"));
        }

        let file_nodes = {
            let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .context("Failed to open graph DB for architecture analysis")?;

            // 提取主要代码文件节点
            let mut stmt = conn
                .prepare("SELECT id, name, file_path FROM nodes WHERE label = 'File' LIMIT 200")?;
            let files: Vec<(i64, String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2).unwrap_or_default(),
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            info!(
                "ArchitectureAnalyzer: Found {} file nodes for layer classification.",
                files.len()
            );
            files
                .iter()
                .map(|f| format!("{} ({})", f.1, f.2))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let lang_str = lang.unwrap_or("zh-CN");

        let report = self
            .llm
            .explain_architecture(&file_nodes, lang_str)
            .await
            .map_err(|e| anyhow::anyhow!("LLM Error: {}", e))?;

        Ok(report)
    }
}
