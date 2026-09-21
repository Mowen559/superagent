use anyhow::{Context, Result};
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourStep {
    pub step: usize,
    pub name: String,
    pub file_path: Option<String>,
    pub in_cycle: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidedTourReport {
    pub total_steps: usize,
    pub has_cycles: bool,
    pub steps: Vec<TourStep>,
    pub markdown: String,
}

pub struct TourBuilder {}

impl TourBuilder {
    pub fn new() -> Self {
        Self {}
    }

    /// 构建基于依赖关系的导读大纲（拓扑排序），返回 Markdown 格式的内容
    pub fn build_tour(&self, db_path: &str) -> Result<String> {
        let report = self.build_tour_report(db_path)?;
        Ok(report.markdown)
    }

    /// 构建结构化导读大纲，包含步骤详情与 Markdown 文本
    pub fn build_tour_report(&self, db_path: &str) -> Result<GuidedTourReport> {
        info!("TourBuilder: Starting robust tour generation");

        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Ok(GuidedTourReport {
                total_steps: 0,
                has_cycles: false,
                steps: vec![],
                markdown: String::new(),
            });
        }

        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("Failed to open graph DB for tour builder")?;

        // 1. 获取核心模块节点 (比如 Class, Trait, Struct, File, Module)
        let mut stmt = conn.prepare(
            "SELECT id, name, file_path FROM nodes WHERE label IN ('Class', 'Struct', 'Interface', 'Trait', 'Module') LIMIT 200",
        )?;
        let nodes: Vec<(i64, String, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2).ok())))?
            .filter_map(|r| r.ok())
            .collect();

        if nodes.is_empty() {
            info!("No core structures found to build a tour.");
            return Ok(GuidedTourReport {
                total_steps: 0,
                has_cycles: false,
                steps: vec![],
                markdown: String::from(
                    "No core structures found in the project graph to build a tour.",
                ),
            });
        }

        // 2. 将边表导入到 petgraph 构建模块依赖图
        let mut graph = DiGraph::<String, ()>::new();
        let mut node_map: HashMap<i64, NodeIndex> = HashMap::new();
        let mut file_map: HashMap<String, Option<String>> = HashMap::new();

        for (id, name, file_path) in &nodes {
            let idx = graph.add_node(name.clone());
            node_map.insert(*id, idx);
            file_map.insert(name.clone(), file_path.clone());
        }

        let mut edge_stmt =
            conn.prepare("SELECT source_id, target_id FROM edges WHERE type IN ('CALLS', 'IMPORTS', 'EXTENDS', 'IMPLEMENTS')")?;
        let edge_iter =
            edge_stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;

        for edge_res in edge_iter.flatten() {
            let (src, tgt) = edge_res;
            if let (Some(&src_idx), Some(&tgt_idx)) = (node_map.get(&src), node_map.get(&tgt)) {
                // source 依赖 target，所以阅读顺序应该是先 target 后 source。
                // 因此构建一条从 tgt 到 src 的边用于拓扑排序。
                if src_idx != tgt_idx {
                    graph.add_edge(tgt_idx, src_idx, ());
                }
            }
        }

        // 3. 执行健壮拓扑排序（包含贪心破环保障）
        let (sorted_names, has_cycles) = Self::robust_toposort(&graph);

        let mut steps = Vec::new();
        let mut markdown = String::from("# Codebase Guided Tour\n\nBased on dependency topology, here is the recommended reading order to understand the codebase:\n\n");

        if has_cycles {
            markdown.push_str("> [!NOTE]\n> Cyclic dependencies were detected and gracefully resolved via greedy edge breaking. All nodes are included below.\n\n");
        }

        for (i, name) in sorted_names.iter().enumerate() {
            let file_opt = file_map.get(name).cloned().flatten();
            let file_display = file_opt.as_deref().unwrap_or("unknown").to_string();

            steps.push(TourStep {
                step: i + 1,
                name: name.clone(),
                file_path: file_opt,
                in_cycle: has_cycles,
            });

            markdown.push_str(&format!("{}. **{}** (`{}`)\n", i + 1, name, file_display));
        }

        Ok(GuidedTourReport {
            total_steps: steps.len(),
            has_cycles,
            steps,
            markdown,
        })
    }

    /// 健壮拓扑排序：标准拓扑排序优先；当存在循环依赖时，使用贪心破环算法确保全部节点有序输出
    pub fn robust_toposort(graph: &DiGraph<String, ()>) -> (Vec<String>, bool) {
        if let Ok(sorted_indices) = toposort(graph, None) {
            let steps = sorted_indices
                .into_iter()
                .map(|idx| graph[idx].clone())
                .collect();
            return (steps, false);
        }

        let mut in_degrees: HashMap<NodeIndex, usize> = HashMap::new();
        for node in graph.node_indices() {
            in_degrees.insert(
                node,
                graph
                    .neighbors_directed(node, petgraph::Direction::Incoming)
                    .count(),
            );
        }

        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        for (&node, &deg) in &in_degrees {
            if deg == 0 {
                queue.push_back(node);
            }
        }

        let mut result = Vec::new();
        let total_nodes = graph.node_count();

        while result.len() < total_nodes {
            if let Some(curr) = queue.pop_front() {
                result.push(graph[curr].clone());
                in_degrees.remove(&curr);

                for neighbor in graph.neighbors_directed(curr, petgraph::Direction::Outgoing) {
                    if let Some(deg) = in_degrees.get_mut(&neighbor) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(neighbor);
                        }
                    }
                }
            } else {
                // 队列为空但仍有未访问节点，挑出入度最小的节点破环入队
                let candidate = in_degrees
                    .iter()
                    .min_by_key(|(node, in_deg)| {
                        let out_deg = graph
                            .neighbors_directed(**node, petgraph::Direction::Outgoing)
                            .count();
                        (**in_deg, std::cmp::Reverse(out_deg))
                    })
                    .map(|(node, _)| *node);

                if let Some(picked) = candidate {
                    queue.push_back(picked);
                    in_degrees.remove(&picked);
                } else {
                    break;
                }
            }
        }

        (result, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_robust_toposort_with_cycle() {
        let mut graph = DiGraph::<String, ()>::new();
        let a = graph.add_node("A".to_string());
        let b = graph.add_node("B".to_string());
        let c = graph.add_node("C".to_string());

        // A -> B -> C -> A (完全环路)
        graph.add_edge(a, b, ());
        graph.add_edge(b, c, ());
        graph.add_edge(c, a, ());

        let (result, has_cycles) = TourBuilder::robust_toposort(&graph);
        assert!(has_cycles);
        assert_eq!(result.len(), 3);
        assert!(result.contains(&"A".to_string()));
        assert!(result.contains(&"B".to_string()));
        assert!(result.contains(&"C".to_string()));
    }
}
