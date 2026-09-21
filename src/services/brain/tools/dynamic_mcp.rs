//! 动态远程 HTTP MCP 工具桥接中枢 (Dynamic HTTP MCP Tool Bridge)
//!
//! 允许外部微服务（如 Java Spring Boot、Python FastAPI）在运行时通过 HTTP 注册自身业务工具，
//! 智能体大脑在推理时能够直接将该工具暴露给大模型，并在模型调用时自动向微服务回调执行。

use crate::domains::agent_cognitive::executor::ToolExecutor;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

lazy_static! {
    /// 全局动态远程 MCP 工具注册表
    pub static ref GLOBAL_DYNAMIC_MCP_REGISTRY: Arc<Mutex<DynamicMcpToolRegistry>> =
        Arc::new(Mutex::new(DynamicMcpToolRegistry::new()));
}

/// 远程业务工具定义元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteMcpToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub require_audit: bool,
    #[serde(default)]
    pub input_schema: Value,
    pub endpoint_url: String,
    pub service_name: String,
}

/// 动态 HTTP 远程 MCP 工具执行器 (实现 ToolExecutor 特征)
pub struct HttpRemoteMcpTool {
    pub def: RemoteMcpToolDefinition,
}

impl HttpRemoteMcpTool {
    pub fn new(def: RemoteMcpToolDefinition) -> Self {
        Self { def }
    }
}

impl ToolExecutor for HttpRemoteMcpTool {
    fn name(&self) -> &'static str {
        Box::leak(self.def.name.clone().into_boxed_str())
    }

    fn description(&self) -> &'static str {
        Box::leak(self.def.description.clone().into_boxed_str())
    }

    fn parameters_schema(&self) -> Value {
        if self.def.input_schema.is_object() {
            self.def.input_schema.clone()
        } else {
            json!({
                "type": "object",
                "properties": {}
            })
        }
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let endpoint = &self.def.endpoint_url;
        tracing::info!(
            "[DynamicMCP] 转发调用远程业务工具: tool='{}', service='{}', endpoint='{}'",
            self.def.name,
            self.def.service_name,
            endpoint
        );

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| format!("构建 HTTP 客户端失败: {}", e))?;

        let payload = json!({
            "tool": self.def.name,
            "arguments": args
        });

        let resp = client
            .post(endpoint)
            .json(&payload)
            .send()
            .map_err(|e| format!("调用远程微服务工具端点 [{}] 失败: {}", endpoint, e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "远程工具端点返回错误 HTTP 状态码: {} ({})",
                resp.status(),
                endpoint
            ));
        }

        let resp_json: Value = resp
            .json()
            .map_err(|e| format!("解析远程工具端点响应 JSON 失败: {}", e))?;

        Ok(resp_json)
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(format!(
            "[Remote Tool: {} ({})] {}",
            self.def.name, self.def.service_name, self.def.description
        ))
    }
}

/// 动态 MCP 工具注册表管理类
#[derive(Default)]
pub struct DynamicMcpToolRegistry {
    tools: HashMap<String, RemoteMcpToolDefinition>,
}

impl DynamicMcpToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// 注册某个外部微服务批量上报的工具列表
    pub fn register_service_tools(
        &mut self,
        service_name: &str,
        endpoint_url: &str,
        raw_tools: Vec<Value>,
    ) -> usize {
        let mut count = 0;
        for raw in raw_tools {
            let name = match raw.get("name").and_then(|v| v.as_str()) {
                Some(n) if !n.trim().is_empty() => n.trim().to_string(),
                _ => continue,
            };

            let description = raw
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let require_audit = raw
                .get("requireAudit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let input_schema = raw
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object" }));

            let def = RemoteMcpToolDefinition {
                name: name.clone(),
                description,
                require_audit,
                input_schema,
                endpoint_url: endpoint_url.to_string(),
                service_name: service_name.to_string(),
            };

            self.tools.insert(name.clone(), def);
            count += 1;
            tracing::info!(
                "[DynamicMCP] 成功挂载微服务远程工具: {} (服务: {})",
                name,
                service_name
            );
        }
        count
    }

    /// 获取所有已挂载的远程工具列表
    pub fn get_all_tools(&self) -> Vec<RemoteMcpToolDefinition> {
        self.tools.values().cloned().collect()
    }

    /// 移除某个微服务的所有已注册工具
    pub fn remove_service_tools(&mut self, service_name: &str) -> usize {
        let before = self.tools.len();
        self.tools.retain(|_, def| def.service_name != service_name);
        before - self.tools.len()
    }

    /// 实例化所有远程工具为 ToolExecutor 集合，供注入 AgentLoop
    pub fn create_tool_executors(&self) -> Vec<Box<dyn ToolExecutor>> {
        self.tools
            .values()
            .map(|def| Box::new(HttpRemoteMcpTool::new(def.clone())) as Box<dyn ToolExecutor>)
            .collect()
    }

    /// 直接按名称执行已注册的工具
    pub fn invoke_tool(&self, name: &str, args: Value) -> Result<Value, String> {
        let def = self
            .tools
            .get(name)
            .ok_or_else(|| format!("未找到名称为 '{}' 的远程 MCP 工具", name))?;

        let tool = HttpRemoteMcpTool::new(def.clone());
        tool.execute(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_mcp_registry_and_schema() {
        let mut registry = DynamicMcpToolRegistry::new();
        let tools = vec![json!({
            "name": "crawl_sample",
            "description": "抓取目标网页样本数据",
            "requireAudit": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string" }
                },
                "required": ["url"]
            }
        })];

        let count = registry.register_service_tools("engine-service", "http://127.0.0.1:8080/invoke", tools);
        assert_eq!(count, 1);

        let list = registry.get_all_tools();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "crawl_sample");
        assert_eq!(list[0].service_name, "engine-service");

        let executors = registry.create_tool_executors();
        assert_eq!(executors.len(), 1);
        assert_eq!(executors[0].name(), "crawl_sample");
        assert_eq!(executors[0].description(), "抓取目标网页样本数据");
    }
}
