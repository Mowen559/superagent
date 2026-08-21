//! 动态脚本工具装载器 (Dynamic Script Tool)
//! 允许从 json 配置载入外部脚本（Python, Node.js, Shell, Executable 等），
//! 无需重新编译 Rust 代码即可将外部技能热插拔注入到 AgentLoop 中。

use crate::services::brain::executor::ToolExecutor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 动态工具的 Schema 定义 (对应 `.tool.json`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolConfig {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// 可选的解释器，例如 "python", "node", "bash", "powershell"。如果不填，则直接执行 script_path
    pub interpreter: Option<String>,
    /// 要执行的脚本文件路径或可执行文件命令
    pub script_path: String,
    /// 额外参数
    pub extra_args: Option<Vec<String>>,
}

pub struct DynamicScriptTool {
    config: DynamicToolConfig,
    base_dir: PathBuf,
}

impl DynamicScriptTool {
    pub fn new(config: DynamicToolConfig, base_dir: PathBuf) -> Self {
        Self { config, base_dir }
    }

    /// 从指定的 json 文件载入动态工具
    pub fn load_from_file(json_path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(json_path)
            .map_err(|e| format!("Failed to read tool config at {:?}: {}", json_path, e))?;
        
        let config: DynamicToolConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse tool config JSON at {:?}: {}", json_path, e))?;

        let base_dir = json_path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        Ok(Self::new(config, base_dir))
    }
}

impl ToolExecutor for DynamicScriptTool {
    fn name(&self) -> &'static str {
        // 由于 Box<dyn ToolExecutor> 需要 &'static str，我们使用 Box::leak 产生 &'static str
        Box::leak(self.config.name.clone().into_boxed_str())
    }

    fn description(&self) -> &'static str {
        Box::leak(self.config.description.clone().into_boxed_str())
    }

    fn parameters_schema(&self) -> Value {
        self.config.parameters.clone()
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let args_json_str = serde_json::to_string(&args)
            .map_err(|e| format!("Failed to serialize tool arguments: {}", e))?;

        let script_full_path = if Path::new(&self.config.script_path).is_absolute() {
            PathBuf::from(&self.config.script_path)
        } else {
            self.base_dir.join(&self.config.script_path)
        };

        let mut cmd = if let Some(ref interp) = self.config.interpreter {
            let mut c = Command::new(interp);
            c.arg(&script_full_path);
            c
        } else {
            Command::new(&script_full_path)
        };

        if let Some(ref extra) = self.config.extra_args {
            for arg in extra {
                cmd.arg(arg);
            }
        }

        // 将 JSON 参数作为命令行最后一个参数传入
        cmd.arg(&args_json_str);

        // 设置工作目录为脚本所在的目录
        cmd.current_dir(&self.base_dir);

        let output = cmd.output().map_err(|e| {
            format!(
                "Failed to execute dynamic script tool '{}' (cmd: {:?}): {}",
                self.config.name, script_full_path, e
            )
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            return Err(format!(
                "Script exit code {:?}.\nStderr: {}\nStdout: {}",
                output.status.code(),
                stderr.trim(),
                stdout.trim()
            ));
        }

        // 优先解析 stdout 为 JSON 对象，解析失败则包裹为 text string
        if let Ok(json_val) = serde_json::from_str::<Value>(&stdout) {
            Ok(json_val)
        } else {
            Ok(serde_json::json!({
                "output": stdout.trim(),
                "stderr": stderr.trim()
            }))
        }
    }
}

/// 扫描目录中的所有 `.tool.json` 配置文件并载入为 DynamicScriptTool Vec
pub fn scan_dynamic_tools(dir: &Path) -> Vec<DynamicScriptTool> {
    let mut tools = Vec::new();
    if !dir.exists() || !dir.is_dir() {
        return tools;
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".tool.json") {
                        match DynamicScriptTool::load_from_file(&path) {
                            Ok(tool) => {
                                eprintln!("[DynamicTool] Successfully loaded dynamic tool '{}' from {:?}", tool.config.name, path);
                                tools.push(tool);
                            }
                            Err(e) => {
                                eprintln!("[DynamicTool Error] Failed to load tool from {:?}: {}", path, e);
                            }
                        }
                    }
                }
            } else if path.is_dir() {
                // 递归扫描一层子目录
                tools.extend(scan_dynamic_tools(&path));
            }
        }
    }

    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn test_dynamic_tool_scan_and_execution() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path();

        let script_path = dir_path.join("echo_tool.py");
        fs::write(
            &script_path,
            r#"import sys, json
args = json.loads(sys.argv[1])
print(json.dumps({"received": args, "status": "ok"}))
"#,
        )
        .unwrap();

        let tool_config_path = dir_path.join("echo.tool.json");
        let tool_json = json!({
            "name": "dynamic_echo",
            "description": "Test dynamic python tool",
            "parameters": {
                "type": "object",
                "properties": {
                    "msg": { "type": "string" }
                }
            },
            "interpreter": "python",
            "script_path": "echo_tool.py"
        });
        fs::write(&tool_config_path, tool_json.to_string()).unwrap();

        let tools = scan_dynamic_tools(dir_path);
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.name(), "dynamic_echo");

        let res = tool.execute(json!({"msg": "hello world"})).unwrap();
        assert_eq!(res["status"], "ok");
        assert_eq!(res["received"]["msg"], "hello world");
    }

    #[test]
    fn test_actual_dynamic_greet_plugin() {
        let cwd = std::env::current_dir().unwrap();
        let dynamic_dir = cwd.join("dynamic_tools");
        assert!(dynamic_dir.exists(), "dynamic_tools directory should exist");
        
        let tools = scan_dynamic_tools(&dynamic_dir);
        let greet_tool = tools.iter().find(|t| t.name() == "dynamic_greet");
        assert!(greet_tool.is_some(), "dynamic_greet tool should be loaded");
        
        let tool = greet_tool.unwrap();
        let res = tool.execute(json!({"name": "SuperAgent"})).unwrap();
        assert_eq!(res["status"], "success");
        assert_eq!(res["message"], "Hello SuperAgent, this is a message from the newly created dynamic Python plugin!");
    }
}
