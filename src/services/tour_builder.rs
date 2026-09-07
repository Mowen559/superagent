use anyhow::{Context, Result};
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

pub struct TourBuilder {
    // 依赖注入
}

impl TourBuilder {
    pub fn new() -> Self {
        Self {}
    }

    /// 构建基于依赖关系的导读大纲（拓扑排序），返回 Markdown 格式的内容
    pub fn build_tour(&self, db_path: &str) -> Result<String> {
        info!("TourBuilder: Starting tour generation using topological sort");

        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Ok(String::new());
        }

        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("Failed to open graph DB for tour builder")?;

        // 1. 获取核心模块节点 (比如 Class, Trait, Struct)
        let mut stmt = conn.prepare(
            "SELECT id, name FROM nodes WHERE label IN ('Class', 'Struct', 'Interface', 'Trait')",
        )?;
        let nodes: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        if nodes.is_empty() {
            info!("No core structures found to build a tour.");
            return Ok(String::from(
                "No core structures found in the project graph to build a tour.",
            ));
        }

        // 2. 将边表导入到 petgraph 构建模块依赖图
        let mut graph = DiGraph::<String, ()>::new();
        let mut node_map: HashMap<i64, NodeIndex> = HashMap::new();

        for (id, name) in &nodes {
            let idx = graph.add_node(name.clone());
            node_map.insert(*id, idx);
        }

        let mut edge_stmt =
            conn.prepare("SELECT source_id, target_id FROM edges WHERE type = 'CALLS'")?;
        let edge_iter =
            edge_stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;

        for edge_res in edge_iter {
            if let Ok((src, tgt)) = edge_res {
                if let (Some(&src_idx), Some(&tgt_idx)) = (node_map.get(&src), node_map.get(&tgt)) {
                    // source 依赖 target，所以阅读顺序应该是先 target 后 source。
                    // 因此构建一条从 tgt 到 src 的边用于拓扑排序。
                    graph.add_edge(tgt_idx, src_idx, ());
                }
            }
        }

        // 3. 执行拓扑排序
        let mut output = String::from("# Codebase Guided Tour\n\nBased on dependency analysis, here is the recommended reading order to understand the codebase:\n\n");
        match toposort(&graph, None) {
            Ok(sorted_indices) => {
                let mut tour_steps = Vec::new();
                for idx in sorted_indices {
                    tour_steps.push(graph[idx].clone());
                }

                info!("Generated Guided Tour Order ({} steps)", tour_steps.len());
                for (i, step) in tour_steps.iter().enumerate() {
                    output.push_str(&format!("{}. **{}**\n", i + 1, step));
                }
            }
            Err(cycle) => {
                warn!("Cycle detected in dependencies! Cannot perform strict topological sort. Cyclic node ID: {:?}", cycle.node_id());
                output.push_str("> [!WARNING]\n> Cyclic dependencies detected. A strict topological sort is not possible. The following guide might be incomplete.\n\n");
            }
        }

        Ok(output)
    }
}
