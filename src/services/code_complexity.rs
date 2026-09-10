//! Code complexity and hot-path bottleneck detector.
//!
//! Inspired by codebase-memory-mcp's graph complexity and hot-path detection.
//! Analyzes cyclomatic complexity, cognitive complexity, loop nesting depth,
//! linear scans in loops (hidden O(N^2) risks), allocations in loops, and recursive calls.

use serde::{Deserialize, Serialize};
use rusqlite::Connection;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionComplexity {
    pub name: String,
    pub file_path: String,
    pub cyclomatic: usize,
    pub cognitive: usize,
    pub loop_depth: usize,
    pub linear_scan_in_loop: bool,
    pub alloc_in_loop: bool,
    pub is_recursive: bool,
    pub smell_score: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComplexityReport {
    pub total_functions_analyzed: usize,
    pub high_complexity_top: Vec<FunctionComplexity>,
    pub hot_path_risks: Vec<FunctionComplexity>,
    pub summary: String,
}

pub struct ComplexityAnalyzer;

impl ComplexityAnalyzer {
    /// Analyzes the raw source code of a single function or method
    pub fn analyze_function_source(name: &str, file_path: &str, source: &str) -> FunctionComplexity {
        let mut cyclomatic = 1;
        let mut cognitive = 0;
        let mut current_depth = 0;
        let mut max_loop_depth = 0;
        let mut in_loop = false;
        let mut loop_stack = Vec::new();
        let mut linear_scan_in_loop = false;
        let mut alloc_in_loop = false;
        let mut is_recursive = false;

        let clean_name = name.split("::").last().unwrap_or(name).trim();

        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }

            // Track block nesting
            let open_braces = trimmed.chars().filter(|&c| c == '{').count();
            let close_braces = trimmed.chars().filter(|&c| c == '}').count();

            // Branch keyword checks for cyclomatic complexity
            let is_branch = trimmed.starts_with("if ")
                || trimmed.starts_with("else if ")
                || trimmed.starts_with("match ")
                || trimmed.starts_with("case ")
                || trimmed.starts_with("catch ")
                || trimmed.contains(" && ")
                || trimmed.contains(" || ");

            if is_branch {
                cyclomatic += 1;
                cognitive += 1 + current_depth;
            }

            // Loop detection
            let is_loop_start = trimmed.starts_with("for ")
                || trimmed.starts_with("while ")
                || trimmed.starts_with("loop ")
                || trimmed.starts_with("for (")
                || trimmed.starts_with("while (");

            if is_loop_start {
                cyclomatic += 1;
                cognitive += 1 + current_depth;
                in_loop = true;
                loop_stack.push(current_depth);
                let current_loop_nesting = loop_stack.len();
                if current_loop_nesting > max_loop_depth {
                    max_loop_depth = current_loop_nesting;
                }
            }

            // Check operations inside loops
            if in_loop {
                // Linear scan detection
                if trimmed.contains(".contains(")
                    || trimmed.contains(".find(")
                    || trimmed.contains(".filter(")
                    || trimmed.contains(".indexOf(")
                    || trimmed.contains(".position(")
                {
                    linear_scan_in_loop = true;
                }

                // Allocations inside loop
                if trimmed.contains(".push(")
                    || trimmed.contains(".clone()")
                    || trimmed.contains("format!(")
                    || trimmed.contains(".to_string()")
                    || trimmed.contains("Vec::new()")
                    || trimmed.contains("new ")
                {
                    alloc_in_loop = true;
                }
            }

            // Check recursion
            if !clean_name.is_empty() && trimmed.contains(&format!("{}(", clean_name)) {
                // Ignore function header line itself
                if !trimmed.starts_with("fn ") && !trimmed.starts_with("pub fn ") && !trimmed.starts_with("def ") && !trimmed.starts_with("function ") {
                    is_recursive = true;
                }
            }

            // Update brace depth
            if open_braces > close_braces {
                current_depth += open_braces - close_braces;
            } else if close_braces > open_braces {
                current_depth = current_depth.saturating_sub(close_braces - open_braces);
                // Check if loops have terminated
                while let Some(&loop_entry_depth) = loop_stack.last() {
                    if current_depth <= loop_entry_depth {
                        loop_stack.pop();
                    } else {
                        break;
                    }
                }
                if loop_stack.is_empty() {
                    in_loop = false;
                }
            }
        }

        // Calculate smell score
        let mut smell_score = 0;
        if cyclomatic > 10 {
            smell_score += (cyclomatic - 10) * 2;
        }
        if cognitive > 15 {
            smell_score += (cognitive - 15) * 2;
        }
        if max_loop_depth >= 2 {
            smell_score += max_loop_depth * 5;
        }
        if linear_scan_in_loop {
            smell_score += 15; // Major hidden bottleneck
        }
        if alloc_in_loop {
            smell_score += 8;
        }
        if is_recursive {
            smell_score += 10;
        }

        FunctionComplexity {
            name: name.to_string(),
            file_path: file_path.to_string(),
            cyclomatic,
            cognitive,
            loop_depth: max_loop_depth,
            linear_scan_in_loop,
            alloc_in_loop,
            is_recursive,
            smell_score,
        }
    }

    /// Evaluates functions indexed in graph database
    pub fn analyze_project_complexity(db_path: &str, limit: usize) -> Result<ComplexityReport, String> {
        let p = Path::new(db_path);
        if !p.exists() {
            return Ok(ComplexityReport {
                total_functions_analyzed: 0,
                high_complexity_top: Vec::new(),
                hot_path_risks: Vec::new(),
                summary: "Database not found.".to_string(),
            });
        }

        let conn = Connection::open_with_flags(
            p,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ).map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT name, file_path, properties FROM nodes WHERE label IN ('Function', 'Method') LIMIT ?1"
        ).map_err(|e| e.to_string())?;

        let rows = stmt.query_map(rusqlite::params![(limit * 3) as i64], |r| {
            let name: String = r.get(0)?;
            let file_path: String = r.get(1)?;
            let props: Option<String> = r.get(2).ok();
            Ok((name, file_path, props.unwrap_or_default()))
        }).map_err(|e| e.to_string())?;

        let mut analyzed = Vec::new();
        for item in rows.flatten() {
            let (name, file, props) = item;
            // If body source snippet is available in properties, analyze it, else estimate from name/props
            let mut comp = Self::analyze_function_source(&name, &file, &props);
            // Fallback estimation if properties does not have source
            if comp.cyclomatic == 1 && props.contains("complexity") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&props) {
                    if let Some(c) = v.get("complexity").and_then(|x| x.as_u64()) {
                        comp.cyclomatic = c as usize;
                        comp.smell_score = comp.cyclomatic.saturating_sub(5) * 2;
                    }
                }
            }
            analyzed.push(comp);
        }

        let total = analyzed.len();
        analyzed.sort_by(|a, b| b.smell_score.cmp(&a.smell_score));

        let high_complexity = analyzed.iter().take(limit).cloned().collect();
        let hot_paths = analyzed
            .into_iter()
            .filter(|c| c.linear_scan_in_loop || c.loop_depth >= 2)
            .take(limit)
            .collect();

        let summary = format!(
            "Analyzed {} functions from graph. Found high complexity bottlenecks and hot paths.",
            total
        );

        Ok(ComplexityReport {
            total_functions_analyzed: total,
            high_complexity_top: high_complexity,
            hot_path_risks: hot_paths,
            summary,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_hot_path_with_linear_scan_in_loop() {
        let code = r#"
            pub fn find_matches(items: &[String], targets: &[String]) -> Vec<String> {
                let mut result = Vec::new();
                for item in items {
                    for target in targets {
                        if item.contains(target) {
                            result.push(item.clone());
                        }
                    }
                }
                result
            }
        "#;

        let comp = ComplexityAnalyzer::analyze_function_source("find_matches", "search.rs", code);
        assert!(comp.loop_depth >= 2);
        assert!(comp.linear_scan_in_loop);
        assert!(comp.alloc_in_loop);
        assert!(comp.smell_score > 20);
    }
}
