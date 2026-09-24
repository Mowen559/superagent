use std::path::Path;
use super::types::{ReviewCategory, ReviewRule, ReviewSeverity};

pub struct RuleRegistry {
    rules: Vec<ReviewRule>,
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self {
            rules: Self::builtin_rules(),
        }
    }

    /// 内置工业级通用与高危缺陷规则库（参考 Alibaba OpenCodeReview）
    pub fn builtin_rules() -> Vec<ReviewRule> {
        vec![
            ReviewRule {
                id: "SEC-SQLI-001".to_string(),
                name: "SQL Injection Risk".to_string(),
                language: "all".to_string(),
                severity: ReviewSeverity::Critical,
                category: ReviewCategory::Security,
                description: "Avoid direct string concatenation or formatting inside raw SQL queries. Use parameterized queries or ORM bindings.".to_string(),
                patterns: vec![
                    "SELECT .* FROM .* WHERE .* =\\s*[\"']\\s*\\+".to_string(),
                    "format!\\(\".*SELECT .* FROM .*\"".to_string(),
                    "query\\(.*\\+.*\\)".to_string(),
                ],
            },
            ReviewRule {
                id: "BUG-NPE-001".to_string(),
                name: "Potential Null/None Dereference".to_string(),
                language: "all".to_string(),
                severity: ReviewSeverity::Critical,
                category: ReviewCategory::Bug,
                description: "Unchecked access on nullable or Option/Result types may lead to panic, crash, or NullPointerException.".to_string(),
                patterns: vec![
                    "\\.unwrap\\(\\)".to_string(),
                    "\\.expect\\(\\)".to_string(),
                    "(\\w+)\\.get\\(.*\\)\\.toString\\(\\)".to_string(),
                ],
            },
            ReviewRule {
                id: "RES-LEAK-001".to_string(),
                name: "Unclosed Resource or File Handle Leak".to_string(),
                language: "all".to_string(),
                severity: ReviewSeverity::Warning,
                category: ReviewCategory::ResourceLeak,
                description: "Open streams, network sockets, or file handles must be enclosed in RAII, try-with-resources, or defer/finally blocks.".to_string(),
                patterns: vec![
                    "new FileInputStream\\(".to_string(),
                    "os\\.Open\\(".to_string(),
                    "open\\(.*\\)\\.read\\(\\)".to_string(),
                ],
            },
            ReviewRule {
                id: "CONC-RACE-001".to_string(),
                name: "Thread-Safety or Concurrent Race Hazard".to_string(),
                language: "all".to_string(),
                severity: ReviewSeverity::Critical,
                category: ReviewCategory::Concurrency,
                description: "Mutable shared state modified across asynchronous tasks or threads without adequate synchronization primitives.".to_string(),
                patterns: vec![
                    "static mut ".to_string(),
                    "unsafe \\{".to_string(),
                    "HashMap<.*>".to_string(),
                ],
            },
            ReviewRule {
                id: "SEC-SECRET-001".to_string(),
                name: "Hardcoded Credential or API Key".to_string(),
                language: "all".to_string(),
                severity: ReviewSeverity::Critical,
                category: ReviewCategory::Security,
                description: "Avoid embedding cleartext secrets, tokens, passwords, or private keys directly in source code.".to_string(),
                patterns: vec![
                    "(?i)api[_-]?key\\s*=\\s*[\"'][a-zA-Z0-9_\\-]{16,}[\"']".to_string(),
                    "(?i)password\\s*=\\s*[\"'][^\"']+[\"']".to_string(),
                    "(?i)bearer\\s+[a-zA-Z0-9_\\-\\.]+".to_string(),
                ],
            },
            ReviewRule {
                id: "PERF-LOOP-001".to_string(),
                name: "N+1 Query or Inefficient Loop IO".to_string(),
                language: "all".to_string(),
                severity: ReviewSeverity::Warning,
                category: ReviewCategory::Performance,
                description: "Database queries, remote HTTP calls, or file reads inside loops may severely degrade performance.".to_string(),
                patterns: vec![
                    "for .* in .* \\{.*\\.query\\(.*\\)".to_string(),
                    "while .* \\{.*\\.send\\(.*\\)".to_string(),
                ],
            },
        ]
    }

    /// 尝试从工作区根目录读取自定义规则文件 (.opencodereview/rule.json 或 .superagent/rule.json)
    pub fn load_workspace_rules(workspace_root: &Path) -> Self {
        let mut registry = Self::new();
        let candidate_paths = [
            workspace_root.join(".opencodereview").join("rule.json"),
            workspace_root.join(".superagent").join("rule.json"),
        ];

        for path in candidate_paths {
            if path.exists() && path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(custom_rules) = serde_json::from_str::<Vec<ReviewRule>>(&content) {
                        for rule in custom_rules {
                            registry.rules.retain(|r| r.id != rule.id);
                            registry.rules.push(rule);
                        }
                    }
                }
                break;
            }
        }

        registry
    }

    pub fn rules(&self) -> &[ReviewRule] {
        &self.rules
    }

    /// 查找匹配特定语言或全语言的规则列表
    pub fn find_rules_for_file(&self, file_path: &str) -> Vec<&ReviewRule> {
        let ext = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let lang = match ext.as_str() {
            "rs" => "rust",
            "go" => "go",
            "java" => "java",
            "py" => "python",
            "js" | "jsx" | "ts" | "tsx" | "mjs" => "javascript",
            _ => "unknown",
        };

        self.rules
            .iter()
            .filter(|r| r.language == "all" || r.language == lang)
            .collect()
    }
}
