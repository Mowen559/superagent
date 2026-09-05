use anyhow::{Context, Result};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Dfs;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

pub struct DomainAnalyzer {
    llm: crate::services::llm_provider::LlmProvider,
}

impl DomainAnalyzer {
    pub fn new(llm: crate::services::llm_provider::LlmProvider) -> Self {
        Self { llm }
    }

    /// 利用 petgraph 构建有向图，并使用 DFS 提取业务骨架
    pub async fn analyze_domain_flows(&self, db_path: &str, lang: Option<&str>) -> Result<String> {
        info!("DomainAnalyzer: Starting domain flow analysis using petgraph");

        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Err(anyhow::anyhow!("Graph DB not found"));
        }

        let (entry_nodes, graph, node_map) = {
            let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .context("Failed to open graph DB for domain analysis")?;

            // 1. 获取所有入口节点 (API / Controller / Route)
            let mut stmt = conn.prepare("SELECT id, name FROM nodes WHERE label IN ('Route', 'Controller', 'Endpoint') LIMIT 50")?;
            let entry_nodes: Vec<(i64, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();

            if entry_nodes.is_empty() {
                info!("No entry points (Route/Controller) found for domain flow analysis.");
                return Ok("No entry points found to trace domain flows.".to_string());
            }

            // 2. 将边表导入到 petgraph
            let mut graph = DiGraph::<i64, String>::new();
            let mut node_map: HashMap<i64, NodeIndex> = HashMap::new();

            let mut edge_stmt =
                conn.prepare("SELECT source_id, target_id, type FROM edges WHERE type = 'CALLS'")?;
            let edge_iter = edge_stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;

            for (src, tgt, edge_type) in edge_iter.flatten() {
                let src_idx = *node_map.entry(src).or_insert_with(|| graph.add_node(src));
                let tgt_idx = *node_map.entry(tgt).or_insert_with(|| graph.add_node(tgt));
                graph.add_edge(src_idx, tgt_idx, edge_type);
            }

            (entry_nodes, graph, node_map)
        };

        info!(
            "Loaded {} nodes and {} edges into memory graph.",
            graph.node_count(),
            graph.edge_count()
        );

        let mut full_report = String::new();
        let lang_str = lang.unwrap_or("zh-CN");

        // 3. 对每个入口节点运行 DFS 提取业务调用链路
        for (entry_id, entry_name) in entry_nodes {
            if let Some(&start_idx) = node_map.get(&entry_id) {
                let mut dfs = Dfs::new(&graph, start_idx);
                let mut downstream = Vec::new();

                while let Some(visited) = dfs.next(&graph) {
                    let node_id = graph[visited];
                    if node_id != entry_id {
                        downstream.push(node_id.to_string());
                    }
                }

                let downstream_str = downstream.join(",");

                match self
                    .llm
                    .explain_domain_flow(&entry_name, &downstream_str, lang_str)
                    .await
                {
                    Ok(report) => {
                        full_report.push_str(&format!(
                            "## Flow for Entry: {}\n\n{}\n\n",
                            entry_name, report
                        ));
                    }
                    Err(e) => {
                        warn!("LLM error generating flow for {}: {}", entry_name, e);
                    }
                }
            }
        }

        Ok(if full_report.is_empty() {
            "Failed to generate domain flows.".to_string()
        } else {
            full_report
        })
    }
}
