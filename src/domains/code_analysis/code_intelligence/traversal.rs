use super::types::{DeadCodeSymbol, GraphSchema, TraceHop, TracePathResult};
use super::CodeIntelligenceRuntime;

impl CodeIntelligenceRuntime {
    /// 路径追踪 (Trace Path)：沿 CALLS/IMPORTS/HTTP_CALLS 多跳追踪调用链与跨服务依赖
    pub fn trace_path(
        &self,
        function_name: &str,
        direction: &str,
        depth: usize,
        _mode: &str,
    ) -> Result<TracePathResult, String> {
        let conn = self.open_ro()?;
        let target_name = function_name.trim();

        // 1. 定位起点节点
        let mut start_stmt = conn
            .prepare(
                "SELECT id, name, label, file_path, properties FROM nodes \
                 WHERE qualified_name = ?1 OR name = ?1 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;

        let root_node = start_stmt
            .query_row(rusqlite::params![target_name], Self::map_node_row)
            .map_err(|_| format!("Root symbol '{}' not found in graph", target_name))?;

        let root_id_int: i64 = root_node.id.parse().unwrap_or(0);

        let mut hops: Vec<TraceHop> = Vec::new();
        let mut visited_edges: std::collections::HashSet<(i64, i64, String)> =
            std::collections::HashSet::new();
        let mut visited_nodes: std::collections::HashSet<i64> = std::collections::HashSet::new();
        visited_nodes.insert(root_id_int);

        let mut current_layer: Vec<i64> = vec![root_id_int];
        let max_depth = depth.clamp(1, 10);

        for _ in 0..max_depth {
            if current_layer.is_empty() {
                break;
            }

            let placeholders = current_layer
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");

            let mut next_layer = Vec::new();

            // 根据 direction 决定查询方向
            // inbound: 逆向查询谁调用了 current (target_id in current)
            // outbound: 正向查询 current 调用了谁 (source_id in current)
            // both: 包含双向
            let query_inbound = direction.eq_ignore_ascii_case("inbound") || direction.eq_ignore_ascii_case("both");
            let query_outbound = direction.eq_ignore_ascii_case("outbound") || direction.eq_ignore_ascii_case("both");

            if query_outbound {
                let sql = format!(
                    "SELECT e.source_id, sn.name, e.target_id, tn.name, e.type \
                     FROM edges e \
                     JOIN nodes sn ON sn.id = e.source_id \
                     JOIN nodes tn ON tn.id = e.target_id \
                     WHERE e.source_id IN ({}) AND e.type IN ('CALLS', 'HTTP_CALLS', 'IMPORTS', 'USAGE')",
                    placeholders
                );
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                let params: Vec<&dyn rusqlite::ToSql> =
                    current_layer.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok());

                for (src_id, src_name, tgt_id, tgt_name, edge_type) in rows {
                    if visited_edges.insert((src_id, tgt_id, edge_type.clone())) {
                        hops.push(TraceHop {
                            from_id: src_id.to_string(),
                            from_name: src_name,
                            to_id: tgt_id.to_string(),
                            to_name: tgt_name,
                            edge_type,
                        });
                        if visited_nodes.insert(tgt_id) {
                            next_layer.push(tgt_id);
                        }
                    }
                }
            }

            if query_inbound {
                let sql = format!(
                    "SELECT e.source_id, sn.name, e.target_id, tn.name, e.type \
                     FROM edges e \
                     JOIN nodes sn ON sn.id = e.source_id \
                     JOIN nodes tn ON tn.id = e.target_id \
                     WHERE e.target_id IN ({}) AND e.type IN ('CALLS', 'HTTP_CALLS', 'IMPORTS', 'USAGE')",
                    placeholders
                );
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                let params: Vec<&dyn rusqlite::ToSql> =
                    current_layer.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok());

                for (src_id, src_name, tgt_id, tgt_name, edge_type) in rows {
                    if visited_edges.insert((src_id, tgt_id, edge_type.clone())) {
                        hops.push(TraceHop {
                            from_id: src_id.to_string(),
                            from_name: src_name,
                            to_id: tgt_id.to_string(),
                            to_name: tgt_name,
                            edge_type,
                        });
                        if visited_nodes.insert(src_id) {
                            next_layer.push(src_id);
                        }
                    }
                }
            }

            current_layer = next_layer;
        }

        // 收集所有可达节点的详情
        let reachable_ids: Vec<i64> = visited_nodes
            .into_iter()
            .filter(|&id| id != root_id_int)
            .collect();

        let mut reachable_nodes = Vec::new();
        if !reachable_ids.is_empty() {
            let placeholders = reachable_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id, name, label, file_path, properties FROM nodes WHERE id IN ({}) LIMIT 200",
                placeholders
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let params: Vec<&dyn rusqlite::ToSql> =
                reachable_ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            reachable_nodes = stmt
                .query_map(rusqlite::params_from_iter(params), Self::map_node_row)
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
        }

        let total_hops = hops.len();

        Ok(TracePathResult {
            root_node,
            direction: direction.to_string(),
            depth: max_depth,
            hops,
            total_hops,
            reachable_nodes,
        })
    }

    /// 获取图谱 Schema 结构（节点 labels 与边 types 的统计分布）
    pub fn get_graph_schema(&self) -> Result<GraphSchema, String> {
        let conn = self.open_ro()?;
        let total_nodes: usize = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
            .unwrap_or(0);
        let total_edges: usize = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap_or(0);

        let mut stmt = conn
            .prepare("SELECT label, COUNT(*) FROM nodes GROUP BY label")
            .map_err(|e| e.to_string())?;
        let mut node_labels = std::collections::BTreeMap::new();
        let _ = stmt
            .query_map([], |row| {
                let label: String = row.get(0)?;
                let count: usize = row.get(1)?;
                node_labels.insert(label, count);
                Ok(())
            })
            .map(|iter| iter.for_each(drop));

        let mut stmt2 = conn
            .prepare("SELECT type, COUNT(*) FROM edges GROUP BY type")
            .map_err(|e| e.to_string())?;
        let mut edge_types = std::collections::BTreeMap::new();
        let _ = stmt2
            .query_map([], |row| {
                let edge_type: String = row.get(0)?;
                let count: usize = row.get(1)?;
                edge_types.insert(edge_type, count);
                Ok(())
            })
            .map(|iter| iter.for_each(drop));

        Ok(GraphSchema {
            node_labels,
            edge_types,
            total_nodes,
            total_edges,
        })
    }

    /// 死代码/孤儿符号挖掘：入度为 0 且非入口导出的函数/方法
    pub fn get_dead_code(&self, limit: usize) -> Result<Vec<DeadCodeSymbol>, String> {
        let conn = self.open_ro()?;
        let sql = "SELECT n.id, n.name, n.qualified_name, n.label, n.file_path, n.start_line \
                   FROM nodes n \
                   WHERE n.label IN ('Function', 'Method') \
                     AND n.file_path <> '' \
                     AND n.file_path NOT LIKE '%/tests/%' \
                     AND n.file_path NOT LIKE '%/test/%' \
                     AND n.file_path NOT LIKE '%_test.%' \
                     AND n.file_path NOT LIKE '%main.%' \
                     AND n.file_path NOT LIKE '%index.%' \
                     AND n.file_path NOT LIKE '%lib.%' \
                     AND n.name NOT IN ('main', 'new', 'default', 'init', 'test') \
                     AND NOT EXISTS ( \
                         SELECT 1 FROM edges e WHERE e.target_id = n.id AND e.type IN ('CALLS', 'USAGE') \
                     ) \
                   ORDER BY n.file_path, n.start_line \
                   LIMIT ?1";

        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(DeadCodeSymbol {
                    id: row.get::<_, i64>(0)?.to_string(),
                    name: row.get::<_, String>(1)?,
                    qualified_name: row.get::<_, String>(2)?,
                    label: row.get::<_, String>(3)?,
                    file_path: row.get::<_, String>(4)?,
                    start_line: row.get::<_, Option<i64>>(5)?.unwrap_or(1) as usize,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }
}
