use std::collections::{HashSet, HashMap};
use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::services::code_intelligence::CodeIntelligenceRuntime;

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
}

