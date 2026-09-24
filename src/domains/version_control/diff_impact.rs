use std::collections::{HashSet, HashMap};
use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::domains::code_analysis::code_intelligence::CodeIntelligenceRuntime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeImpactReport {
    pub changed_files: Vec<String>,
    pub directly_affected_symbols: Vec<ChangedSymbolImpact>,
    pub downstream_affected_symbols: Vec<DownstreamImpact>,
    pub total_impact_count: usize,
    pub risk_level: String,
    pub risk_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedSymbolImpact {
    pub symbol_name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub callers_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownstreamImpact {
    pub symbol_name: String,
    pub file_path: String,
    pub via_call_chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolImpactRadar {
    pub symbol: String,
    pub total_callers: usize,
    pub callers: Vec<SymbolCallerInfo>,
    pub affected_files: Vec<String>,
    pub risk_level: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolCallerInfo {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub snippet: String,
}

pub struct DiffImpactAnalyzer;

impl DiffImpactAnalyzer {
    pub fn detect_changes(project_root: &Path, db_path: &str) -> Result<ChangeImpactReport, String> {
        let changed_files = Self::get_git_changed_files(project_root);

        if changed_files.is_empty() {
            return Ok(ChangeImpactReport {
                changed_files: Vec::new(),
                directly_affected_symbols: Vec::new(),
                downstream_affected_symbols: Vec::new(),
                total_impact_count: 0,
                risk_level: "LOW".to_string(),
                risk_reason: "Clean workspace without uncommitted changes".to_string(),
            });
        }

        let rt = CodeIntelligenceRuntime::new(db_path);
        if !rt.is_indexed() {
            let total = changed_files.len();
            let risk = if total > 10 { "HIGH" } else if total > 3 { "MEDIUM" } else { "LOW" };
            return Ok(ChangeImpactReport {
                changed_files: changed_files.clone(),
                directly_affected_symbols: Vec::new(),
                downstream_affected_symbols: Vec::new(),
                total_impact_count: total,
                risk_level: risk.to_string(),
                risk_reason: format!("Graph not indexed; calculated risk from {} modified files", total),
            });
        }

        let conn = rt.open_ro()?;

        // 查询修改文件内定义的所有直接受波及符号
        let mut directly_affected_symbols = Vec::new();
        let mut direct_node_ids = Vec::new();

        for file in &changed_files {
            // 格式化为相对路径匹配
            let clean_file = file.replace('\\', "/");
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, qualified_name, file_path \
                     FROM nodes \
                     WHERE file_path = ?1 OR file_path LIKE ?2 \
                     ORDER BY start_line",
                )
                .map_err(|e| e.to_string())?;

            let like_pattern = format!("%/{}", clean_file);
            let rows = stmt
                .query_map(rusqlite::params![clean_file, like_pattern], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok());

            for (id, name, qn, fp) in rows {
                let callers = rt.get_callers(&id.to_string(), 50).unwrap_or_default();
                direct_node_ids.push(id);
                directly_affected_symbols.push(ChangedSymbolImpact {
                    symbol_name: name,
                    qualified_name: qn,
                    file_path: fp,
                    callers_count: callers.len(),
                });
            }
        }

        // 沿图谱追踪所有受间接影响的下游或调用方
        let mut downstream_map: HashMap<String, Vec<String>> = HashMap::new();
        let direct_set: HashSet<i64> = direct_node_ids.iter().cloned().collect();

        for &node_id in &direct_node_ids {
            let impact = rt.get_impact(&node_id.to_string(), 3).unwrap_or_default();
            for impacted_node in impact {
                let imp_id_int: i64 = impacted_node.id.parse().unwrap_or(0);
                if !direct_set.contains(&imp_id_int) {
                    let fp = impacted_node.file_path.unwrap_or_default();
                    downstream_map
                        .entry(impacted_node.label)
                        .or_default()
                        .push(fp);
                }
            }
        }

        let downstream_affected_symbols: Vec<DownstreamImpact> = downstream_map
            .into_iter()
            .map(|(name, fps)| DownstreamImpact {
                symbol_name: name.clone(),
                file_path: fps.first().cloned().unwrap_or_default(),
                via_call_chain: fps,
            })
            .collect();

        let total_impact = directly_affected_symbols.len() + downstream_affected_symbols.len();

        let (risk_level, risk_reason) = if total_impact > 15 || changed_files.len() > 10 {
            ("HIGH".to_string(), format!("High impact: {} symbols and {} files affected", total_impact, changed_files.len()))
        } else if total_impact >= 3 || changed_files.len() >= 3 {
            ("MEDIUM".to_string(), format!("Moderate impact: {} affected symbols across {} files", total_impact, changed_files.len()))
        } else {
            ("LOW".to_string(), format!("Isolated changes: {} symbol affected", total_impact))
        };

        Ok(ChangeImpactReport {
            changed_files,
            directly_affected_symbols,
            downstream_affected_symbols,
            total_impact_count: total_impact,
            risk_level,
            risk_reason,
        })
    }

    fn get_git_changed_files(project_root: &Path) -> Vec<String> {
        let mut files = HashSet::new();

        // 1. git diff --name-only
        if let Ok(output) = std::process::Command::new("git")
            .args(["diff", "--name-only"])
            .current_dir(project_root)
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        files.insert(trimmed.to_string());
                    }
                }
            }
        }

        // 2. git diff --staged --name-only
        if let Ok(output) = std::process::Command::new("git")
            .args(["diff", "--staged", "--name-only"])
            .current_dir(project_root)
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        files.insert(trimmed.to_string());
                    }
                }
            }
        }

        files.into_iter().collect()
    }

    /// Pre-edit symbol impact analysis (改前调用图谱影响雷达)
    pub fn analyze_symbol_impact(
        project_root: &Path,
        db_path: &str,
        symbol: &str,
        exclude_file: Option<&Path>,
    ) -> Result<SymbolImpactRadar, String> {
        let mut callers = Vec::new();
        let mut affected_files_set = HashSet::new();

        let clean_symbol = symbol.trim();
        if clean_symbol.is_empty() {
            return Err("Symbol name cannot be empty".to_string());
        }

        // 1. 优先尝试从 CodeIntelligenceRuntime 图数据库获取调用者拓扑
        if !db_path.trim().is_empty() {
            let rt = CodeIntelligenceRuntime::new(db_path);
            if rt.is_indexed() {
                if let Ok(conn) = rt.open_ro() {
                    if let Ok(mut stmt) = conn.prepare("SELECT id, name, file_path, start_line FROM nodes WHERE name = ?1 LIMIT 10") {
                        let matching_nodes: Vec<(i64, String, String, usize)> = stmt
                            .query_map(rusqlite::params![clean_symbol], |row| {
                                Ok((
                                    row.get::<_, i64>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2).unwrap_or_default(),
                                    row.get::<_, usize>(3).unwrap_or(1),
                                ))
                            })
                            .map(|iter| iter.filter_map(|r| r.ok()).collect())
                            .unwrap_or_default();

                        for (id, _name, _def_file, _) in matching_nodes {
                            if let Ok(graph_callers) = rt.get_callers(&id.to_string(), 30) {
                                for c in graph_callers {
                                    let file = c.file_path.unwrap_or_default();
                                    if let Some(ex) = exclude_file {
                                        let ex_str = ex.to_string_lossy().replace('\\', "/");
                                        if file.ends_with(&ex_str) {
                                            continue;
                                        }
                                    }
                                    affected_files_set.insert(file.clone());
                                    callers.push(SymbolCallerInfo {
                                        name: c.label,
                                        file,
                                        line: 1,
                                        snippet: format!("Calls symbol '{}'", clean_symbol),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. 混合韧性回退：若图谱未索引或未查到调用方，使用快速全工程符号扫描
        if callers.is_empty() && project_root.exists() {
            let symbol_word = format!(r"\b{}\b", regex::escape(clean_symbol));
            if let Ok(re) = regex::Regex::new(&symbol_word) {
                let exclude_str = exclude_file
                    .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                    .unwrap_or_default();

                let walker = walkdir::WalkDir::new(project_root)
                    .max_depth(8)
                    .into_iter()
                    .filter_entry(|e| {
                        if e.depth() == 0 {
                            return true;
                        }
                        let name = e.file_name().to_string_lossy();
                        !name.starts_with('.')
                            && name != "target"
                            && name != "node_modules"
                            && name != "dist"
                            && name != "build"
                    });

                for entry in walker.filter_map(|e| e.ok()) {
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let path = entry.path();
                    let file_name = path.file_name().unwrap_or_default().to_string_lossy();

                    // 排除定义文件自身
                    if !exclude_str.is_empty() && file_name == exclude_str {
                        continue;
                    }

                    // 过滤源代码文件后缀
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                    if !matches!(ext, "rs" | "java" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "kt") {
                        continue;
                    }

                    if let Ok(content) = std::fs::read_to_string(path) {
                        for (idx, line) in content.lines().enumerate() {
                            if re.is_match(line) {
                                let rel_path = path
                                    .strip_prefix(project_root)
                                    .unwrap_or(path)
                                    .to_string_lossy()
                                    .replace('\\', "/");
                                affected_files_set.insert(rel_path.clone());
                                callers.push(SymbolCallerInfo {
                                    name: file_name.to_string(),
                                    file: rel_path,
                                    line: idx + 1,
                                    snippet: line.trim().chars().take(120).collect(),
                                });
                                if callers.len() >= 25 {
                                    break;
                                }
                            }
                        }
                    }
                    if callers.len() >= 25 {
                        break;
                    }
                }
            }
        }

        let total_callers = callers.len();
        let affected_files: Vec<String> = affected_files_set.into_iter().collect();

        let (risk_level, recommendation) = if total_callers >= 10 || affected_files.len() >= 4 {
            (
                "HIGH".to_string(),
                format!(
                    "High Impact: Symbol '{}' is referenced by {} external locations across {} files. Modifying signature or semantics will likely break downstream modules. Coordinated refactoring required.",
                    clean_symbol, total_callers, affected_files.len()
                ),
            )
        } else if total_callers > 0 {
            (
                "MEDIUM".to_string(),
                format!(
                    "Moderate Impact: Symbol '{}' is referenced by {} external caller(s) in {:?}. Ensure caller arguments and contracts are preserved.",
                    clean_symbol, total_callers, affected_files
                ),
            )
        } else {
            (
                "LOW".to_string(),
                format!(
                    "Isolated: No external references found for '{}'. Safe to modify locally.",
                    clean_symbol
                ),
            )
        };

        Ok(SymbolImpactRadar {
            symbol: clean_symbol.to_string(),
            total_callers,
            callers,
            affected_files,
            risk_level,
            recommendation,
        })
    }
}

/// Helper to extract declared function/class/struct identifier from code snippet
pub fn extract_declared_symbol(code: &str) -> Option<String> {
    for line in code.lines() {
        let trimmed = line.trim();
        // Rust: pub fn func_name / fn func_name / pub struct Foo / struct Foo / enum Bar / trait Baz
        if let Some(rest) = trimmed
            .strip_prefix("pub fn ")
            .or_else(|| trimmed.strip_prefix("fn "))
            .or_else(|| trimmed.strip_prefix("pub async fn "))
            .or_else(|| trimmed.strip_prefix("async fn "))
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
        if let Some(rest) = trimmed
            .strip_prefix("pub struct ")
            .or_else(|| trimmed.strip_prefix("struct "))
            .or_else(|| trimmed.strip_prefix("pub enum "))
            .or_else(|| trimmed.strip_prefix("enum "))
            .or_else(|| trimmed.strip_prefix("pub trait "))
            .or_else(|| trimmed.strip_prefix("trait "))
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
        // Java: public/protected/private class Foo / interface Foo / enum Foo
        if let Some(pos) = trimmed
            .find("class ")
            .or_else(|| trimmed.find("interface "))
            .or_else(|| trimmed.find("enum "))
        {
            let after = &trimmed[pos..];
            let parts: Vec<&str> = after.split_whitespace().collect();
            if parts.len() >= 2 {
                let name: String = parts[1]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
        // Java method: public/protected Result<...> methodName(
        if (trimmed.starts_with("public ") || trimmed.starts_with("protected "))
            && trimmed.contains('(')
        {
            if let Some(paren_pos) = trimmed.find('(') {
                let before_paren = &trimmed[..paren_pos];
                if let Some(method_name) = before_paren.split_whitespace().last() {
                    let clean: String = method_name
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !clean.is_empty() && clean != "class" && clean != "interface" {
                        return Some(clean);
                    }
                }
            }
        }
        // TypeScript/JavaScript: export function name / export const name / function name
        if let Some(rest) = trimmed
            .strip_prefix("export function ")
            .or_else(|| trimmed.strip_prefix("function "))
            .or_else(|| trimmed.strip_prefix("export const "))
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
        // Python: def name( / class Name
        if let Some(rest) = trimmed
            .strip_prefix("def ")
            .or_else(|| trimmed.strip_prefix("async def "))
            .or_else(|| trimmed.strip_prefix("class "))
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_changes_clean_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let report = DiffImpactAnalyzer::detect_changes(dir.path(), "").unwrap();
        assert_eq!(report.risk_level, "LOW");
        assert_eq!(report.total_impact_count, 0);
    }

    #[test]
    fn test_extract_declared_symbol() {
        assert_eq!(
            extract_declared_symbol("pub async fn execute_tool(args: Value) -> Result<Value, String> {"),
            Some("execute_tool".to_string())
        );
        assert_eq!(
            extract_declared_symbol("public Result<Long> createJob(@Valid @RequestBody CreateJobCmd cmd) {"),
            Some("createJob".to_string())
        );
        assert_eq!(
            extract_declared_symbol("export function useAgentTask(taskId: string) {"),
            Some("useAgentTask".to_string())
        );
        assert_eq!(
            extract_declared_symbol("def process_request(data: dict):"),
            Some("process_request".to_string())
        );
        assert_eq!(
            extract_declared_symbol("    let x = 42;"),
            None
        );
    }

    #[test]
    fn test_analyze_symbol_impact_workspace_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let caller_file = dir.path().join("caller.rs");
        let def_file = dir.path().join("def.rs");

        std::fs::write(&def_file, "pub fn calculate_hash() -> u64 { 123 }").unwrap();
        std::fs::write(&caller_file, "use crate::def::calculate_hash;\nfn run() { calculate_hash(); }").unwrap();

        let radar = DiffImpactAnalyzer::analyze_symbol_impact(
            dir.path(),
            "",
            "calculate_hash",
            Some(&def_file),
        ).unwrap();

        assert_eq!(radar.symbol, "calculate_hash");
        assert_eq!(radar.total_callers, 2); // 2 references in caller.rs
        assert!(radar.affected_files.iter().any(|f| f.contains("caller.rs")));
        assert_eq!(radar.risk_level, "MEDIUM");
    }
}

