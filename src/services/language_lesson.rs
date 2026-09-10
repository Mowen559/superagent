//! Language concepts and engineering design patterns analyzer.
//!
//! Inspired by UnderstandAnything's `language-lesson.ts`. Detects software design
//! patterns and programming paradigms across codebases and graph nodes (e.g.
//! Dependency Injection, Observer, Middleware, Singleton, Repository, Concurrency, etc.).

use serde::{Deserialize, Serialize};
use rusqlite::Connection;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LanguageConcept {
    pub name: String,
    pub category: String,
    pub description: String,
    pub matched_symbols: Vec<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LanguageLessonReport {
    pub language: String,
    pub concepts: Vec<LanguageConcept>,
    pub summary: String,
}

struct ConceptPattern {
    name: &'static str,
    category: &'static str,
    description: &'static str,
    keywords: &'static [&'static str],
}

const CONCEPT_PATTERNS: &[ConceptPattern] = &[
    ConceptPattern {
        name: "Dependency Injection",
        category: "Architectural Pattern",
        description: "Decouples component construction from usage via container or injection points.",
        keywords: &["inject", "provider", "container", "autowired", "@inject", "@autowired", "di_container", "provide"],
    },
    ConceptPattern {
        name: "Observer Pattern / Event Driven",
        category: "Behavioral Pattern",
        description: "Enables loose coupling through publish-subscribe or listener mechanisms.",
        keywords: &["subscribe", "publish", "event", "observable", "listener", "emit", "receiver", "sender", "eventbus"],
    },
    ConceptPattern {
        name: "Middleware Pipeline",
        category: "Structural Pattern",
        description: "Composes requests/commands through an extensible pipeline of interceptors.",
        keywords: &["middleware", "interceptor", "pipeline", "filter_chain", "tower::layer", "handle_request"],
    },
    ConceptPattern {
        name: "Singleton / Shared Instance",
        category: "Creational Pattern",
        description: "Ensures a type has only one instance and provides global or shared access.",
        keywords: &["singleton", "shared_instance", "lazy_static", "once_cell", "arc_swap", "get_instance"],
    },
    ConceptPattern {
        name: "Repository / DAO Pattern",
        category: "Data Access Pattern",
        description: "Mediates between domain logic and data mapping layers via collection-like interface.",
        keywords: &["repository", "dao", "mapper", "store", "find_by_", "save(", "delete("],
    },
    ConceptPattern {
        name: "Builder Pattern",
        category: "Creational Pattern",
        description: "Separates the construction of a complex object from its representation.",
        keywords: &["builder", "build()", "with_", "set_builder"],
    },
    ConceptPattern {
        name: "Concurrency & Channels",
        category: "Concurrency Paradigm",
        description: "Coordinates concurrent tasks using message passing channels or shared-memory primitives.",
        keywords: &["async", "await", "channel", "mpsc", "tokio::spawn", "thread", "mutex", "rwlock", "select!"],
    },
    ConceptPattern {
        name: "Error Handling & Result Pattern",
        category: "Language Idiom",
        description: "Encodes operation outcomes explicitly as values (Result/Either/Error) rather than unchecked panics.",
        keywords: &["result<", "either<", "try_catch", "errorhandler", "err(", "unwrap_or"],
    },
    ConceptPattern {
        name: "Type Narrowing & Pattern Matching",
        category: "Language Idiom",
        description: "Employs algebraic data types, discriminated unions or structural pattern matching.",
        keywords: &["match ", "pattern", "type_guard", "discriminated", "exhaustive_match", "switch"],
    },
    ConceptPattern {
        name: "Streams & Reactive Data Flow",
        category: "Data Processing",
        description: "Transforms sequential or asynchronous events via chained stream operators.",
        keywords: &["stream", "pipe", "transform", "readable", "writable", "flow", "flux", "mono"],
    },
];

/// Detects concepts present in arbitrary text/source code content
pub fn detect_concepts_in_content(content: &str, _ext: &str) -> Vec<LanguageConcept> {
    let lower = content.to_lowercase();
    let mut detected = Vec::new();

    for cp in CONCEPT_PATTERNS {
        let mut matches = Vec::new();
        for kw in cp.keywords {
            if lower.contains(kw) {
                matches.push(kw.to_string());
            }
        }

        if !matches.is_empty() {
            let confidence = ((matches.len() as f64 * 0.25).min(1.0) * 100.0).round() / 100.0;
            detected.push(LanguageConcept {
                name: cp.name.to_string(),
                category: cp.category.to_string(),
                description: cp.description.to_string(),
                matched_symbols: matches,
                confidence,
            });
        }
    }

    detected.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    detected
}

/// Scans the graph database nodes (functions, classes, modules, summaries) to detect project-wide concepts
pub fn detect_project_concepts(db_path: &str, sample_limit: usize) -> Result<LanguageLessonReport, String> {
    let path = std::path::Path::new(db_path);
    if !path.exists() {
        return Ok(LanguageLessonReport {
            language: "unknown".to_string(),
            concepts: Vec::new(),
            summary: "Database does not exist or has not been indexed yet.".to_string(),
        });
    }

    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ).map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT name, label, properties FROM nodes WHERE label IN ('Function', 'Method', 'Class', 'Interface', 'Module') LIMIT ?1"
    ).map_err(|e| e.to_string())?;

    let mut symbol_corpus = String::new();
    let rows = stmt.query_map(rusqlite::params![sample_limit as i64], |row| {
        let name: String = row.get(0)?;
        let label: String = row.get(1)?;
        let props: Option<String> = row.get(2).ok();
        Ok(format!("{} {} {}", label, name, props.unwrap_or_default()))
    }).map_err(|e| e.to_string())?;

    for item in rows.flatten() {
        symbol_corpus.push_str(&item);
        symbol_corpus.push(' ');
    }

    let concepts = detect_concepts_in_content(&symbol_corpus, "");
    let summary = if concepts.is_empty() {
        "No distinctive architectural patterns detected from graph nodes.".to_string()
    } else {
        format!(
            "Identified {} design and architectural concepts in codebase.",
            concepts.len()
        )
    };

    Ok(LanguageLessonReport {
        language: "multi-paradigm".to_string(),
        concepts,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_concepts_in_content() {
        let code = r#"
            pub struct EventDispatcher {
                listeners: Vec<Box<dyn Fn(&Event)>>,
            }
            impl EventDispatcher {
                pub fn subscribe(&mut self, listener: impl Fn(&Event) + 'static) {
                    self.listeners.push(Box::new(listener));
                }
                pub fn publish(&self, event: &Event) {
                    for l in &self.listeners { l(event); }
                }
            }

            pub async fn process_job(id: u64) -> Result<(), String> {
                let (tx, mut rx) = mpsc::channel(100);
                tokio::spawn(async move {
                    tx.send(1).await.ok();
                });
                Ok(())
            }
        "#;

        let concepts = detect_concepts_in_content(code, "rs");
        assert!(!concepts.is_empty());

        let names: Vec<String> = concepts.iter().map(|c| c.name.clone()).collect();
        assert!(names.iter().any(|n| n.contains("Observer")));
        assert!(names.iter().any(|n| n.contains("Concurrency")));
        assert!(names.iter().any(|n| n.contains("Error Handling")));
    }
}
