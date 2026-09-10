use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::services::architecture_analyzer::ArchitectureAnalyzer;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingGuide {
    pub project_name: String,
    pub project_path: String,
    pub tech_stack: Vec<String>,
    pub entry_points: Vec<String>,
    pub layers_summary: Vec<LayerSummaryItem>,
    pub recommended_reading_order: Vec<ReadingStep>,
    pub development_workflows: Vec<WorkflowCommand>,
    pub architectural_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerSummaryItem {
    pub layer: String,
    pub description: String,
    pub file_count: usize,
    pub example_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingStep {
    pub step: usize,
    pub title: String,
    pub target_file: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCommand {
    pub name: String,
    pub command: String,
    pub description: String,
}

pub struct OnboardService;

impl OnboardService {
    pub fn generate_guide(project_root: &Path, db_path: Option<&str>) -> Result<OnboardingGuide, String> {
        let project_name = project_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown Project")
            .to_string();
        let project_path_str = project_root.to_string_lossy().to_string();

        let tech_stack = Self::detect_tech_stack(project_root);
        let workflows = Self::detect_workflows(project_root);

        let mut layers_summary = Vec::new();
        let mut entry_points = Vec::new();
        let mut reading_order = Vec::new();

        // 尝试结合图谱分层与导览
        if let Some(dbp) = db_path {
            if let Ok(report) = ArchitectureAnalyzer::detect_layers_deterministic(dbp) {
                for layer in report.layers {
                    let count = layer.file_count;
                    let examples: Vec<String> = layer.files.iter().take(3).cloned().collect();
                    let desc = match layer.name.as_str() {
                        "API" => "HTTP controllers, REST endpoints, and external RPC entry surfaces",
                        "Service" => "Application business use cases and process orchestration services",
                        "Domain" => "Core domain models, aggregates, and business entities",
                        "Data" => "Data persistence, SQL repositories, and mappers",
                        "Infrastructure" => "External system adapters, messaging, and cloud drivers",
                        "UI" => "Frontend views, components, and user interaction logic",
                        _ => "Utility routines and cross-cutting helpers",
                    };
                    layers_summary.push(LayerSummaryItem {
                        layer: layer.name,
                        description: desc.to_string(),
                        file_count: count,
                        example_files: examples,
                    });
                }
            }
        }

        // 探测关键入口文件
        let candidate_entries = [
            ("src/main.rs", "Rust Application Entrypoint"),
            ("src/lib.rs", "Rust Library Root"),
            ("main.go", "Go Application Main"),
            ("index.ts", "TypeScript/Node Entrypoint"),
            ("src/index.ts", "TypeScript Source Entrypoint"),
            ("src/App.tsx", "React Frontend Root Component"),
            ("app.py", "Python Web Entrypoint"),
            ("main.py", "Python Application Main"),
        ];

        for (rel_path, desc) in candidate_entries {
            let full = project_root.join(rel_path);
            if full.exists() {
                entry_points.push(format!("{} ({})", rel_path, desc));
                reading_order.push(ReadingStep {
                    step: reading_order.len() + 1,
                    title: format!("Inspect {}", rel_path),
                    target_file: rel_path.to_string(),
                    purpose: desc.to_string(),
                });
            }
        }

        if reading_order.is_empty() {
            reading_order.push(ReadingStep {
                step: 1,
                title: "Examine Project Manifest".to_string(),
                target_file: if project_root.join("Cargo.toml").exists() {
                    "Cargo.toml".to_string()
                } else if project_root.join("package.json").exists() {
                    "package.json".to_string()
                } else {
                    "README.md".to_string()
                },
                purpose: "Understand core dependencies and project dependencies overview".to_string(),
            });
        }

        let architectural_rules = vec![
            "Strict Directional Dependencies: Upper layers may call lower layers; lower layers must never import higher layers.".to_string(),
            "Domain Purity: Domain models and entities must not depend on database or web framework specifics.".to_string(),
            "External Isolation: All third-party SDK calls and cloud resources must be mediated by Infrastructure adapters.".to_string(),
            "Safe Concurrency: Use read-only transactions for analytical queries; preserve idempotent mutations.".to_string(),
        ];

        Ok(OnboardingGuide {
            project_name,
            project_path: project_path_str,
            tech_stack,
            entry_points,
            layers_summary,
            recommended_reading_order: reading_order,
            development_workflows: workflows,
            architectural_rules,
        })
    }

    fn detect_tech_stack(root: &Path) -> Vec<String> {
        let mut stack = Vec::new();
        if root.join("Cargo.toml").exists() {
            stack.push("Rust (Cargo)".to_string());
        }
        if root.join("package.json").exists() {
            stack.push("TypeScript / JavaScript (Node.js)".to_string());
            if root.join("pnpm-lock.yaml").exists() {
                stack.push("Package Manager: pnpm".to_string());
            } else if root.join("yarn.lock").exists() {
                stack.push("Package Manager: yarn".to_string());
            }
        }
        if root.join("pom.xml").exists() || root.join("build.gradle").exists() {
            stack.push("Java (JVM)".to_string());
        }
        if root.join("go.mod").exists() {
            stack.push("Go (Go Modules)".to_string());
        }
        if root.join("pyproject.toml").exists() || root.join("requirements.txt").exists() {
            stack.push("Python".to_string());
        }
        if stack.is_empty() {
            stack.push("General Software Repository".to_string());
        }
        stack
    }

    fn detect_workflows(root: &Path) -> Vec<WorkflowCommand> {
        let mut workflows = Vec::new();
        if root.join("Cargo.toml").exists() {
            workflows.push(WorkflowCommand {
                name: "Build".to_string(),
                command: "cargo check --lib --bins".to_string(),
                description: "Fast compilation check without code generation".to_string(),
            });
            workflows.push(WorkflowCommand {
                name: "Test".to_string(),
                command: "cargo test --lib".to_string(),
                description: "Execute library test suite".to_string(),
            });
        } else if root.join("package.json").exists() {
            workflows.push(WorkflowCommand {
                name: "Dev".to_string(),
                command: "pnpm dev".to_string(),
                description: "Start local development server".to_string(),
            });
            workflows.push(WorkflowCommand {
                name: "Test".to_string(),
                command: "pnpm test".to_string(),
                description: "Run automated tests".to_string(),
            });
        } else {
            workflows.push(WorkflowCommand {
                name: "Inspect".to_string(),
                command: "git status".to_string(),
                description: "Check working directory status".to_string(),
            });
        }
        workflows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_onboarding_guide_generation() {
        let dir = tempfile::tempdir().unwrap();
        let cargo_toml = dir.path().join("Cargo.toml");
        std::fs::write(&cargo_toml, "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let main_rs = src_dir.join("main.rs");
        std::fs::write(&main_rs, "fn main() {}\n").unwrap();

        let guide = OnboardService::generate_guide(dir.path(), None).unwrap();
        assert!(guide.tech_stack.iter().any(|s| s.contains("Rust")));
        assert!(!guide.entry_points.is_empty());
        assert!(!guide.recommended_reading_order.is_empty());
        assert_eq!(guide.recommended_reading_order[0].target_file, "src/main.rs");
        assert!(!guide.development_workflows.is_empty());
    }
}

