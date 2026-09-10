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
                crate::services::understand_service::detect_primary_language(db_path)
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

        // 3. TODO: 这里应该从 entry_points_resp 提取具体的文件路径，通过图推演受影响的节点。
        // 为简化示例，我们直接将其作为推演报告的基础。

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

        let report_content = format!(
            "# 需求逆向推演报告\n\n## 原始需求\n{}\n\n## 拟修改入口与影响面分析\n{}\n\n> 本报告由 DeepCloud 逆向推演引擎基于当前代码图谱自动生成。",
            requirement, entry_points_resp
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
