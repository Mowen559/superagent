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
