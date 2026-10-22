use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use super::tool_retriever::ToolMetadata;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerSpec {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_transport() -> String {
    "stdio".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfigFile {
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerSpec>,
}

pub struct McpDiscoveryClient {
    registered_servers: Vec<McpServerSpec>,
}

impl McpDiscoveryClient {
    pub fn new() -> Self {
        Self {
            registered_servers: Vec::new(),
        }
    }

    /// 从本地配置文件或插件目录发现并加载所有声明的 MCP Server
    pub fn discover_from_file<P: AsRef<Path>>(&mut self, config_path: P) -> Result<usize, String> {
        let path = config_path.as_ref();
        if !path.exists() {
            return Ok(0);
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("读取 MCP 配置文件失败 [{}]: {}", path.display(), e))?;

        let parsed: McpConfigFile = serde_json::from_str(&content)
            .map_err(|e| format!("解析 MCP 配置文件 JSON 失败: {}", e))?;

        let mut count = 0;
        for (name, mut spec) in parsed.mcp_servers {
            if spec.name.is_empty() {
                spec.name = name;
            }
            if spec.enabled {
                self.registered_servers.push(spec);
                count += 1;
            }
        }
        Ok(count)
    }

    /// 扫描插件目录下的所有 deepcloud-plugin.json 或 manifest.json 并提取 mcp 声明
    pub fn discover_from_plugins_dir<P: AsRef<Path>>(&mut self, dir: P) -> Result<usize, String> {
        let p = dir.as_ref();
        if !p.exists() || !p.is_dir() {
            return Ok(0);
        }

        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                let sub_path = entry.path();
                let manifest_file = if sub_path.is_file() && sub_path.file_name().map_or(false, |n| n == "manifest.json" || n == "deepcloud-plugin.json") {
                    Some(sub_path)
                } else if sub_path.is_dir() {
                    let candidate1 = sub_path.join("manifest.json");
                    let candidate2 = sub_path.join("deepcloud-plugin.json");
                    if candidate1.exists() {
                        Some(candidate1)
                    } else if candidate2.exists() {
                        Some(candidate2)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(mf) = manifest_file {
                    if let Ok(content) = std::fs::read_to_string(&mf) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(mcp) = val.get("mcp").and_then(|v| v.get("server")) {
                                if let Ok(mut spec) = serde_json::from_value::<McpServerSpec>(mcp.clone()) {
                                    if spec.name.is_empty() {
                                        spec.name = val.get("id").and_then(|i| i.as_str()).unwrap_or("unnamed_mcp").to_string();
                                    }
                                    if spec.enabled {
                                        self.registered_servers.push(spec);
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(count)
    }

    /// 转换已发现的 Server 为 ToolMetadata 供检索与挂载
    pub fn to_tool_metadata(&self) -> Vec<ToolMetadata> {
        self.registered_servers
            .iter()
            .map(|s| ToolMetadata {
                name: format!("mcp_server_{}", s.name),
                description: format!("外部 MCP 服务 [{}]，由命令 `{}` 提供能力", s.name, s.command),
                is_core: false,
            })
            .collect()
    }

    pub fn get_registered_servers(&self) -> &[McpServerSpec] {
        &self.registered_servers
    }
}
