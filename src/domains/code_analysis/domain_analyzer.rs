use anyhow::{Context, Result};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Dfs;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowStep {
    pub id: String,
    pub name: String,
    pub label: String,
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainFlow {
    pub entry_id: String,
    pub entry_name: String,
    pub steps: Vec<FlowStep>,
    pub step_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainFlowReport {
    pub flows: Vec<DomainFlow>,
    pub total_flows: usize,
}

pub struct DomainAnalyzer {
    llm: Option<crate::domains::model_gateway::providers::LlmProvider>,
}

impl DomainAnalyzer {
    pub fn new(llm: crate::domains::model_gateway::providers::LlmProvider) -> Self {
        Self { llm: Some(llm) }
    }

    pub fn without_llm() -> Self {
        Self { llm: None }
    }

    /// 提取结构化业务领域调用链路，无需 LLM 介入即可输出明确的流程节点拓扑
    pub fn extract_domain_flows_structured(db_path: &str) -> Result<DomainFlowReport> {
        if !Path::new(db_path).exists() {
            return Err(anyhow::anyhow!("Graph DB not found: {}", db_path));
        }

        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("Failed to open graph DB for domain analysis")?;

        // 1. 获取入口节点 (Route, Controller, Endpoint, Function, Method)
        let mut stmt = conn.prepare(
            "SELECT id, name FROM nodes WHERE label IN ('Route', 'Controller', 'Endpoint') LIMIT 50",
        )?;
        let entry_nodes: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        if entry_nodes.is_empty() {
            return Ok(DomainFlowReport {
                flows: vec![],
                total_flows: 0,
            });
        }

        // 2. 将边表导入到 petgraph
        let mut graph = DiGraph::<i64, String>::new();
        let mut node_map: HashMap<i64, NodeIndex> = HashMap::new();

        let mut edge_stmt =
            conn.prepare("SELECT source_id, target_id, type FROM edges WHERE type IN ('CALLS', 'HTTP_CALLS', 'IMPORTS')")?;
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

        // 缓存所有节点的基本信息以便高效拼装
        let mut info_stmt = conn.prepare("SELECT id, name, label, file_path FROM nodes")?;
        let mut node_info: HashMap<i64, (String, String, Option<String>)> = HashMap::new();
        let _ = info_stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let label: String = row.get(2)?;
                let file: Option<String> = row.get(3).ok();
                node_info.insert(id, (name, label, file));
                Ok(())
            })?
            .for_each(drop);

        let mut flows = Vec::new();

        for (entry_id, entry_name) in entry_nodes {
            if let Some(&start_idx) = node_map.get(&entry_id) {
                let mut dfs = Dfs::new(&graph, start_idx);
                let mut steps = Vec::new();

                while let Some(visited) = dfs.next(&graph) {
                    let node_id = graph[visited];
                    if node_id != entry_id {
                        if let Some((name, label, file)) = node_info.get(&node_id) {
                            steps.push(FlowStep {
                                id: node_id.to_string(),
                                name: name.clone(),
                                label: label.clone(),
                                file_path: file.clone(),
                            });
                        }
                    }
                }

                let step_count = steps.len();
                flows.push(DomainFlow {
                    entry_id: entry_id.to_string(),
                    entry_name,
                    steps,
                    step_count,
                });
            }
        }

        let total_flows = flows.len();
        Ok(DomainFlowReport { flows, total_flows })
    }

    /// 利用 petgraph 构建有向图，并使用 DFS 提取业务骨架与自然语言解读
    pub async fn analyze_domain_flows(&self, db_path: &str, lang: Option<&str>) -> Result<String> {
        info!("DomainAnalyzer: Starting domain flow analysis using petgraph");

        let structured = Self::extract_domain_flows_structured(db_path)?;
        if structured.flows.is_empty() {
            return Ok("No entry points found to trace domain flows.".to_string());
        }

        let mut full_report = String::new();
        let lang_str = lang.unwrap_or("zh-CN");

        if let Some(llm) = &self.llm {
            for flow in &structured.flows {
                let downstream_str = flow
                    .steps
                    .iter()
                    .map(|s| format!("{}({})", s.name, s.label))
                    .collect::<Vec<_>>()
                    .join(", ");

                match llm
                    .explain_domain_flow(&flow.entry_name, &downstream_str, lang_str)
                    .await
                {
                    Ok(report) => {
                        full_report.push_str(&format!(
                            "## Flow for Entry: {}\n\n{}\n\n",
                            flow.entry_name, report
                        ));
                    }
                    Err(e) => {
                        warn!("LLM error generating flow for {}: {}", flow.entry_name, e);
                    }
                }
            }
        } else {
            full_report.push_str("# 业务领域流分析拓扑 (确定性提取)\n\n");
            for flow in &structured.flows {
                full_report.push_str(&format!("### 入口: `{}` (包含 {} 个下游步骤)\n", flow.entry_name, flow.step_count));
                for (idx, step) in flow.steps.iter().take(15).enumerate() {
                    let file = step.file_path.as_deref().unwrap_or("unknown");
                    full_report.push_str(&format!("{}. **{}** [{}] (`{}`)\n", idx + 1, step.name, step.label, file));
                }
                if flow.steps.len() > 15 {
                    full_report.push_str(&format!("- *(及其他 {} 个调用节点)*\n", flow.steps.len() - 15));
                }
                full_report.push_str("\n");
            }
        }

        Ok(if full_report.is_empty() {
            "Failed to generate domain flows.".to_string()
        } else {
            full_report
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_domain_flows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("graph.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                label TEXT NOT NULL,
                file_path TEXT
            );
            CREATE TABLE edges (
                source_id INTEGER,
                target_id INTEGER,
                type TEXT
            );
            INSERT INTO nodes VALUES (1, 'create_user_api', 'Route', 'src/api/user.rs');
            INSERT INTO nodes VALUES (2, 'register_user', 'Function', 'src/services/user_service.rs');
            INSERT INTO nodes VALUES (3, 'insert_db', 'Function', 'src/dao/user_dao.rs');
            INSERT INTO edges VALUES (1, 2, 'CALLS');
            INSERT INTO edges VALUES (2, 3, 'CALLS');
            "#,
        )
        .unwrap();
        drop(conn);

        let report = DomainAnalyzer::extract_domain_flows_structured(db_path.to_str().unwrap()).unwrap();
        assert_eq!(report.total_flows, 1);
        let flow = &report.flows[0];
        assert_eq!(flow.entry_name, "create_user_api");
        assert_eq!(flow.step_count, 2);
        assert_eq!(flow.steps[0].name, "register_user");
        assert_eq!(flow.steps[1].name, "insert_db");
    }
}
