use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use crate::services::brain::executor::ToolExecutor;
use crate::services::brain::tools::dynamic_tool::scan_dynamic_tools;

pub struct PluginRegistry {
    tools: Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register(&self, tool: Box<dyn ToolExecutor>) {
        let mut guard = self.tools.lock().unwrap();
        guard.insert(tool.name().to_string(), tool);
    }

    pub fn get_tools_ref(&self) -> Arc<Mutex<HashMap<String, Box<dyn ToolExecutor>>>> {
        self.tools.clone()
    }

    /// 自动扫描动态插件/技能目录并挂载 (MCP Foundation)
    pub fn load_dynamic_plugins(&self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let dirs = vec![
            cwd.join("dynamic_tools"),
            cwd.join("skills").join("plugins"),
            cwd.join(".dsh").join("skills"),
        ];

        for dir in dirs {
            if dir.exists() && dir.is_dir() {
                let dynamic_tools = scan_dynamic_tools(&dir);
                for tool in dynamic_tools {
                    let tool_name = tool.name().to_string();
                    self.register(Box::new(tool));
                    eprintln!("[PluginRegistry] Hot-plugged dynamic tool (MCP): {}", tool_name);
                }
            }
        }
    }
}
