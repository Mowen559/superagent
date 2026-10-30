use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::tools::dynamic_tool::scan_dynamic_tools;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct PluginRegistry {
    tools: Arc<Mutex<HashMap<String, Arc<dyn ToolExecutor>>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        let registry = Self {
            tools: Arc::new(Mutex::new(HashMap::new())),
        };
        // 自动挂载由外部微服务注册的动态 MCP 工具
        let remote_tools = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
            .lock()
            .unwrap()
            .create_tool_executors();
        for tool in remote_tools {
            registry.register(tool);
        }
        registry
    }

    pub fn register(&self, tool: Box<dyn ToolExecutor>) {
        let mut guard = self.tools.lock().unwrap();
        guard.insert(tool.name().to_string(), Arc::from(tool));
    }

    pub fn get_tools_ref(&self) -> Arc<Mutex<HashMap<String, Arc<dyn ToolExecutor>>>> {
        self.tools.clone()
    }

    /// 自动扫描动态插件/技能目录并挂载 (MCP Foundation)
    pub fn load_dynamic_plugins(&self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.load_dynamic_plugins_from(&cwd);
    }

    pub fn load_dynamic_plugins_from(&self, cwd: &std::path::Path) {
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
                    eprintln!(
                        "[PluginRegistry] Hot-plugged dynamic tool (MCP): {}",
                        tool_name
                    );
                }
            }
        }

        // 同步挂载外部微服务注册的动态 MCP 远程工具
        let remote_tools = crate::services::brain::tools::dynamic_mcp::GLOBAL_DYNAMIC_MCP_REGISTRY
            .lock()
            .unwrap()
            .create_tool_executors();
        for tool in remote_tools {
            let tool_name = tool.name().to_string();
            self.register(tool);
            eprintln!("[PluginRegistry] Mounted dynamic remote MCP tool: {}", tool_name);
        }
    }
}
