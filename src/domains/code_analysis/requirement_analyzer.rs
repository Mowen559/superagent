use crate::domains::model_gateway::providers::LlmProvider;
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::Path;
use tracing::{info, warn};

pub struct RequirementAnalyzer {
    llm: LlmProvider,
}

impl RequirementAnalyzer {
    pub fn new(llm: LlmProvider) -> Self {
        Self { llm }
    }

    /// 分析需求，寻找改动入口点并推演影响面
    pub async fn analyze_requirement(
        &self,
        workspace_root: &Path,
        db_path: &str,
        requirement: &str,
        reporter: Option<std::sync::Arc<dyn crate::services::progress::ProgressReporter>>,
    ) -> Result<()> {
        info!("RequirementAnalyzer: Starting requirement impact prediction");

        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Ok(());
        }

        if let Some(r) = &reporter {
            r.report(
                "Fetching file context from DB",
                crate::services::progress::StepStatus::InProgress,
            );
        }

        let mut file_context = String::new();
        let mut project_language = "unknown".to_string();
        {
            project_language =
                crate::domains::knowledge_graph::understand_service::detect_primary_language(db_path)
                    .unwrap_or_else(|_| "unknown".to_string());
            let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .context("Failed to open graph DB for requirement analysis")?;

            let mut stmt = conn.prepare(
                "SELECT name, COALESCE(summary, '') FROM nodes WHERE label = 'File' LIMIT 100",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let name: String = row.get(0)?;
                let summary: String = row.get(1)?;
                file_context.push_str(&format!("{}: {}\n", name, summary));
            }
        } // conn and stmt are dropped here

        if let Some(r) = &reporter {
            r.report(
                "Fetching file context from DB",
                crate::services::progress::StepStatus::Completed,
            );
        }

        info!("Sending requirement to LLM to predict entry points...");
        if let Some(r) = &reporter {
            r.report(
                "Predicting entry points via LLM",
                crate::services::progress::StepStatus::InProgress,
            );
        }

        // 2. 询问 LLM 拟修改的入口文件
        let entry_points_resp = self
            .llm
            .predict_requirement_entry(requirement, &file_context, "zh-CN", &project_language)
            .await
            .unwrap_or_else(|e| format!("LLM Error: {}", e));

        info!("Predicted Entry Points:\n{}", entry_points_resp);

        // 3. 从 entry_points_resp 提取具体文件路径，并在 Graph DB 中真实推演受影响的调用方节点
        let mut impacted_analysis_md = String::new();
        if let Ok(conn) = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            let candidate_files: Vec<String> = entry_points_resp
                .split(|c: char| {
                    c.is_whitespace()
                        || c == '`'
                        || c == '"'
                        || c == '\''
                        || c == '('
                        || c == ')'
                        || c == '['
                        || c == ']'
                })
                .filter(|token| {
                    token.ends_with(".rs")
                        || token.ends_with(".java")
                        || token.ends_with(".ts")
                        || token.ends_with(".tsx")
                        || token.ends_with(".py")
                        || token.ends_with(".go")
                        || token.ends_with(".js")
                })
                .map(|s| s.trim_start_matches("./").replace('\\', "/"))
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            if !candidate_files.is_empty() {
                impacted_analysis_md.push_str("\n\n### 代码知识图谱受影响节点推演 (Graph Blast Radius)\n");
                let mut total_impacted_count = 0;

                for file in &candidate_files {
                    let like_pattern = format!("%{}%", file);
                    let node_stmt = conn.prepare(
                        "SELECT id, name, label FROM nodes WHERE file_path LIKE ?1 LIMIT 50",
                    );
                    if let Ok(mut stmt) = node_stmt {
                        let nodes: Vec<(i64, String, String)> = stmt
                            .query_map([&like_pattern], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                            .map(|mapped| mapped.filter_map(|r| r.ok()).collect())
                            .unwrap_or_default();

                        if !nodes.is_empty() {
                            let id_list = nodes
                                .iter()
                                .map(|(id, _, _)| id.to_string())
                                .collect::<Vec<_>>()
                                .join(",");
                            let edge_sql = format!(
                                "SELECT DISTINCT source_id FROM edges WHERE target_id IN ({}) LIMIT 50",
                                id_list
                            );
                            if let Ok(mut edge_stmt) = conn.prepare(&edge_sql) {
                                let source_ids: Vec<i64> = edge_stmt
                                    .query_map([], |r| r.get(0))
                                    .map(|mapped| mapped.filter_map(|r| r.ok()).collect())
                                    .unwrap_or_default();

                                if !source_ids.is_empty() {
                                    let s_id_list = source_ids
                                        .iter()
                                        .map(|id| id.to_string())
                                        .collect::<Vec<_>>()
                                        .join(",");
                                    let caller_sql = format!(
                                        "SELECT name, label, file_path FROM nodes WHERE id IN ({}) LIMIT 30",
                                        s_id_list
                                    );
                                    if let Ok(mut caller_stmt) = conn.prepare(&caller_sql) {
                                        let callers: Vec<(String, String, String)> = caller_stmt
                                            .query_map([], |r| {
                                                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                                            })
                                            .map(|mapped| mapped.filter_map(|r| r.ok()).collect())
                                            .unwrap_or_default();

                                        if !callers.is_empty() {
                                            total_impacted_count += callers.len();
                                            impacted_analysis_md.push_str(&format!(
                                                "- **入口源文件**: `{}` (包含 {} 个符号)\n",
                                                file,
                                                nodes.len()
                                            ));
                                            impacted_analysis_md
                                                .push_str("  - **直接下游调用方/影响节点**:\n");
                                            for (c_name, c_label, c_file) in callers {
                                                impacted_analysis_md.push_str(&format!(
                                                    "    - `[{}] {}` (文件: `{}`)\n",
                                                    c_label, c_name, c_file
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if total_impacted_count == 0 {
                    impacted_analysis_md.push_str("- 未检测到已索引的直接下游反向调用依赖（此入口文件属于高层入口或独立模块）。\n");
                }
            }
        }

        // 4. 持久化分析结果到工作区
        if let Some(r) = &reporter {
            r.report(
                "Predicting entry points via LLM",
                crate::services::progress::StepStatus::Completed,
            );
        }
        if let Some(r) = &reporter {
            r.report(
                "Saving impact report",
                crate::services::progress::StepStatus::InProgress,
            );
        }

        let report_dir = workspace_root.join(".deepcloud");
        if !report_dir.exists() {
            fs::create_dir_all(&report_dir)?;
        }
        let report_path = report_dir.join("requirement_impact.md");

        let lang = match project_language.to_lowercase().as_str() {
            "java" => crate::services::language_rules::SupportedLanguage::Java,
            "go" | "golang" => crate::services::language_rules::SupportedLanguage::Go,
            "rust" => crate::services::language_rules::SupportedLanguage::Rust,
            "typescript" | "ts" => crate::services::language_rules::SupportedLanguage::TypeScript,
            "javascript" | "js" => crate::services::language_rules::SupportedLanguage::JavaScript,
            "python" | "py" => crate::services::language_rules::SupportedLanguage::Python,
            _ => crate::services::language_rules::SupportedLanguage::Unknown,
        };
        let rules_checklist =
            if lang != crate::services::language_rules::SupportedLanguage::Unknown {
                let rules =
                    crate::services::language_rules::BuiltinRuleset::get_rules_for_language(lang);
                let mut buf =
                    String::from("\n\n## 实施与验收规范红线 (Deterministic Quality Checklist)\n");
                for r in rules {
                    buf.push_str(&format!(
                        "- **[{}] {}**: {}\n",
                        r.category.display_name(),
                        r.title,
                        r.description
                    ));
                }
                buf
            } else {
                String::new()
            };

        let report_content = format!(
            "# 需求逆向推演报告\n\n## 原始需求\n{}\n\n## 拟修改入口与影响面分析\n{}{}{}\n\n> 本报告由 DeepCloud 逆向推演引擎基于当前代码图谱自动生成。",
            requirement, entry_points_resp, impacted_analysis_md, rules_checklist
        );

        fs::write(&report_path, report_content)?;
        info!("Saved requirement impact report to {:?}", report_path);

        if let Some(r) = &reporter {
            r.report(
                "Saving impact report",
                crate::services::progress::StepStatus::Completed,
            );
        }

        Ok(())
    }

    /// 规划需求，输出用于后续 Agent 执行的结构化 Checklist (JSON)
    pub async fn plan_requirement(
        &self,
        workspace_root: &Path,
        db_path: &str,
        requirement: &str,
        reporter: Option<std::sync::Arc<dyn crate::services::progress::ProgressReporter>>,
    ) -> Result<()> {
        info!("RequirementAnalyzer: Starting requirement planning (Planner Mode)");

        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Ok(());
        }

        if let Some(r) = &reporter {
            r.report(
                "Fetching file context from DB",
                crate::services::progress::StepStatus::InProgress,
            );
        }

        let mut file_context = String::new();
        {
            let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .context("Failed to open graph DB for requirement planning")?;

            let mut stmt = conn.prepare(
                "SELECT name, COALESCE(summary, '') FROM nodes WHERE label = 'File' LIMIT 100",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let name: String = row.get(0)?;
                let summary: String = row.get(1)?;
                file_context.push_str(&format!("{}: {}\n", name, summary));
            }
        }

        if let Some(r) = &reporter {
            r.report(
                "Fetching file context from DB",
                crate::services::progress::StepStatus::Completed,
            );
        }

        info!("Sending requirement to LLM for Execution Plan generation...");
        if let Some(r) = &reporter {
            r.report(
                "Generating execution plan via LLM",
                crate::services::progress::StepStatus::InProgress,
            );
        }

        let mut system_prompt = r#"你是一个架构规划引擎 (Planner)。
你负责接收用户的自然语言需求，结合当前代码库的上下文信息，推演出一份详尽的修改建议，并严格遵守以下 JSON 格式。
不要使用任何 Markdown 代码块包裹，也不要返回任何解释性文本，必须且只能返回合法的 JSON 数组，数组里的每一项代表一个受影响的文件及其修改指引。"#.to_string();

        if let Some(instructions) =
            crate::services::project_context::load_project_instructions(Some(workspace_root))
        {
            system_prompt.push_str(&format!(
                "\n\n[Project Instructions from .deepcloud.md]:\n{}",
                instructions
            ));
        }

        let messages = vec![serde_json::json!({
            "role": "user",
            "content": format!(
                "需求描述:\n{}\n\n代码库关键文件上下文:\n{}\n\n请输出执行计划。",
                requirement, file_context
            )
        })];

        let (plan_resp, _) = self
            .llm
            .chat(messages, Some(&system_prompt))
            .await
            .map_err(|e| anyhow::anyhow!("LLM Plan Engine Error: {}", e))?;

        let cleaned_json = crate::domains::model_gateway::providers::strip_code_fences(&plan_resp);

        if let Some(r) = &reporter {
            r.report(
                "Generating execution plan via LLM",
                crate::services::progress::StepStatus::Completed,
            );
        }
        if let Some(r) = &reporter {
            r.report(
                "Saving execution plan",
                crate::services::progress::StepStatus::InProgress,
            );
        }

        let plan_dir = workspace_root.join(".deepcloud");
        if !plan_dir.exists() {
            fs::create_dir_all(&plan_dir)?;
        }
        let plan_path = plan_dir.join("execution_plan.json");

        fs::write(&plan_path, cleaned_json)?;
        info!("Saved execution plan to {:?}", plan_path);

        if let Some(r) = &reporter {
            r.report(
                "Saving execution plan",
                crate::services::progress::StepStatus::Completed,
            );
        }

        Ok(())
    }
}
