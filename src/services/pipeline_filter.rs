//! Generic 8-stage pipeline command output filter engine.
//!
//! Inspired by RTK's architecture (docs/contributing/TECHNICAL.md and builtin filters).
//! Implements an 8-stage pipeline:
//! 1. strip_ansi
//! 2. replace (regex substitutions)
//! 3. match_output (fast short-circuit checks)
//! 4. strip_lines_matching & keep_lines_matching
//! 5. truncate_lines_at
//! 6. tail_lines
//! 7. max_lines
//! 8. on_empty

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineRule {
    pub name: String,
    pub description: String,
    pub match_command: String,
    pub strip_ansi: bool,
    pub replace: Vec<(String, String)>,
    pub match_output: Vec<(String, String)>,
    pub strip_lines_matching: Vec<String>,
    pub keep_lines_matching: Vec<String>,
    pub truncate_lines_at: Option<usize>,
    pub tail_lines: Option<usize>,
    pub max_lines: Option<usize>,
    pub on_empty: Option<String>,
}

pub struct PipelineFilterEngine;

impl PipelineFilterEngine {
    /// Executes the full 8-stage filtering pipeline according to the given rule
    pub fn filter(input: &str, rule: &PipelineRule) -> String {
        // Stage 1: strip_ansi
        let text = if rule.strip_ansi {
            Self::strip_ansi_codes(input)
        } else {
            input.to_string()
        };

        // Stage 2: regex replace
        let mut text = text;
        for (pat, rep) in &rule.replace {
            if let Ok(re) = Regex::new(pat) {
                text = re.replace_all(&text, rep.as_str()).to_string();
            }
        }

        // Stage 3: match_output short-circuit check
        for (pat, short_msg) in &rule.match_output {
            if let Ok(re) = Regex::new(pat) {
                if re.is_match(&text) {
                    return short_msg.clone();
                }
            }
        }

        // Stage 4: line-by-line strip / keep
        let strip_res: Vec<Regex> = rule
            .strip_lines_matching
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        let keep_res: Vec<Regex> = rule
            .keep_lines_matching
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        let mut lines: Vec<String> = text
            .lines()
            .filter(|line| {
                // If keep list is specified, line must match at least one
                if !keep_res.is_empty() && !keep_res.iter().any(|r| r.is_match(line)) {
                    return false;
                }
                // If strip list is specified, line must not match any
                if strip_res.iter().any(|r| r.is_match(line)) {
                    return false;
                }
                true
            })
            .map(|s| s.to_string())
            .collect();

        // Stage 5: truncate_lines_at
        if let Some(max_len) = rule.truncate_lines_at {
            for line in &mut lines {
                if line.len() > max_len {
                    line.truncate(max_len);
                    line.push_str("...");
                }
            }
        }

        // Stage 6: tail_lines (keep only last N lines)
        if let Some(tail_n) = rule.tail_lines {
            if lines.len() > tail_n {
                let start = lines.len() - tail_n;
                lines = lines[start..].to_vec();
            }
        }

        // Stage 7: max_lines (keep first N lines)
        if let Some(max_n) = rule.max_lines {
            if lines.len() > max_n {
                lines.truncate(max_n);
                lines.push(format!("... (truncated {} remaining lines)", lines.len()));
            }
        }

        // Join filtered lines
        let result = lines.join("\n").trim().to_string();

        // Stage 8: on_empty fallback
        if result.is_empty() {
            if let Some(on_empty) = &rule.on_empty {
                return on_empty.clone();
            }
        }

        result
    }

    /// Strips standard ANSI terminal escape sequences
    pub fn strip_ansi_codes(s: &str) -> String {
        let ansi_regex = Regex::new(r"\x1B\[[0-?]*[ -/]*[@-~]").unwrap();
        ansi_regex.replace_all(s, "").to_string()
    }

    /// Finds a matching rule from standard built-in rules for the command string
    pub fn match_builtin_rule(cmd: &str) -> Option<&'static PipelineRule> {
        let cmd_clean = cmd.trim();
        for rule in get_builtin_rules() {
            if let Ok(re) = Regex::new(&rule.match_command) {
                if re.is_match(cmd_clean) {
                    return Some(rule);
                }
            }
        }
        None
    }
}

pub fn get_builtin_rules() -> &'static [PipelineRule] {
    static RULES: std::sync::OnceLock<Vec<PipelineRule>> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            PipelineRule {
                name: "cargo-clippy".to_string(),
                description: "Filters compilation banners from cargo clippy".to_string(),
                match_command: r"^cargo\s+clippy".to_string(),
                strip_ansi: true,
                replace: Vec::new(),
                match_output: vec![(
                    r"Finished.*dev.*target\(s\)\s+in".to_string(),
                    "cargo clippy: ok (no warnings)".to_string(),
                )],
                strip_lines_matching: vec![
                    r"^\s*Compiling\s+".to_string(),
                    r"^\s*Checking\s+".to_string(),
                    r"^\s*Finished\s+".to_string(),
                ],
                keep_lines_matching: Vec::new(),
                truncate_lines_at: Some(160),
                tail_lines: None,
                max_lines: Some(100),
                on_empty: Some("cargo clippy: ok".to_string()),
            },
            PipelineRule {
                name: "eslint".to_string(),
                description: "Compresses ESLint output to errors and rule violations".to_string(),
                match_command: r"^(npx\s+)?eslint".to_string(),
                strip_ansi: true,
                replace: Vec::new(),
                match_output: vec![(
                    r"✨\s+Done in".to_string(),
                    "eslint: ok (0 errors)".to_string(),
                )],
                strip_lines_matching: vec![
                    r"^\s*$".to_string(),
                    r"^\s*info\s+".to_string(),
                ],
                keep_lines_matching: Vec::new(),
                truncate_lines_at: Some(180),
                tail_lines: None,
                max_lines: Some(120),
                on_empty: Some("eslint: ok (clean)".to_string()),
            },
            PipelineRule {
                name: "terraform-plan".to_string(),
                description: "Extracts summary changes from terraform plan".to_string(),
                match_command: r"^terraform\s+plan".to_string(),
                strip_ansi: true,
                replace: Vec::new(),
                match_output: vec![(
                    r"No changes\. Your infrastructure matches the configuration".to_string(),
                    "terraform plan: no changes".to_string(),
                )],
                strip_lines_matching: vec![
                    r"^Refreshing state\.\.\.".to_string(),
                    r"^\s*Reading\.\.\.".to_string(),
                ],
                keep_lines_matching: Vec::new(),
                truncate_lines_at: Some(140),
                tail_lines: Some(40),
                max_lines: Some(50),
                on_empty: Some("terraform plan: ok".to_string()),
            },
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_strip_ansi_and_short_circuit() {
        let rule = &get_builtin_rules()[0]; // cargo-clippy
        let raw = "\x1B[32m    Finished\x1B[0m dev [unoptimized + debuginfo] target(s) in 0.5s";
        let filtered = PipelineFilterEngine::filter(raw, rule);
        assert_eq!(filtered, "cargo clippy: ok (no warnings)");
    }

    #[test]
    fn test_pipeline_strip_noise_lines() {
        let rule = &get_builtin_rules()[0];
        let raw = r#"
   Compiling libc v0.2.140
   Checking my-crate v0.1.0
warning: unused variable: `x`
 --> src/main.rs:4:9
   Compiling tokio v1.28.0
        "#;
        let filtered = PipelineFilterEngine::filter(raw, rule);
        assert!(!filtered.contains("Compiling"));
        assert!(!filtered.contains("Checking"));
        assert!(filtered.contains("warning: unused variable: `x`"));
    }
}
