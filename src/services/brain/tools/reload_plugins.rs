//! 热重载插件工具 (Reload Plugins Tool)
//! 允许 Agent 在运行时动态扫描 `skills/plugins/` 或 `dynamic_tools/` 目录下的 `.tool.json`，
//! 将新的外部脚本工具热加载到内存中的工具列表中，无需重新编译 `.exe`！

use crate::services::brain::tools::dynamic_tool::scan_dynamic_tools;
use crate::services::brain::executor::ToolExecutor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct ReloadPluginsTool {
    tools_ref: Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>>,
}

impl ReloadPluginsTool {
    pub fn new(tools_ref: Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>>) -> Self {
        Self { tools_ref }
    }
}

impl ToolExecutor for ReloadPluginsTool {
    fn name(&self) -> &'static str {
        "reload_plugins"
    }

    fn description(&self) -> &'static str {
        "Scan and hot-reload dynamic tools from 'skills/plugins/' and 'dynamic_tools/' directories without recompiling or restarting."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_directory": {
                    "type": "string",
                    "description": "Optional specific directory path to scan for .tool.json configurations. Defaults to scanning workspace dynamic tool directories."
                }
            }
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: reload_plugins]
- Role: Evolution (自举进化)
- Usage: Dynamically load external Python/Node scripts without recompiling.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let mut dirs_to_scan = Vec::new();

        if let Some(target_dir) = args.get("target_directory").and_then(|v| v.as_str()) {
            if !target_dir.trim().is_empty() {
                dirs_to_scan.push(PathBuf::from(target_dir));
            }
        }

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        dirs_to_scan.push(cwd.join("dynamic_tools"));
        dirs_to_scan.push(cwd.join("skills").join("plugins"));
        dirs_to_scan.push(cwd.join(".dsh").join("skills"));

        let mut loaded_names = Vec::new();

        for dir in dirs_to_scan {
            if dir.exists() && dir.is_dir() {
                let dynamic_tools = scan_dynamic_tools(&dir);
                for tool in dynamic_tools {
                    let tool_name = tool.name().to_string();
                    let mut lock = self.tools_ref.lock().unwrap();
                    lock.insert(tool_name.clone(), Box::new(tool));
                    loaded_names.push(tool_name);
                }
            }
        }

        let lock = self.tools_ref.lock().unwrap();
        let all_tools: Vec<String> = lock.keys().cloned().collect();

        Ok(json!({
            "status": "success",
            "reloaded_tools": loaded_names,
            "reloaded_count": loaded_names.len(),
            "all_available_tools": all_tools,
            "total_count": all_tools.len()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reload_plugins_and_call_dynamic_greet() {
        let tools: Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>> = Arc::new(Mutex::new(HashMap::new()));
        let reload_tool = ReloadPluginsTool::new(tools.clone());

        // 1. 执行 reload_plugins 工具
        let reload_result = reload_tool.execute(json!({})).expect("Reload plugins failed");
        assert_eq!(reload_result["status"], "success");
        let reloaded_tools = reload_result["reloaded_tools"].as_array().expect("reloaded_tools should be array");
        assert!(reloaded_tools.iter().any(|t| t.as_str() == Some("dynamic_greet")));

        // 2. 从内存工具池中获取 dynamic_greet 并执行
        let lock = tools.lock().unwrap();
        let dynamic_greet_tool = lock.get("dynamic_greet").expect("dynamic_greet must be present in memory");
        
        let call_res = dynamic_greet_tool.execute(json!({ "name": "SuperAgent" })).expect("dynamic_greet execution failed");
        assert_eq!(call_res["status"], "success");
        assert_eq!(
            call_res["message"],
            "Hello SuperAgent, this is a message from the newly created dynamic Python plugin!"
        );
    }
}
