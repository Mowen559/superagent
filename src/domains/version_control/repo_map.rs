//! Aider-style Repository Map & PageRank Engine
//! Generates a concise, high-density structural skeleton of the entire codebase
//! ranked by symbol connectivity and relevance, keeping token usage under ~1,500 tokens.

use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RankedSymbol {
    pub name: String,
    pub label: String,
    pub file_path: String,
    pub start_line: i64,
    pub rank_score: f64,
}

pub struct RepoMapEngine {
    db_path: PathBuf,
}

impl RepoMapEngine {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    /// Run PageRank on symbol graph and build an Aider-style compressed Repo Map tree
    pub fn generate_map(
        &self,
        focus_path: Option<&str>,
        query: Option<&str>,
        max_symbols: usize,
    ) -> Result<String, String> {
        if !self.db_path.exists() {
            return Ok("# Repository Map\nNo codebase memory database found. Please run indexer or inspect workspace files directly.".to_string());
        }

        let conn = Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Failed to open codebase memory db: {}", e))?;

        // 1. Fetch nodes (skip generic file/module nodes)
        let mut node_stmt = conn
            .prepare(
                "SELECT id, name, label, file_path, COALESCE(start_line, 0) \
                 FROM nodes \
                 WHERE label NOT IN ('File', 'Module', 'Directory') \
                   AND file_path IS NOT NULL AND file_path != '' \
                   AND file_path NOT LIKE '%/target/%' AND file_path NOT LIKE '%/node_modules/%' \
                 LIMIT 5000",
            )
            .map_err(|e| e.to_string())?;

        let mut node_map: HashMap<i64, (String, String, String, i64)> = HashMap::new();
        let rows = node_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        for r in rows.flatten() {
            node_map.insert(r.0, (r.1, r.2, r.3, r.4));
        }

        if node_map.is_empty() {
            return Ok("# Repository Map\nWorkspace symbol index is empty.".to_string());
        }

        // 2. Fetch edges for PageRank connectivity
        let mut edge_stmt = conn
            .prepare("SELECT source_id, target_id FROM edges LIMIT 20000")
            .map_err(|e| e.to_string())?;

        let mut out_edges: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut in_degree: HashMap<i64, usize> = HashMap::new();

        let edge_rows = edge_stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;

        for r in edge_rows.flatten() {
            if node_map.contains_key(&r.0) && node_map.contains_key(&r.1) {
                out_edges.entry(r.0).or_default().push(r.1);
                *in_degree.entry(r.1).or_default() += 1;
            }
        }

        // 3. Compute PageRank
        let total_nodes = node_map.len() as f64;
        let damping = 0.85;
        let mut pr_scores: HashMap<i64, f64> = HashMap::new();
        for &id in node_map.keys() {
            pr_scores.insert(id, 1.0 / total_nodes);
        }

        for _ in 0..10 {
            let mut next_pr = HashMap::new();
            for (&node_id, _) in &node_map {
                let mut incoming_sum = 0.0;
                // Accumulate from callers
                for (&src, targets) in &out_edges {
                    if targets.contains(&node_id) {
                        let src_pr = pr_scores.get(&src).copied().unwrap_or(0.0);
                        incoming_sum += src_pr / (targets.len() as f64);
                    }
                }
                let score = ((1.0 - damping) / total_nodes) + (damping * incoming_sum);
                next_pr.insert(node_id, score);
            }
            pr_scores = next_pr;
        }

        // 4. Boost by focus_path or query match
        let focus_norm = focus_path.map(|p| p.to_lowercase().replace('\\', "/"));
        let query_words: Vec<String> = query
            .unwrap_or("")
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() >= 2)
            .collect();

        let mut ranked = Vec::new();
        for (id, (name, label, file_path, start_line)) in node_map {
            let mut score = pr_scores.get(&id).copied().unwrap_or(0.0);

            let fp_norm = file_path.to_lowercase().replace('\\', "/");
            if let Some(ref fp) = focus_norm {
                if fp_norm.contains(fp) {
                    score *= 3.0;
                }
            }

            let name_norm = name.to_lowercase();
            for word in &query_words {
                if name_norm.contains(word) || fp_norm.contains(word) {
                    score *= 2.5;
                }
            }

            ranked.push(RankedSymbol {
                name,
                label,
                file_path,
                start_line,
                rank_score: score,
            });
        }

        ranked.sort_by(|a, b| b.rank_score.partial_cmp(&a.rank_score).unwrap_or(std::cmp::Ordering::Equal));
        let selected: Vec<RankedSymbol> = ranked.into_iter().take(max_symbols).collect();

        // 5. Group by file and format into an Aider-style Tree Map
        let mut by_file: BTreeMap<String, Vec<RankedSymbol>> = BTreeMap::new();
        for sym in selected {
            by_file.entry(sym.file_path.clone()).or_default().push(sym);
        }

        let mut out = String::from("# Repository Map (Ranked Semantic Skeleton)\n");
        for (file, mut syms) in by_file {
            syms.sort_by_key(|s| s.start_line);
            out.push_str(&format!("📄 `{}`:\n", file));
            for sym in syms {
                out.push_str(&format!("  │  L{}: {} `{}`\n", sym.start_line, sym.label, sym.name));
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_repo_map_empty_and_populated() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_codebase.db");
        let engine = RepoMapEngine::new(db_path.clone());

        // Initially no db
        let res_no_db = engine.generate_map(None, None, 10).unwrap();
        assert!(res_no_db.contains("No codebase memory database found"));

        // Create db and populate nodes
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(r#"
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                label TEXT NOT NULL,
                file_path TEXT,
                start_line INTEGER
            );
            CREATE TABLE edges (
                id INTEGER PRIMARY KEY,
                source_id INTEGER NOT NULL,
                target_id INTEGER NOT NULL,
                type TEXT NOT NULL
            );
            INSERT INTO nodes VALUES (1, 'UserController', 'Class', 'src/api/UserController.java', 20);
            INSERT INTO nodes VALUES (2, 'UserService', 'Interface', 'src/service/UserService.java', 10);
            INSERT INTO nodes VALUES (3, 'getUserById', 'Method', 'src/api/UserController.java', 35);
            INSERT INTO edges VALUES (1, 1, 2, 'CALLS');
        "#).unwrap();

        let res = engine.generate_map(None, Some("User"), 10).unwrap();
        assert!(res.contains("Repository Map"));
        assert!(res.contains("UserController.java"));
        assert!(res.contains("UserService.java"));
        assert!(res.contains("UserController"));
    }
}
