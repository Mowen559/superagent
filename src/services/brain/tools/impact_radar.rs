use crate::services::brain::executor::ToolExecutor;
use crate::services::diff_impact::DiffImpactAnalyzer;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 改前调用图谱波及面雷达工具 (Impact Radar Tool)
pub struct ImpactRadarTool {
    pub workspace_root: PathBuf,
}

impl ImpactRadarTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

impl ToolExecutor for ImpactRadarTool {
    fn name(&self) -> &'static str {
        "impact_radar"
    }

    fn description(&self) -> &'static str {
        "Inspect the call-graph impact radius and external callers of a function, class, or symbol across the codebase before modifying it. Use this to prevent breaking external modules and coordinate refactoring."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "The function name, method name, class, struct, or symbol identifier to analyze."
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional file path where the symbol is defined (to exclude local self-references)."
                }
            },
            "required": ["symbol"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: impact_radar]
- Role: Pre-edit Safety Sensor (改前防损雷达)
- Usage: Call `impact_radar` before modifying any public function, class, or interface to identify all referencing callers and avoid breaking downstream contracts.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let symbol = args
            .get("symbol")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'symbol' parameter")?;

        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(std::path::Path::new);

        let app_paths = crate::config::app_paths::AppPaths::new();
        let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
        let cwd_str = self.workspace_root.to_string_lossy().to_string();
        let db_path = config.codebase_memory_db_path(Some(&cwd_str));
        let db_path_str = db_path.to_string_lossy().to_string();

        let radar = DiffImpactAnalyzer::analyze_symbol_impact(
            &self.workspace_root,
            &db_path_str,
            symbol,
            file_path,
        )?;

        Ok(serde_json::to_value(radar).map_err(|e| e.to_string())?)
    }
}
