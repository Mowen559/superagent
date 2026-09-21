use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::brain::tools::dynamic_tool::{DynamicScriptTool, DynamicToolConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// OpenClaw 动态技能与工具自主合成器 (Autonomous Tool Synthesizer)
pub struct SynthesizeTool {
    tools_ref: Arc<Mutex<HashMap<String, Arc<dyn ToolExecutor>>>>,
    workspace_root: PathBuf,
}

impl SynthesizeTool {
    pub fn new(
        tools_ref: Arc<Mutex<HashMap<String, Arc<dyn ToolExecutor>>>>,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            tools_ref,
            workspace_root,
        }
    }
}

impl ToolExecutor for SynthesizeTool {
    fn name(&self) -> &'static str {
        "synthesize_tool"
    }

    fn description(&self) -> &'static str {
        "OpenClaw Autonomous Tool Synthesizer: Dynamically author, test, and hot-load a new executable script tool (Python, Node.js, Shell, PowerShell) into the agent's active tool belt without recompiling or restarting."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Unique identifier for the new tool in snake_case (e.g., 'extract_api_routes', 'parse_log_stack')."
                },
                "description": {
                    "type": "string",
                    "description": "Detailed explanation of what the tool does and instructions on parameter inputs."
                },
                "parameters": {
                    "type": "object",
                    "description": "Standard JSON schema object defining the parameters expected by this tool."
                },
                "interpreter": {
                    "type": "string",
                    "enum": ["python", "node", "powershell", "bash", "sh"],
                    "description": "The interpreter executable to run the script."
                },
                "script_content": {
                    "type": "string",
                    "description": "The full source code of the script to execute when this tool is invoked."
                }
            },
            "required": ["tool_name", "description", "parameters", "script_content"]
        })
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(r#"
[Tool Projection: synthesize_tool]
- Role: OpenClaw Dynamic Weaponry (工具自合成与能力扩充)
- Usage: When encountering repetitive, complex, or domain-specific tasks not covered by static tools, author a dedicated Python/Node/Shell script tool using `synthesize_tool`. The new tool is immediately hot-loaded into your active tool set without restarting.
"#.trim().to_string())
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let tool_name = args
            .get("tool_name")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'tool_name'")?
            .trim();

        if tool_name.is_empty()
            || tool_name.contains('/')
            || tool_name.contains('\\')
            || tool_name.contains("..")
        {
            return Err("Invalid 'tool_name': must be a clean identifier without path separators".to_string());
        }

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'description'")?;

        let parameters = args
            .get("parameters")
            .cloned()
            .ok_or("Missing required 'parameters' JSON schema")?;

        let interpreter = args
            .get("interpreter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let script_content = args
            .get("script_content")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'script_content'")?;

        // 确定脚本文件后缀
        let script_filename = match interpreter.as_deref() {
            Some("python") => "tool.py",
            Some("node") => "tool.js",
            Some("powershell") => "tool.ps1",
            _ => "tool.sh",
        };

        // 构造落地目录：workspace_root/dynamic_tools/<tool_name>/
        let tool_dir = self.workspace_root.join("dynamic_tools").join(tool_name);
        fs::create_dir_all(&tool_dir)
            .map_err(|e| format!("Failed to create tool directory {:?}: {}", tool_dir, e))?;

        let script_path = tool_dir.join(script_filename);
        fs::write(&script_path, script_content)
            .map_err(|e| format!("Failed to write script file {:?}: {}", script_path, e))?;

        let manifest_config = DynamicToolConfig {
            name: tool_name.to_string(),
            description: description.to_string(),
            parameters: parameters.clone(),
            interpreter: interpreter.clone(),
            script_path: script_filename.to_string(),
            extra_args: None,
            timeout_ms: 60_000,
            allow_external_path: false,
        };

        let manifest_path = tool_dir.join(".tool.json");
        let manifest_json = serde_json::to_string_pretty(&manifest_config)
            .map_err(|e| format!("Failed to serialize tool manifest: {}", e))?;
        fs::write(&manifest_path, manifest_json)
            .map_err(|e| format!("Failed to write manifest {:?}: {}", manifest_path, e))?;

        // 即刻热加载到激活的工具箱 (Hot-Load into active runtime)
        let dynamic_tool = DynamicScriptTool::new(manifest_config, tool_dir);
        let mut guard = self.tools_ref.lock().map_err(|_| "Failed to lock tools registry".to_string())?;
        guard.insert(tool_name.to_string(), Arc::new(dynamic_tool));

        eprintln!(
            "\x1b[32m[OpenClaw SynthesizeTool] Successfully created & hot-loaded tool: '{}'\x1b[0m",
            tool_name
        );

        Ok(json!({
            "status": "synthesized_and_loaded",
            "tool_name": tool_name,
            "manifest_path": manifest_path.to_string_lossy(),
            "interpreter": interpreter,
            "message": format!(
                "Tool '{}' has been successfully authored, registered, and hot-loaded into memory! You can call it immediately as '{}'.",
                tool_name, tool_name
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_synthesize_dynamic_tool() {
        let dir = tempdir().unwrap();
        let tools_map: Arc<Mutex<HashMap<String, Arc<dyn ToolExecutor>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let tool = SynthesizeTool::new(tools_map.clone(), dir.path().to_path_buf());

        let args = json!({
            "tool_name": "calc_add",
            "description": "Adds two numbers",
            "parameters": {
                "type": "object",
                "properties": {
                    "a": { "type": "number" },
                    "b": { "type": "number" }
                }
            },
            "interpreter": "python",
            "script_content": "import sys, json\nargs = json.loads(sys.argv[1]) if len(sys.argv) > 1 else {}\nprint(json.dumps({'res': args.get('a', 0) + args.get('b', 0)}))"
        });

        let res = tool.execute(args).unwrap();
        assert_eq!(res["status"], "synthesized_and_loaded");
        assert_eq!(res["tool_name"], "calc_add");

        let guard = tools_map.lock().unwrap();
        assert!(guard.contains_key("calc_add"));
        assert_eq!(guard.get("calc_add").unwrap().name(), "calc_add");
    }
}
