use crate::services::brain::eval_harness::{run_golden_benchmark, verify_workspace_syntax};
use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};

/// 黄金架构基准评测工具 (Golden Benchmark Eval Tool)
pub struct GoldenBenchmarkTool;

impl ToolExecutor for GoldenBenchmarkTool {
    fn name(&self) -> &'static str {
        "run_benchmark"
    }

    fn description(&self) -> &'static str {
        "Run the DeepCloud Golden Benchmark Evaluation Harness to evaluate and verify code against architectural rules (Pass@1 rate, required patterns, forbidden JPA/anti-patterns, and timing metrics)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Optional keyword or domain/case ID to filter which benchmark cases to run (e.g. 'controller', 'mybatis', 'spider_job', 'frontend', 'liteflow')."
                },
                "case_id": {
                    "type": "string",
                    "description": "Optional specific case ID to evaluate custom_code against."
                },
                "custom_code": {
                    "type": "string",
                    "description": "Optional code snippet to evaluate against the specified case instead of the default mock code."
                },
                "verify_workspace": {
                    "type": "boolean",
                    "description": "Optional flag to trigger dynamic project health & syntax verification for the workspace."
                }
            }
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: run_benchmark]
- Role: Conformance & Eval Harness (架构黄金基准评测机)
- Usage: Call `run_benchmark` to objectively evaluate code against DeepCloud ironclad rules (Pass@1 rate, forbidden JPA/Map returns, DTO annotations, LiteFlow EL specs).
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let filter = args.get("filter").and_then(|v| v.as_str());
        let case_id = args.get("case_id").and_then(|v| v.as_str());
        let custom_code = args.get("custom_code").and_then(|v| v.as_str());
        let verify_workspace = args.get("verify_workspace").and_then(|v| v.as_bool()).unwrap_or(false);

        let custom_target = match (case_id, custom_code) {
            (Some(cid), Some(code)) => Some((cid, code)),
            _ => None,
        };

        let report = run_golden_benchmark(filter, custom_target);
        let mut report_val = serde_json::to_value(&report).map_err(|e| e.to_string())?;

        if verify_workspace {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let ws_status = verify_workspace_syntax(&cwd);
            report_val["workspace_verification"] = match ws_status {
                Ok(msg) => json!({ "status": "passed", "message": msg }),
                Err(err) => json!({ "status": "failed", "error": err }),
            };
        }

        Ok(report_val)
    }
}
