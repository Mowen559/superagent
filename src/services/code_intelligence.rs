use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// Provides unified access to the Code Intelligence graph database
pub struct CodeIntelligenceRuntime {
    db_path: PathBuf,
}

impl CodeIntelligenceRuntime {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
        }
    }

    pub fn is_indexed(&self) -> bool {
        self.db_path.exists()
    }

    pub fn db_path_str(&self) -> String {
        self.db_path.to_string_lossy().to_string()
    }


    pub fn get_file_symbols(&self, rel_path: &str) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, label, qualified_name, start_line, end_line \
                 FROM nodes WHERE file_path = ?1 AND label NOT IN ('File','Module') \
                 ORDER BY start_line LIMIT 500",
            )
            .map_err(|e| e.to_string())?;
            
        let symbols = stmt
            .query_map(rusqlite::params![rel_path], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, i64>(0)?.to_string(),
                    "name": row.get::<_, String>(1)?,
                    "label": row.get::<_, String>(2)?,
                    "qualifiedName": row.get::<_, String>(3)?,
                    "startLine": row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    "endLine": row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                }))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
            
        Ok(symbols)
    }

    pub fn file_relations(&self, rel_path: &str, outgoing: bool) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.open_ro()?;
        let (from_col, to_col) = if outgoing { ("sn", "tn") } else { ("tn", "sn") };
        let types = "'CALLS','IMPORTS','USAGE','HTTP_CALLS','WRITES','EXTENDS','IMPLEMENTS'";
        
        let sql = format!(
            "SELECT {to}.file_path, e.type, COUNT(*) AS cnt \
             FROM edges e \
             JOIN nodes sn ON sn.id = e.source_id \
             JOIN nodes tn ON tn.id = e.target_id \
             WHERE {from}.file_path = ?1 AND {to}.file_path <> '' AND {to}.file_path <> ?1 \
               AND {to}.file_path NOT LIKE '%/assets/%' AND {to}.file_path NOT LIKE '%.min.js' \
               AND e.type IN ({types}) \
             GROUP BY {to}.file_path, e.type",
            to = to_col,
            from = from_col,
            types = types
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows: Vec<(String, String, i64)> = stmt
            .query_map(rusqlite::params![rel_path], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Fold (file, edge_type, count) rows into one entry per file.
        let mut by_file: std::collections::BTreeMap<String, (i64, std::collections::BTreeMap<String, i64>)> = std::collections::BTreeMap::new();
        for (file, edge_type, count) in rows {
            let entry = by_file.entry(file).or_default();
            entry.0 += count;
            entry.1.insert(edge_type, count);
        }

        let mut out: Vec<serde_json::Value> = by_file
            .into_iter()
            .map(|(file, (total, types))| {
                serde_json::json!({
                    "file": file,
                    "total": total,
                    "types": types,
                })
            })
            .collect();
        out.sort_by_key(|v| -v.get("total").and_then(|t| t.as_i64()).unwrap_or(0));
        Ok(out)
    }

    pub fn get_project_summary(&self) -> Result<Option<String>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare("SELECT summary FROM project_summaries LIMIT 1")
            .map_err(|e| e.to_string())?;
        let summary: Option<String> = stmt
            .query_row([], |row| row.get(0))
            .ok();
        Ok(summary)
    }

    pub fn get_file_summaries(&self, limit: i64) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare(
                "SELECT file_path, name, properties FROM nodes \
                 WHERE label = 'File' AND file_path NOT LIKE '%/assets/%' \
                 ORDER BY id LIMIT ?1"
            )
            .map_err(|e| e.to_string())?;

        let files = stmt
            .query_map(rusqlite::params![limit], |row| {
                let file_path: String = row.get(0)?;
                let name: String = row.get(1)?;
                let properties: String = row.get(2)?;
                let parsed_props: serde_json::Value = serde_json::from_str(&properties).unwrap_or_default();
                let summary = parsed_props.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
                
                Ok(serde_json::json!({
                    "file": file_path,
                    "name": name,
                    "summary": summary,
                }))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .filter(|v: &serde_json::Value| {
                v.get("summary").and_then(|s| s.as_str()).unwrap_or("") != ""
            })
            .collect();
        Ok(files)
    }

    pub fn query_nodes_fts(&self, query: &str, limit: usize) -> Result<Vec<crate::models::graph::GraphNode>, String> {
        let conn = self.open_ro()?;
        let fts_query: String = query
            .split_whitespace()
            .map(|t| format!("\"{}\"*", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");

        let sql = "SELECT n.id, n.name, n.label, n.file_path, n.properties \
                   FROM nodes_fts f JOIN nodes n ON n.id = f.rowid \
                   WHERE nodes_fts MATCH ?1 ORDER BY rank LIMIT ?2";

        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![fts_query, limit as i64], Self::map_node_row)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn search_nodes_like(&self, query: &str, limit: usize) -> Result<Vec<crate::models::graph::GraphNode>, String> {
        let conn = self.open_ro()?;
        let like = format!("%{}%", query.trim());
        let mut stmt = conn
            .prepare(
                "SELECT id, name, label, file_path, properties FROM nodes \
                 WHERE name LIKE ?1 OR qualified_name LIKE ?1 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let nodes = stmt
            .query_map(rusqlite::params![like, limit as i64], Self::map_node_row)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(nodes)
    }

    pub fn query_nodes(&self, limit: usize, labels: Option<&[String]>) -> Result<Vec<crate::models::graph::GraphNode>, String> {
        let conn = self.open_ro()?;
        if let Some(lbls) = labels {
            if lbls.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = lbls.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT id, name, label, file_path, properties FROM nodes \
                 WHERE label IN ({}) LIMIT {}",
                placeholders, limit
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let params = rusqlite::params_from_iter(lbls.iter());
            let nodes = stmt
                .query_map(params, Self::map_node_row)
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            Ok(nodes)
        } else {
            let mut stmt = conn
                .prepare("SELECT id, name, label, file_path, properties FROM nodes LIMIT ?1")
                .map_err(|e| e.to_string())?;
            let nodes = stmt
                .query_map(rusqlite::params![limit as i64], Self::map_node_row)
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            Ok(nodes)
        }
    }

    pub fn query_edges(&self, max_edges: usize) -> Result<Vec<crate::models::graph::GraphEdge>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare("SELECT id, source_id, target_id, type, properties FROM edges LIMIT ?1")
            .map_err(|e| e.to_string())?;
            
        let edges = stmt
            .query_map(rusqlite::params![max_edges as i64], |row| {
                let properties: String = row.get(4).unwrap_or_else(|_| "{}".to_string());
                Ok(crate::models::graph::GraphEdge {
                    source: row.get::<_, i64>(1)?.to_string(),
                    target: row.get::<_, i64>(2)?.to_string(),
                    edge_type: row.get(3)?,
                    label: None,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(edges)
    }

    fn map_node_row(row: &rusqlite::Row) -> rusqlite::Result<crate::models::graph::GraphNode> {
        let properties: String = row.get(4).unwrap_or_else(|_| "{}".to_string());
        let props_json: serde_json::Value = serde_json::from_str(&properties).unwrap_or_default();
        let language = props_json.get("language").and_then(|v| v.as_str()).map(String::from);
        Ok(crate::models::graph::GraphNode {
            id: row.get::<_, i64>(0)?.to_string(),
            label: row.get(1)?,
            node_type: row.get(2)?,
            file_path: row.get(3).ok(),
            language,
            summary: None,
            complexity: None,
            tags: vec![],
        })
    }


    pub fn language_breakdown(&self) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare(
                "SELECT json_extract(properties, '$.language') AS lang, COUNT(*) AS cnt \
                 FROM nodes \
                 WHERE label = 'File' AND json_extract(properties, '$.language') IS NOT NULL \
                 GROUP BY lang ORDER BY cnt DESC LIMIT 10",
            )
            .map_err(|e| e.to_string())?;

        let langs = stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "name": row.get::<_, String>(0)?,
                    "files": row.get::<_, i64>(1)?
                }))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(langs)
    }

    pub fn get_project_stats(&self) -> Result<serde_json::Value, String> {
        let conn = self.open_ro()?;
        let total_nodes: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap_or(0);
        let total_edges: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap_or(0);
        let total_files: i64 = conn.query_row("SELECT COUNT(*) FROM nodes WHERE label = 'File'", [], |r| r.get(0)).unwrap_or(0);

        let mut stmt = conn.prepare("SELECT label, COUNT(*) FROM nodes GROUP BY label").unwrap();
        let mut label_dist = serde_json::Map::new();
        let _ = stmt.query_map([], |row| {
            let label: String = row.get(0)?;
            let cnt: i64 = row.get(1)?;
            label_dist.insert(label, serde_json::json!(cnt));
            Ok(())
        }).map(|iter| iter.for_each(drop));

        let mut stmt2 = conn.prepare("SELECT type, COUNT(*) FROM edges GROUP BY type").unwrap();
        let mut edge_dist = serde_json::Map::new();
        let _ = stmt2.query_map([], |row| {
            let edge_type: String = row.get(0)?;
            let cnt: i64 = row.get(1)?;
            edge_dist.insert(edge_type, serde_json::json!(cnt));
            Ok(())
        }).map(|iter| iter.for_each(drop));

        Ok(serde_json::json!({
            "totalNodes": total_nodes,
            "totalEdges": total_edges,
            "totalFiles": total_files,
            "labelDistribution": label_dist,
            "edgeDistribution": edge_dist,
        }))
    }

    pub fn get_hot_symbols(&self, limit: i64) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.name, t.label, t.file_path, COUNT(*) AS inbound \
                 FROM edges e \
                 JOIN nodes t ON t.id = e.target_id \
                 WHERE e.type IN ('CALLS', 'USAGE', 'IMPLEMENTS', 'EXTENDS') \
                   AND t.label NOT IN ('File', 'Module') \
                 GROUP BY t.id ORDER BY inbound DESC LIMIT ?1"
            )
            .map_err(|e| e.to_string())?;

        let hot = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, i64>(0)?.to_string(),
                    "name": row.get::<_, String>(1)?,
                    "label": row.get::<_, String>(2)?,
                    "file": row.get::<_, String>(3)?,
                    "references": row.get::<_, i64>(4)?
                }))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(hot)
    }

    pub fn get_modules(&self, limit: i64) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare("SELECT name, file_path, properties FROM nodes WHERE label = 'Module' LIMIT ?1")
            .map_err(|e| e.to_string())?;

        let modules = stmt
            .query_map(rusqlite::params![limit], |row| {
                let props: String = row.get(2).unwrap_or_else(|_| "{}".into());
                let parsed: serde_json::Value = serde_json::from_str(&props).unwrap_or_default();
                let summary = parsed.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Ok(serde_json::json!({
                    "name": row.get::<_, String>(0)?,
                    "file": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    "summary": summary
                }))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(modules)
    }


    pub fn get_file_nodes(&self, file_path: &str) -> Result<Vec<crate::models::graph::GraphNode>, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, label, file_path, properties FROM nodes WHERE file_path = ?1 LIMIT 200"
        ).map_err(|e| e.to_string())?;

        let nodes = stmt
            .query_map(rusqlite::params![file_path], Self::map_node_row)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(nodes)
    }

    pub fn get_callers(&self, target_id: &str, limit: usize) -> Result<Vec<crate::models::graph::GraphNode>, String> {
        let conn = self.open_ro()?;
        let sql = "SELECT n.id, n.name, n.label, n.file_path, n.properties FROM nodes n JOIN edges e ON n.id = e.source_id WHERE e.target_id = ?1 LIMIT ?2";
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let target_id_int: i64 = target_id.parse().unwrap_or(0);
        let nodes = stmt
            .query_map(rusqlite::params![target_id_int, limit as i64], Self::map_node_row)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(nodes)
    }

    pub fn get_callees(&self, source_id: &str, limit: usize) -> Result<Vec<crate::models::graph::GraphNode>, String> {
        let conn = self.open_ro()?;
        let sql = "SELECT n.id, n.name, n.label, n.file_path, n.properties FROM nodes n JOIN edges e ON n.id = e.target_id WHERE e.source_id = ?1 LIMIT ?2";
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let source_id_int: i64 = source_id.parse().unwrap_or(0);
        let nodes = stmt
            .query_map(rusqlite::params![source_id_int, limit as i64], Self::map_node_row)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(nodes)
    }

    pub fn get_impact(&self, target_id: &str, depth: usize) -> Result<Vec<crate::models::graph::GraphNode>, String> {
        let conn = self.open_ro()?;
        let mut impact_nodes = Vec::new();
        let mut current_level = vec![target_id.to_string()];
        let mut visited = std::collections::HashSet::new();
        visited.insert(target_id.to_string());

        for _ in 0..depth {
            if current_level.is_empty() {
                break;
            }

            let placeholders = current_level.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT n.id, n.name, n.label, n.file_path, n.properties
                 FROM nodes n JOIN edges e ON n.id = e.source_id
                 WHERE e.target_id IN ({})",
                placeholders
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

            let params_vec: Vec<&dyn rusqlite::ToSql> = current_level.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            let nodes: Vec<crate::models::graph::GraphNode> = stmt
                .query_map(rusqlite::params_from_iter(params_vec), Self::map_node_row)
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();

            let mut next_level = Vec::new();
            for node in nodes {
                if visited.insert(node.id.clone()) {
                    next_level.push(node.id.clone());
                    impact_nodes.push(node);
                }
            }
            current_level = next_level;
        }

        Ok(impact_nodes)
    }

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

    /// 获取符号代码片段及其行号、源码上下文与邻居节点
    pub fn get_code_snippet(
        &self,
        qualified_name: &str,
        include_neighbors: bool,
        project_root: Option<&Path>,
    ) -> Result<CodeSnippetResult, String> {
        let conn = self.open_ro()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, label, qualified_name, file_path, start_line, end_line \
                 FROM nodes \
                 WHERE qualified_name = ?1 OR name = ?1 \
                 ORDER BY (qualified_name = ?1) DESC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;

        let (node_id, name, label, qn, file_path, start_line_opt, end_line_opt) = stmt
            .query_row(rusqlite::params![qualified_name], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })
            .map_err(|_| format!("Symbol '{}' not found in graph database", qualified_name))?;

        let start_line = start_line_opt.unwrap_or(1).max(1) as usize;
        let end_line = end_line_opt.unwrap_or(start_line as i64).max(start_line as i64) as usize;

        let content = if let Some(root) = project_root {
            let full_path = root.join(&file_path);
            if full_path.exists() {
                if let Ok(file_str) = std::fs::read_to_string(&full_path) {
                    let lines: Vec<&str> = file_str.lines().collect();
                    let s_idx = start_line.saturating_sub(1).min(lines.len());
                    let e_idx = end_line.min(lines.len());
                    let slice = &lines[s_idx..e_idx];
                    slice
                        .iter()
                        .enumerate()
                        .map(|(idx, line)| format!("{:4} | {}", start_line + idx, line))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    format!("// [Unable to read file contents at {}]", file_path)
                }
            } else {
                format!("// [File not found on disk at {}]", full_path.display())
            }
        } else {
            format!("// [Symbol {} defined in {} lines {}-{}]", qn, file_path, start_line, end_line)
        };

        let mut neighbors = Vec::new();
        if include_neighbors {
            let callers = self.get_callers(&node_id.to_string(), 10).unwrap_or_default();
            let callees = self.get_callees(&node_id.to_string(), 10).unwrap_or_default();
            neighbors.extend(callers);
            neighbors.extend(callees);
        }

        Ok(CodeSnippetResult {
            qualified_name: qn,
            name,
            label,
            file_path,
            start_line,
            end_line,
            content,
            neighbors,
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

    /// 结构化图谱搜索：支持按标签、名称模式、文件通配、度数过滤和分页
    pub fn search_graph(
        &self,
        label: Option<&str>,
        name_pattern: Option<&str>,
        file_pattern: Option<&str>,
        min_degree: Option<usize>,
        max_degree: Option<usize>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<GraphSearchResultItem>, String> {
        let conn = self.open_ro()?;
        let mut where_clauses = vec!["1=1".to_string()];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(lbl) = label.filter(|s| !s.trim().is_empty()) {
            where_clauses.push("n.label = ?".to_string());
            params.push(Box::new(lbl.to_string()));
        }

        if let Some(np) = name_pattern.filter(|s| !s.trim().is_empty()) {
            let like_str = if np.contains('%') || np.contains('*') {
                np.replace('*', "%")
            } else {
                format!("%{}%", np)
            };
            where_clauses.push("(n.name LIKE ? OR n.qualified_name LIKE ?)".to_string());
            params.push(Box::new(like_str.clone()));
            params.push(Box::new(like_str));
        }

        if let Some(fp) = file_pattern.filter(|s| !s.trim().is_empty()) {
            let like_str = if fp.contains('%') || fp.contains('*') {
                fp.replace('*', "%")
            } else {
                format!("%{}%", fp)
            };
            where_clauses.push("n.file_path LIKE ?".to_string());
            params.push(Box::new(like_str));
        }

        if let Some(min_d) = min_degree {
            where_clauses.push("(COALESCE(in_e.cnt, 0) + COALESCE(out_e.cnt, 0)) >= ?".to_string());
            params.push(Box::new(min_d as i64));
        }

        if let Some(max_d) = max_degree {
            where_clauses.push("(COALESCE(in_e.cnt, 0) + COALESCE(out_e.cnt, 0)) <= ?".to_string());
            params.push(Box::new(max_d as i64));
        }

        let sql = format!(
            "SELECT \
                n.id, \
                n.name, \
                n.label, \
                n.qualified_name, \
                n.file_path, \
                COALESCE(n.start_line, 1), \
                COALESCE(in_e.cnt, 0) AS in_degree, \
                COALESCE(out_e.cnt, 0) AS out_degree, \
                (COALESCE(in_e.cnt, 0) + COALESCE(out_e.cnt, 0)) AS total_degree \
             FROM nodes n \
             LEFT JOIN (SELECT target_id, COUNT(*) AS cnt FROM edges GROUP BY target_id) in_e ON in_e.target_id = n.id \
             LEFT JOIN (SELECT source_id, COUNT(*) AS cnt FROM edges GROUP BY source_id) out_e ON out_e.source_id = n.id \
             WHERE {} \
             ORDER BY total_degree DESC, n.id ASC \
             LIMIT ? OFFSET ?",
            where_clauses.join(" AND ")
        );

        params.push(Box::new(limit.clamp(1, 200) as i64));
        params.push(Box::new(offset as i64));

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rusqlite_params: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(rusqlite::params_from_iter(rusqlite_params), |row| {
                Ok(GraphSearchResultItem {
                    id: row.get::<_, i64>(0)?.to_string(),
                    name: row.get::<_, String>(1)?,
                    label: row.get::<_, String>(2)?,
                    qualified_name: row.get::<_, String>(3)?,
                    file_path: row.get::<_, String>(4)?,
                    start_line: row.get::<_, Option<i64>>(5)?.unwrap_or(1) as usize,
                    in_degree: row.get::<_, i64>(6)?.max(0) as usize,
                    out_degree: row.get::<_, i64>(7)?.max(0) as usize,
                    total_degree: row.get::<_, i64>(8)?.max(0) as usize,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// 在已索引文件范围内进行源码文本搜索
    pub fn search_code(
        &self,
        query: &str,
        file_pattern: Option<&str>,
        project_root: Option<&Path>,
        limit: usize,
    ) -> Result<Vec<CodeSearchMatch>, String> {
        let conn = self.open_ro()?;
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = "SELECT DISTINCT file_path FROM nodes WHERE label = 'File' AND file_path <> ''".to_string();
        let mut params = Vec::new();

        if let Some(fp) = file_pattern.filter(|s| !s.trim().is_empty()) {
            let like_str = if fp.contains('%') || fp.contains('*') {
                fp.replace('*', "%")
            } else {
                format!("%{}%", fp)
            };
            sql.push_str(" AND file_path LIKE ?1");
            params.push(like_str);
        }

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let files: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let max_matches = limit.clamp(1, 100);
        let mut matches = Vec::new();
        let query_lower = trimmed_query.to_lowercase();

        if let Some(root) = project_root {
            for rel_path in files {
                if matches.len() >= max_matches {
                    break;
                }
                let full_path = root.join(&rel_path);
                if let Ok(content) = std::fs::read_to_string(&full_path) {
                    for (line_idx, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(&query_lower) {
                            matches.push(CodeSearchMatch {
                                file_path: rel_path.clone(),
                                line_number: line_idx + 1,
                                line_content: line.trim().to_string(),
                                surrounding_symbol: None,
                            });
                            if matches.len() >= max_matches {
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            let like_query = format!("%{}%", trimmed_query);
            let mut node_stmt = conn
                .prepare(
                    "SELECT file_path, start_line, name, label FROM nodes \
                     WHERE (name LIKE ?1 OR qualified_name LIKE ?1) AND file_path <> '' \
                     LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;
            let fallback_rows = node_stmt
                .query_map(rusqlite::params![like_query, max_matches as i64], |row| {
                    Ok(CodeSearchMatch {
                        file_path: row.get::<_, String>(0)?,
                        line_number: row.get::<_, Option<i64>>(1)?.unwrap_or(1) as usize,
                        line_content: format!("{}: {}", row.get::<_, String>(3)?, row.get::<_, String>(2)?),
                        surrounding_symbol: Some(row.get::<_, String>(2)?),
                    })
                })
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            matches = fallback_rows;
        }

        Ok(matches)
    }

    pub fn open_ro(&self) -> Result<Connection, String> {
        if !self.is_indexed() {
            return Err("Graph database not found (not indexed)".to_string());
        }
        let conn = Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Failed to open graph DB: {}", e))?;

        // Basic schema validation to ensure the DB is a valid codebase-memory graph
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('nodes', 'edges')",
            )
            .map_err(|e| format!("Schema validation failed: {}", e))?;

        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Schema validation failed: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        if tables.len() != 2 {
            return Err("Graph database schema is invalid or missing required tables".to_string());
        }

        drop(stmt);
        Ok(conn)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceHop {
    pub from_id: String,
    pub from_name: String,
    pub to_id: String,
    pub to_name: String,
    pub edge_type: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracePathResult {
    pub root_node: crate::models::graph::GraphNode,
    pub direction: String,
    pub depth: usize,
    pub hops: Vec<TraceHop>,
    pub total_hops: usize,
    pub reachable_nodes: Vec<crate::models::graph::GraphNode>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSchema {
    pub node_labels: std::collections::BTreeMap<String, usize>,
    pub edge_types: std::collections::BTreeMap<String, usize>,
    pub total_nodes: usize,
    pub total_edges: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSnippetResult {
    pub qualified_name: String,
    pub name: String,
    pub label: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub neighbors: Vec<crate::models::graph::GraphNode>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadCodeSymbol {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub label: String,
    pub file_path: String,
    pub start_line: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSearchResultItem {
    pub id: String,
    pub name: String,
    pub label: String,
    pub qualified_name: String,
    pub file_path: String,
    pub start_line: usize,
    pub in_degree: usize,
    pub out_degree: usize,
    pub total_degree: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSearchMatch {
    pub file_path: String,
    pub line_number: usize,
    pub line_content: String,
    pub surrounding_symbol: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_intelligence_extended_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("graph.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                qualified_name TEXT NOT NULL,
                label TEXT NOT NULL,
                file_path TEXT NOT NULL,
                start_line INTEGER,
                end_line INTEGER,
                properties TEXT
            );
            CREATE TABLE edges (
                id INTEGER PRIMARY KEY,
                source_id INTEGER NOT NULL,
                target_id INTEGER NOT NULL,
                type TEXT NOT NULL,
                properties TEXT
            );
            INSERT INTO nodes VALUES (1, 'handle_request', 'api::handle_request', 'Function', 'src/api.rs', 10, 20, '{}');
            INSERT INTO nodes VALUES (2, 'process_order', 'services::process_order', 'Function', 'src/service.rs', 15, 30, '{}');
            INSERT INTO nodes VALUES (3, 'save_db', 'dao::save_db', 'Function', 'src/dao.rs', 5, 12, '{}');
            INSERT INTO nodes VALUES (4, 'unused_helper', 'utils::unused_helper', 'Function', 'src/utils.rs', 1, 5, '{}');

            INSERT INTO edges VALUES (1, 1, 2, 'CALLS', '{}');
            INSERT INTO edges VALUES (2, 2, 3, 'CALLS', '{}');
            "#,
        )
        .unwrap();
        drop(conn);

        let rt = CodeIntelligenceRuntime::new(&db_path);

        // 1. Trace Path
        let trace = rt.trace_path("handle_request", "outbound", 3, "calls").unwrap();
        assert_eq!(trace.total_hops, 2);
        assert_eq!(trace.reachable_nodes.len(), 2);

        // 2. Schema
        let schema = rt.get_graph_schema().unwrap();
        assert_eq!(schema.total_nodes, 4);
        assert_eq!(schema.total_edges, 2);
        assert_eq!(schema.node_labels.get("Function"), Some(&4));
        assert_eq!(schema.edge_types.get("CALLS"), Some(&2));

        // 3. Dead code
        let dead = rt.get_dead_code(10).unwrap();
        let dead_names: Vec<&str> = dead.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(dead.len(), 2);
        assert!(dead_names.contains(&"unused_helper"));
        assert!(dead_names.contains(&"handle_request"));

        // 4. Search Graph with min_degree
        let search_res = rt.search_graph(Some("Function"), Some("process"), None, Some(1), None, 10, 0).unwrap();
        assert_eq!(search_res.len(), 1);
        assert_eq!(search_res[0].name, "process_order");
        assert_eq!(search_res[0].total_degree, 2); // 1 in (from handle_request) + 1 out (to save_db)

        // 5. Search Code fallback
        let code_res = rt.search_code("order", None, None, 10).unwrap();
        assert_eq!(code_res.len(), 1);
        assert_eq!(code_res[0].file_path, "src/service.rs");
    }
}
