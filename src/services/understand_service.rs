//! Understand-Anything style comprehension over the codebase-memory-mcp graph.
//!
//! File level: symbols defined in a file, files it depends on, files depending on it.
//! Project level: summary, totals, label/edge distributions, languages, modules,
//! and the most referenced symbols.

use std::collections::BTreeMap;
use std::path::Path;
use rusqlite::Connection;
use serde_json::{json, Value};
use crate::services::code_intelligence::CodeIntelligenceRuntime;

/// Edge types that represent a real dependency between files.
const DEP_EDGE_TYPES: &str = "'CALLS','IMPORTS','USAGE','HTTP_CALLS','WRITES','EXTENDS','IMPLEMENTS'";

/// File-level understanding: structure + outgoing dependencies + inbound references.
pub fn understand_file(db_path: &str, rel_path: &str) -> Result<Value, String> {
    let rel_path = rel_path.replace('\\', "/");
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(json!({ "file": rel_path, "indexed": false }));
    }
    let conn = rt.open_ro()?;

    // Symbols defined in this file, ordered by position.
    let mut stmt = conn
        .prepare(
            "SELECT id, name, label, qualified_name, start_line, end_line \
             FROM nodes WHERE file_path = ?1 AND label NOT IN ('File','Module') \
             ORDER BY start_line LIMIT 500",
        )
        .map_err(|e| e.to_string())?;
    let symbols: Vec<Value> = stmt
        .query_map(rusqlite::params![rel_path], |row| {
            Ok(json!({
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

    let dependencies = file_relations(&conn, &rel_path, true)?;
    let dependents = file_relations(&conn, &rel_path, false)?;

    // Label distribution for a quick shape-of-file overview.
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
fn file_relations(conn: &Connection, rel_path: &str, outgoing: bool) -> Result<Vec<Value>, String> {
    let (from_col, to_col) = if outgoing { ("sn", "tn") } else { ("tn", "sn") };
    let sql = format!(
        "SELECT {to}.file_path, e.type, COUNT(*) AS cnt \
         FROM edges e \
         JOIN nodes sn ON sn.id = e.source_id \
         JOIN nodes tn ON tn.id = e.target_id \
         WHERE {from}.file_path = ?1 AND {to}.file_path <> '' AND {to}.file_path <> ?1 \
           AND {to}.file_path NOT LIKE '%/assets/%' AND {to}.file_path NOT LIKE '%.min.js' \
           AND e.type IN ({types}) \
         GROUP BY {to}.file_path, e.type",
        from = from_col, to = to_col, types = DEP_EDGE_TYPES,
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
    let mut by_file: BTreeMap<String, (i64, BTreeMap<String, i64>)> = BTreeMap::new();
    for (file, edge_type, count) in rows {
        let entry = by_file.entry(file).or_default();
        entry.0 += count;
        *entry.1.entry(edge_type).or_insert(0) += count;
    }
    let mut relations: Vec<(String, i64, BTreeMap<String, i64>)> = by_file
        .into_iter()
        .map(|(file, (total, kinds))| (file, total, kinds))
        .collect();
    relations.sort_by(|a, b| b.1.cmp(&a.1));
    relations.truncate(50);

    Ok(relations
        .into_iter()
        .map(|(file, total, kinds)| json!({ "file": file, "references": total, "edgeTypes": kinds }))
        .collect())
}

/// Project-level understanding: totals, distributions, modules, hot symbols.
pub fn understand_project(db_path: &str) -> Result<Value, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(json!({ "indexed": false }));
    }
    let conn = rt.open_ro()?;

    let total_nodes: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap_or(0);
    let total_edges: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap_or(0);
    let total_files: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes WHERE label = 'File'", [], |r| r.get(0))
        .unwrap_or(0);
    let summary: Option<String> = conn
        .query_row("SELECT summary FROM project_summaries LIMIT 1", [], |r| r.get(0))
        .ok();

    let node_labels = count_rows(&conn, "SELECT label, COUNT(*) FROM nodes GROUP BY label ORDER BY 2 DESC LIMIT 20")?;
    let edge_types = count_rows(&conn, "SELECT type, COUNT(*) FROM edges GROUP BY type ORDER BY 2 DESC LIMIT 20")?;
    // File nodes carry `extension` (not `language`) in properties; map it here.
    let languages = language_breakdown(&conn)?;
    // Top-level directory breakdown as a coarse module map.
    let modules = count_rows(
        &conn,
        "SELECT CASE WHEN instr(file_path, '/') > 0 \
                     THEN substr(file_path, 1, instr(file_path, '/') - 1) \
                     ELSE file_path END, COUNT(*) \
         FROM nodes WHERE label = 'File' AND file_path <> '' \
         GROUP BY 1 ORDER BY 2 DESC LIMIT 20",
    )?;

    // Most referenced symbols across the graph (incoming CALLS/USAGE edges).
    // Minified bundle output is excluded: single-letter names and vendored asset
    // files otherwise dominate the ranking through name-collision edges.
    let mut stmt = conn
        .prepare(
            "SELECT n.name, n.label, n.file_path, COUNT(*) AS refs \
             FROM edges e JOIN nodes n ON n.id = e.target_id \
             WHERE e.type IN ('CALLS','USAGE') AND n.label IN ('Function','Method','Class') \
               AND LENGTH(n.name) >= 3 \
               AND n.file_path NOT LIKE '%/assets/%' AND n.file_path NOT LIKE '%.min.js' \
             GROUP BY e.target_id ORDER BY refs DESC LIMIT 20",
        )
        .map_err(|e| e.to_string())?;
    let top_symbols: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "name": row.get::<_, String>(0)?,
                "label": row.get::<_, String>(1)?,
                "filePath": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                "references": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(json!({
        "indexed": true,
        "summary": summary,
        "totals": { "nodes": total_nodes, "edges": total_edges, "files": total_files },
        "nodeLabels": node_labels,
        "edgeTypes": edge_types,
        "languages": languages,
        "modules": modules,
        "topSymbols": top_symbols,
    }))
}

/// Language distribution derived from File-node `extension` properties.
fn language_breakdown(conn: &Connection) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT COALESCE(json_extract(properties, '$.extension'), ''), COUNT(*) \
             FROM nodes WHERE label = 'File' GROUP BY 1 ORDER BY 2 DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut by_lang: BTreeMap<&'static str, i64> = BTreeMap::new();
    for (ext, count) in &rows {
        *by_lang.entry(extension_to_language(ext)).or_insert(0) += count;
    }
    let mut langs: Vec<(&str, i64)> = by_lang.into_iter().collect();
    langs.sort_by(|a, b| b.1.cmp(&a.1));
    langs.truncate(20);
    Ok(langs
        .into_iter()
        .map(|(name, count)| json!({ "name": name, "count": count }))
        .collect())
}

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
        let parts: Vec<String> = by_label.iter().map(|(k, val)| format!("{}={}", k, val)).collect();
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
