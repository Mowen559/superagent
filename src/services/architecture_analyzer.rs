use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use tracing::{info, warn};

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerItem {
    pub name: String,
    pub description: String,
    pub files: Vec<String>,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerAnalysisReport {
    pub layers: Vec<LayerItem>,
    pub total_files: usize,
    pub breakdown: BTreeMap<String, usize>,
}

pub struct ArchitectureAnalyzer {
    llm: Option<crate::domains::model_gateway::providers::LlmProvider>,
}

impl ArchitectureAnalyzer {
    pub fn new(llm: crate::domains::model_gateway::providers::LlmProvider) -> Self {
        Self { llm: Some(llm) }
    }

    pub fn without_llm() -> Self {
        Self { llm: None }
    }

    /// 确定性架构分层检测：基于文件路径模式与命名约定分类，无需 LLM 介入，毫秒级完成
    pub fn detect_layers_deterministic(db_path: &str) -> Result<LayerAnalysisReport> {
        if !Path::new(db_path).exists() {
            return Err(anyhow::anyhow!("Graph DB not found: {}", db_path));
        }

        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("Failed to open graph DB for layer detection")?;

        let mut stmt = conn
            .prepare("SELECT file_path FROM nodes WHERE label = 'File' AND file_path <> '' ORDER BY file_path")?;
        let file_paths: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut layers_map: HashMap<&'static str, Vec<String>> = HashMap::new();

        for path in file_paths {
            let p_lower = path.replace('\\', "/").to_lowercase();
            let layer = Self::classify_file_layer(&p_lower);
            layers_map.entry(layer).or_default().push(path);
        }

        let layer_order = [
            ("API", "接入与接口层 (Controllers, Routes, Endpoints, Interfaces)"),
            ("Service", "应用与业务逻辑层 (Services, UseCases, Handlers)"),
            ("Domain", "领域与核心模型层 (Domain, Models, Entities, Core)"),
            ("Data", "数据持久化与仓储层 (Repositories, DAO, Mappers, Store)"),
            ("Infrastructure", "基础设施与配置层 (Config, Adapters, Infrastructure)"),
            ("UI", "用户界面与前端呈现层 (UI, Views, Components, Pages)"),
            ("Utility", "通用与工具支持层 (Utils, Helpers, Common, Shared)"),
            ("Other", "未分类/其他辅助模块"),
        ];

        let mut result_layers = Vec::new();
        let mut breakdown = BTreeMap::new();
        let mut total_files = 0;

        for (name, desc) in layer_order {
            if let Some(files) = layers_map.remove(name) {
                let count = files.len();
                total_files += count;
                breakdown.insert(name.to_string(), count);
                result_layers.push(LayerItem {
                    name: name.to_string(),
                    description: desc.to_string(),
                    files,
                    file_count: count,
                });
            }
        }

        Ok(LayerAnalysisReport {
            layers: result_layers,
            total_files,
            breakdown,
        })
    }

    fn classify_file_layer(path: &str) -> &'static str {
        if path.contains("/api/") || path.contains("/controllers/") || path.contains("/controller/")
            || path.contains("/routes/") || path.contains("/route/") || path.contains("/endpoints/")
            || path.contains("/endpoint/") || path.contains("/interfaces/") || path.contains("/graphql/")
            || path.contains("/grpc/")
        {
            "API"
        } else if path.contains("/services/") || path.contains("/service/") || path.contains("/application/")
            || path.contains("/app/") || path.contains("/usecases/") || path.contains("/usecase/")
            || path.contains("/handlers/") || path.contains("/handler/")
        {
            "Service"
        } else if path.contains("/domain/") || path.contains("/models/") || path.contains("/model/")
            || path.contains("/entities/") || path.contains("/entity/") || path.contains("/core/")
        {
            "Domain"
        } else if path.contains("/dao/") || path.contains("/repositories/") || path.contains("/repository/")
            || path.contains("/repo/") || path.contains("/mappers/") || path.contains("/mapper/")
            || path.contains("/database/") || path.contains("/db/") || path.contains("/store/")
            || path.contains("/persistence/")
        {
            "Data"
        } else if path.contains("/ui/") || path.contains("/views/") || path.contains("/view/")
            || path.contains("/components/") || path.contains("/component/") || path.contains("/web/")
            || path.contains("/frontend/") || path.contains("/client/") || path.contains("/pages/")
        {
            "UI"
        } else if path.contains("/utils/") || path.contains("/util/") || path.contains("/helpers/")
            || path.contains("/helper/") || path.contains("/common/") || path.contains("/shared/")
            || path.contains("/tools/")
        {
            "Utility"
        } else if path.contains("/config/") || path.contains("/infra/") || path.contains("/infrastructure/")
            || path.contains("/adapter/") || path.contains("/adapters/") || path.contains("/boot/")
        {
            "Infrastructure"
        } else {
            "Other"
        }
    }

    /// 从 Graph DB 提取节点集合，分组并发给大模型进行架构分层（如 Service, Repository, UI）
    pub async fn analyze_layers(&self, db_path: &str, lang: Option<&str>) -> Result<String> {
        info!("ArchitectureAnalyzer: Starting layer analysis");

        if !Path::new(db_path).exists() {
            warn!("Graph DB not found at {:?}", db_path);
            return Err(anyhow::anyhow!("Graph DB not found"));
        }

        // 先获取确定性分层报告作为基底
        let deterministic = Self::detect_layers_deterministic(db_path).unwrap_or(LayerAnalysisReport {
            layers: vec![],
            total_files: 0,
            breakdown: BTreeMap::new(),
        });

        let file_nodes = {
            let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .context("Failed to open graph DB for architecture analysis")?;

            // 提取主要代码文件节点
            let mut stmt = conn
                .prepare("SELECT id, name, file_path FROM nodes WHERE label = 'File' LIMIT 200")?;
            let files: Vec<(i64, String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2).unwrap_or_default(),
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            info!(
                "ArchitectureAnalyzer: Found {} file nodes for layer classification.",
                files.len()
            );
            files
                .iter()
                .map(|f| format!("{} ({})", f.1, f.2))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let lang_str = lang.unwrap_or("zh-CN");

        if let Some(llm) = &self.llm {
            let report = llm
                .explain_architecture(&file_nodes, lang_str)
                .await
                .map_err(|e| anyhow::anyhow!("LLM Error: {}", e))?;
            Ok(report)
        } else {
            // 离线/零 LLM 模式下返回确定性分层格式化 Markdown
            let mut report = String::from("# 架构分层分析报告 (确定性规则引擎)\n\n");
            report.push_str(&format!("共分析 {} 个文件，各层级分布如下：\n\n", deterministic.total_files));
            for layer in deterministic.layers {
                report.push_str(&format!("### {} ({})\n", layer.name, layer.file_count));
                report.push_str(&format!("{}\n\n", layer.description));
                for f in layer.files.iter().take(10) {
                    report.push_str(&format!("- `{}`\n", f));
                }
                if layer.files.len() > 10 {
                    report.push_str(&format!("- *(及其他 {} 个文件)*\n", layer.files.len() - 10));
                }
                report.push_str("\n");
            }
            Ok(report)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_layer_detection() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("graph.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                label TEXT NOT NULL,
                file_path TEXT NOT NULL
            );
            INSERT INTO nodes VALUES (1, 'File', 'src/api/user_controller.rs');
            INSERT INTO nodes VALUES (2, 'File', 'src/services/user_service.rs');
            INSERT INTO nodes VALUES (3, 'File', 'src/domain/user.rs');
            INSERT INTO nodes VALUES (4, 'File', 'src/repositories/user_repo.rs');
            INSERT INTO nodes VALUES (5, 'File', 'src/ui/components/avatar.tsx');
            INSERT INTO nodes VALUES (6, 'File', 'src/utils/crypto.rs');
            INSERT INTO nodes VALUES (7, 'File', 'src/config/app_config.rs');
            INSERT INTO nodes VALUES (8, 'File', 'README.md');
            "#,
        )
        .unwrap();
        drop(conn);

        let report = ArchitectureAnalyzer::detect_layers_deterministic(db_path.to_str().unwrap()).unwrap();
        assert_eq!(report.total_files, 8);
        assert_eq!(report.breakdown.get("API"), Some(&1));
        assert_eq!(report.breakdown.get("Service"), Some(&1));
        assert_eq!(report.breakdown.get("Domain"), Some(&1));
        assert_eq!(report.breakdown.get("Data"), Some(&1));
        assert_eq!(report.breakdown.get("UI"), Some(&1));
        assert_eq!(report.breakdown.get("Utility"), Some(&1));
        assert_eq!(report.breakdown.get("Infrastructure"), Some(&1));
        assert_eq!(report.breakdown.get("Other"), Some(&1));
    }
}
