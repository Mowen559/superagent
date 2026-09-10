//! Understand-Anything style comprehension over the codebase-memory-mcp graph.
//!
//! File level: symbols defined in a file, files it depends on, files depending on it.
//! Project level: summary, totals, label/edge distributions, languages, modules,
//! and the most referenced symbols.

use crate::services::code_intelligence::CodeIntelligenceRuntime;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Edge types that represent a real dependency between files.
const DEP_EDGE_TYPES: &str =
    "'CALLS','IMPORTS','USAGE','HTTP_CALLS','WRITES','EXTENDS','IMPLEMENTS'";

/// File-level understanding: structure + outgoing dependencies + inbound references.
pub fn understand_file(db_path: &str, rel_path: &str) -> Result<Value, String> {
    let rel_path = rel_path.replace('\\', "/");
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(json!({ "file": rel_path, "indexed": false }));
    }

    let symbols = rt.get_file_symbols(&rel_path).unwrap_or_default();
    let dependencies = rt.file_relations(&rel_path, true).unwrap_or_default();
    let dependents = rt.file_relations(&rel_path, false).unwrap_or_default();

    let mut label_counts: BTreeMap<String, i64> = BTreeMap::new();
    for s in &symbols {
        if let Some(l) = s.get("label").and_then(|v| v.as_str()) {
            *label_counts.entry(l.to_string()).or_insert(0) += 1;
        }
    }

    Ok(json!({
        "file": rel_path,
        "indexed": !symbols.is_empty() || !dependencies.is_empty() || !dependents.is_empty(),
        "symbolCount": symbols.len(),
        "symbolsByLabel": label_counts,
        "symbols": symbols,
        "dependencies": dependencies,
        "dependents": dependents,
    }))
}

/// Aggregate cross-file edges for one file. `outgoing` picks the direction:
/// true = files this file depends on, false = files that reference this file.

/// Returns the most prominent programming language detected in the graph database.
pub fn detect_primary_language(db_path: &str) -> Result<String, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok("unknown".to_string());
    }
    let langs = rt.language_breakdown().unwrap_or_default();
    if let Some(first) = langs.first() {
        if let Some(name) = first.get("name").and_then(|n| n.as_str()) {
            return Ok(name.to_string());
        }
    }
    Ok("unknown".to_string())
}

/// Project-level understanding: totals, distributions, modules, hot symbols.
pub fn understand_project(db_path: &str) -> Result<Value, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(json!({ "indexed": false }));
    }

    let summary = rt.get_project_summary().unwrap_or(None);
    let files = rt.get_file_summaries(500).unwrap_or_default();
    let stats = rt.get_project_stats().unwrap_or_default();

    let (module_deps, connected_files, health) = if let Ok(conn) = rt.open_ro() {
        let md = cross_module_dependencies(&conn).unwrap_or_default();
        let cf = most_connected_files(&conn).unwrap_or_default();
        let gh = graph_health(&conn).unwrap_or_else(|_| json!({
            "isolatedFiles": 0,
            "selfEdges": 0,
            "danglingEdges": 0,
        }));
        (md, cf, gh)
    } else {
        (
            Vec::new(),
            Vec::new(),
            json!({
                "isolatedFiles": 0,
                "selfEdges": 0,
                "danglingEdges": 0,
            }),
        )
    };

    let total_nodes = stats.get("totalNodes").cloned().unwrap_or(json!(0));
    let total_edges = stats.get("totalEdges").cloned().unwrap_or(json!(0));
    let total_files = stats.get("totalFiles").cloned().unwrap_or(json!(0));

    Ok(json!({
        "indexed": true,
        "summary": summary,
        "totals": {
            "nodes": total_nodes,
            "edges": total_edges,
            "files": total_files,
        },
        "totalNodes": total_nodes,
        "totalEdges": total_edges,
        "totalFiles": total_files,
        "labelDistribution": stats.get("labelDistribution").unwrap_or(&json!({})),
        "edgeDistribution": stats.get("edgeDistribution").unwrap_or(&json!({})),
        "primaryLanguage": detect_primary_language(db_path).unwrap_or_default(),
        "files": files,
        "hotSymbols": rt.get_hot_symbols(50).unwrap_or_default(),
        "modules": rt.get_modules(500).unwrap_or_default(),
        "moduleDependencies": module_deps,
        "connectedFiles": connected_files,
        "graphHealth": health,
    }))
}

/// Aggregate real dependency edges across top-level modules. This is more
/// useful for architecture comprehension than a directory/file count alone.
fn cross_module_dependencies(conn: &Connection) -> Result<Vec<Value>, String> {
    let sql = format!(
        "SELECT \
           CASE WHEN instr(sn.file_path, '/') > 0 THEN substr(sn.file_path, 1, instr(sn.file_path, '/') - 1) ELSE 'root' END AS source_module, \
           CASE WHEN instr(tn.file_path, '/') > 0 THEN substr(tn.file_path, 1, instr(tn.file_path, '/') - 1) ELSE 'root' END AS target_module, \
           COUNT(*) AS refs, COUNT(DISTINCT sn.file_path || '->' || tn.file_path) AS file_pairs \
         FROM edges e \
         JOIN nodes sn ON sn.id = e.source_id \
         JOIN nodes tn ON tn.id = e.target_id \
         WHERE e.type IN ({}) AND sn.file_path <> '' AND tn.file_path <> '' \
           AND sn.file_path <> tn.file_path \
         GROUP BY source_module, target_module HAVING source_module <> target_module \
         ORDER BY refs DESC LIMIT 40",
        DEP_EDGE_TYPES
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "source": row.get::<_, String>(0)?,
                "target": row.get::<_, String>(1)?,
                "references": row.get::<_, i64>(2)?,
                "filePairs": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

/// Rank files by distinct inbound/outbound neighbours. The ranking highlights
/// seams and potential change-risk hotspots without pretending complexity can
/// be inferred from line count alone.
fn most_connected_files(conn: &Connection) -> Result<Vec<Value>, String> {
    let sql = format!(
        "WITH links AS ( \
           SELECT sn.file_path AS source_file, tn.file_path AS target_file \
           FROM edges e JOIN nodes sn ON sn.id = e.source_id JOIN nodes tn ON tn.id = e.target_id \
           WHERE e.type IN ({}) AND sn.file_path <> '' AND tn.file_path <> '' AND sn.file_path <> tn.file_path \
         ), totals AS ( \
           SELECT source_file AS file_path, COUNT(DISTINCT target_file) AS outbound, 0 AS inbound, COUNT(*) AS refs FROM links GROUP BY source_file \
           UNION ALL \
           SELECT target_file AS file_path, 0 AS outbound, COUNT(DISTINCT source_file) AS inbound, COUNT(*) AS refs FROM links GROUP BY target_file \
         ) \
         SELECT file_path, SUM(outbound), SUM(inbound), SUM(refs) \
         FROM totals GROUP BY file_path \
         ORDER BY (SUM(outbound) + SUM(inbound)) DESC, SUM(refs) DESC LIMIT 30",
        DEP_EDGE_TYPES
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "file": row.get::<_, String>(0)?,
                "outboundFiles": row.get::<_, i64>(1)?,
                "inboundFiles": row.get::<_, i64>(2)?,
                "references": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

fn graph_health(conn: &Connection) -> Result<Value, String> {
    let isolated_files: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes f WHERE f.label = 'File' AND NOT EXISTS ( \
               SELECT 1 FROM edges e JOIN nodes sn ON sn.id = e.source_id JOIN nodes tn ON tn.id = e.target_id \
               WHERE e.type IN ('CALLS','IMPORTS','USAGE','HTTP_CALLS','WRITES','EXTENDS','IMPLEMENTS') \
                 AND (sn.file_path = f.file_path OR tn.file_path = f.file_path) \
                 AND sn.file_path <> tn.file_path)",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let self_edges: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE source_id = target_id",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let dangling_edges: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges e LEFT JOIN nodes sn ON sn.id = e.source_id \
             LEFT JOIN nodes tn ON tn.id = e.target_id WHERE sn.id IS NULL OR tn.id IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "isolatedFiles": isolated_files,
        "selfEdges": self_edges,
        "danglingEdges": dangling_edges,
    }))
}

/// Language distribution derived from File-node `extension` properties.

fn extension_to_language(ext: &str) -> &'static str {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "rs" => "Rust",
        "go" => "Go",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" => "C++",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "py" => "Python",
        "js" | "mjs" | "cjs" => "JavaScript",
        "jsx" => "JSX",
        "ts" | "mts" | "cts" => "TypeScript",
        "tsx" => "TSX",
        "vue" => "Vue",
        "css" | "scss" | "less" => "CSS",
        "html" | "htm" => "HTML",
        "json" => "JSON",
        "yml" | "yaml" => "YAML",
        "toml" => "TOML",
        "xml" => "XML",
        "md" | "mdx" => "Markdown",
        "sql" => "SQL",
        "sh" | "bash" => "Shell",
        "ps1" | "psm1" => "PowerShell",
        "bat" | "cmd" => "Batch",
        "proto" => "Protobuf",
        "gradle" => "Gradle",
        "" => "unknown",
        _ => "other",
    }
}

/// Run a `(text, count)` aggregation query into a JSON object list.
fn count_rows(conn: &Connection, sql: &str) -> Result<Vec<Value>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "name": row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".into()),
                "count": row.get::<_, i64>(1)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Flatten the file-level structural JSON into a compact, token-frugal text
/// block for the LLM prompt. Shared by the understand route and any
/// post-index automation.
pub fn summarize_file_structure(v: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();

    if let Some(n) = v.get("symbolCount").and_then(|x| x.as_i64()) {
        lines.push(format!("符号数量: {}", n));
    }
    if let Some(by_label) = v.get("symbolsByLabel").and_then(|x| x.as_object()) {
        let parts: Vec<String> = by_label
            .iter()
            .map(|(k, val)| format!("{}={}", k, val))
            .collect();
        if !parts.is_empty() {
            lines.push(format!("符号构成: {}", parts.join(", ")));
        }
    }
    if let Some(symbols) = v.get("symbols").and_then(|x| x.as_array()) {
        let names: Vec<String> = symbols
            .iter()
            .take(30)
            .filter_map(|s| {
                let name = s.get("name")?.as_str()?;
                let label = s.get("label").and_then(|l| l.as_str()).unwrap_or("");
                Some(format!("{}({})", name, label))
            })
            .collect();
        if !names.is_empty() {
            lines.push(format!("主要符号: {}", names.join(", ")));
        }
    }
    if let Some(deps) = v.get("dependencies").and_then(|x| x.as_array()) {
        let files: Vec<String> = deps
            .iter()
            .take(15)
            .filter_map(|d| d.get("file")?.as_str().map(|s| s.to_string()))
            .collect();
        if !files.is_empty() {
            lines.push(format!("依赖的文件: {}", files.join(", ")));
        }
    }
    if let Some(deps) = v.get("dependents").and_then(|x| x.as_array()) {
        let files: Vec<String> = deps
            .iter()
            .take(15)
            .filter_map(|d| d.get("file")?.as_str().map(|s| s.to_string()))
            .collect();
        if !files.is_empty() {
            lines.push(format!("被引用的文件: {}", files.join(", ")));
        }
    }

    lines.join("\n")
}

/// Flatten the project-level structural JSON into a compact text block.
pub fn summarize_project_structure(v: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();

    if let Some(t) = v.get("totals") {
        lines.push(format!(
            "规模: nodes={}, edges={}, files={}",
            t.get("nodes").and_then(|x| x.as_i64()).unwrap_or(0),
            t.get("edges").and_then(|x| x.as_i64()).unwrap_or(0),
            t.get("files").and_then(|x| x.as_i64()).unwrap_or(0),
        ));
    }
    if let Some(existing) = v.get("summary").and_then(|x| x.as_str()) {
        if !existing.trim().is_empty() {
            lines.push(format!("已有摘要: {}", existing));
        }
    }
    if let Some(langs) = v.get("languages").and_then(|x| x.as_array()) {
        let parts: Vec<String> = langs
            .iter()
            .take(12)
            .filter_map(|l| {
                let name = l.get("name")?.as_str()?;
                let count = l.get("count")?.as_i64()?;
                Some(format!("{}={}", name, count))
            })
            .collect();
        if !parts.is_empty() {
            lines.push(format!("语言分布: {}", parts.join(", ")));
        }
    }
    if let Some(mods) = v.get("modules").and_then(|x| x.as_array()) {
        let parts: Vec<String> = mods
            .iter()
            .take(15)
            .filter_map(|m| {
                let name = m.get("name")?.as_str()?;
                let count = m.get("count")?.as_i64()?;
                Some(format!("{}({})", name, count))
            })
            .collect();
        if !parts.is_empty() {
            lines.push(format!("顶层模块(文件数): {}", parts.join(", ")));
        }
    }
    if let Some(dependencies) = v.get("moduleDependencies").and_then(|x| x.as_array()) {
        let parts: Vec<String> = dependencies
            .iter()
            .take(16)
            .filter_map(|dependency| {
                Some(format!(
                    "{}→{}({})",
                    dependency.get("source")?.as_str()?,
                    dependency.get("target")?.as_str()?,
                    dependency.get("references")?.as_i64()?
                ))
            })
            .collect();
        if !parts.is_empty() {
            lines.push(format!("跨模块依赖: {}", parts.join(", ")));
        }
    }
    if let Some(files) = v.get("connectedFiles").and_then(|x| x.as_array()) {
        let parts: Vec<String> = files
            .iter()
            .take(12)
            .filter_map(|file| {
                Some(format!(
                    "{}(in={},out={})",
                    file.get("file")?.as_str()?,
                    file.get("inboundFiles")?.as_i64()?,
                    file.get("outboundFiles")?.as_i64()?
                ))
            })
            .collect();
        if !parts.is_empty() {
            lines.push(format!("高连接文件: {}", parts.join(", ")));
        }
    }
    if let Some(health) = v.get("graphHealth") {
        lines.push(format!(
            "图谱健康: isolatedFiles={}, selfEdges={}, danglingEdges={}",
            health
                .get("isolatedFiles")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
            health
                .get("selfEdges")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
            health
                .get("danglingEdges")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
        ));
    }
    if let Some(syms) = v.get("topSymbols").and_then(|x| x.as_array()) {
        let parts: Vec<String> = syms
            .iter()
            .take(15)
            .filter_map(|s| {
                let name = s.get("name")?.as_str()?;
                let refs = s.get("references")?.as_i64()?;
                Some(format!("{}({}refs)", name, refs))
            })
            .collect();
        if !parts.is_empty() {
            lines.push(format!("高频符号: {}", parts.join(", ")));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_understanding_exposes_architecture_dependencies_and_graph_health() {
        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("graph.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                project TEXT,
                label TEXT NOT NULL,
                name TEXT NOT NULL,
                qualified_name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                start_line INTEGER,
                end_line INTEGER,
                properties TEXT
            );
            CREATE TABLE edges (
                id INTEGER PRIMARY KEY,
                project TEXT,
                source_id INTEGER,
                target_id INTEGER,
                type TEXT NOT NULL,
                properties TEXT
            );
            CREATE TABLE project_summaries (summary TEXT);
            INSERT INTO project_summaries VALUES ('fixture');
            INSERT INTO nodes VALUES (1, 'p', 'File', 'a.rs', 'a.rs', 'src/a.rs', 1, 10, '{"extension":".rs"}');
            INSERT INTO nodes VALUES (2, 'p', 'File', 'b.rs', 'b.rs', 'api/b.rs', 1, 10, '{"extension":".rs"}');
            INSERT INTO nodes VALUES (3, 'p', 'Function', 'alpha', 'alpha', 'src/a.rs', 2, 4, '{}');
            INSERT INTO nodes VALUES (4, 'p', 'Function', 'beta', 'beta', 'api/b.rs', 2, 4, '{}');
            INSERT INTO edges VALUES (1, 'p', 3, 4, 'CALLS', '{}');
            "#,
        )
        .unwrap();
        drop(conn);

        let result = understand_project(db_path.to_str().unwrap()).unwrap();
        assert_eq!(
            result.pointer("/totals/files").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            result
                .pointer("/moduleDependencies/0/source")
                .and_then(Value::as_str),
            Some("src")
        );
        assert_eq!(
            result
                .pointer("/moduleDependencies/0/target")
                .and_then(Value::as_str),
            Some("api")
        );
        assert!(result
            .get("connectedFiles")
            .and_then(Value::as_array)
            .is_some_and(|files| files.len() == 2));
        assert_eq!(
            result
                .pointer("/graphHealth/danglingEdges")
                .and_then(Value::as_i64),
            Some(0)
        );
    }
}
