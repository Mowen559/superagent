//! 脏 JSON 容错与自愈修复引擎 (Fuzzy Tool-Call JSON Repair Engine)
//!
//! 专为本地轻量级开源模型 (如 Ollama, Qwen2.5-Coder 7B/14B, DeepSeek-R1-Distill) 打造。
//! 针对小模型在脱机运行时常见的格式漂移、末尾截断、Markdown 外包与控制字符瑕疵，
//! 提供高鲁棒性的自动修复与内嵌工具提取能力。

use regex::Regex;
use serde_json::{json, Value};

/// 脏 JSON 修复器
pub struct FuzzyJsonRepair;

impl FuzzyJsonRepair {
    /// 尝试直接解析，若失败则进行渐进式启发自愈修复
    pub fn parse_or_repair(raw: &str) -> Result<Value, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(json!({}));
        }

        // 1. 若本身合法，直接返回
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            return Ok(val);
        }

        // 2. 剥离 Markdown 代码块 ```json ... ```
        let cleaned = Self::strip_markdown_code_block(trimmed);
        if let Ok(val) = serde_json::from_str::<Value>(&cleaned) {
            return Ok(val);
        }

        // 3. 启发式结构修复（闭合引号、补齐括号、去除悬挂逗号）
        let repaired = Self::repair_syntax_errors(&cleaned);
        if let Ok(val) = serde_json::from_str::<Value>(&repaired) {
            return Ok(val);
        }

        // 4. 深度修复：提取最外层大括号对象
        if let Some(extracted) = Self::extract_outermost_object(&repaired) {
            if let Ok(val) = serde_json::from_str::<Value>(&extracted) {
                return Ok(val);
            }
            let second_repaired = Self::repair_syntax_errors(&extracted);
            if let Ok(val) = serde_json::from_str::<Value>(&second_repaired) {
                return Ok(val);
            }
        }

        Err(format!("无法自愈修复的非法 JSON: {}", raw))
    }

    /// 剥离 Markdown 代码块标志
    pub fn strip_markdown_code_block(text: &str) -> String {
        let trimmed = text.trim();
        let re = Regex::new(r"(?s)```(?:json|JSON)?\s*(.*?)\s*```").unwrap();
        if let Some(caps) = re.captures(trimmed) {
            if let Some(m) = caps.get(1) {
                return m.as_str().trim().to_string();
            }
        }
        trimmed.to_string()
    }

    /// 修复常见的 JSON 语法错误
    pub fn repair_syntax_errors(input: &str) -> String {
        let mut s = input.trim().to_string();

        // 1. 替换中文标点符号为标准英文符号
        s = s.replace('“', "\"")
            .replace('”', "\"")
            .replace('：', ":")
            .replace('，', ",");

        // 2. 移除尾随逗号 (Trailing Commas): ", }" -> " }" 或 ", ]" -> " ]"
        let trailing_comma_obj = Regex::new(r",\s*\}").unwrap();
        s = trailing_comma_obj.replace_all(&s, "}").to_string();
        let trailing_comma_arr = Regex::new(r",\s*\]").unwrap();
        s = trailing_comma_arr.replace_all(&s, "]").to_string();

        // 3. 补齐未闭合的字符串双引号
        // 扫描是否单数个双引号（忽略转义 \"）
        let mut in_string = false;
        let mut chars = s.chars().peekable();
        let mut balanced = String::new();

        while let Some(c) = chars.next() {
            if c == '\\' {
                balanced.push(c);
                if let Some(next_c) = chars.next() {
                    balanced.push(next_c);
                }
                continue;
            }
            if c == '"' {
                in_string = !in_string;
            }
            balanced.push(c);
        }

        // 若双引号未闭合，补上一个双引号
        if in_string {
            balanced.push('"');
        }

        // 4. 补齐末尾缺失的闭合括号 } 或 ]
        let mut open_braces = 0i32;
        let mut open_brackets = 0i32;
        let mut in_str = false;
        let mut prev = '\0';

        for ch in balanced.chars() {
            if ch == '"' && prev != '\\' {
                in_str = !in_str;
            }
            if !in_str {
                match ch {
                    '{' => open_braces += 1,
                    '}' => {
                        if open_braces > 0 {
                            open_braces -= 1;
                        }
                    }
                    '[' => open_brackets += 1,
                    ']' => {
                        if open_brackets > 0 {
                            open_brackets -= 1;
                        }
                    }
                    _ => {}
                }
            }
            prev = ch;
        }

        let mut fixed = balanced;
        // 优先闭合中括号
        for _ in 0..open_brackets {
            fixed.push(']');
        }
        // 闭合大括号
        for _ in 0..open_braces {
            fixed.push('}');
        }

        fixed
    }

    /// 提取最外层大括号对象（如果前后包裹了分析性废话）
    pub fn extract_outermost_object(text: &str) -> Option<String> {
        let first_brace = text.find('{')?;
        let last_brace = text.rfind('}')?;
        if last_brace > first_brace {
            Some(text[first_brace..=last_brace].to_string())
        } else {
            None
        }
    }

    /// 针对未触发原生 tool_calls 的本地小模型：
    /// 自动从回复文本 (content) 中嗅探内嵌的工具调用 (Fallback Tool Call Extraction)
    pub fn extract_embedded_tool_call(content: &str) -> Option<(String, Value)> {
        let trimmed = content.trim();

        // 模式 1: 包含 <tool_call> ... </tool_call> 标签（如部分开源模型标准）
        let xml_re = Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap();
        if let Some(caps) = xml_re.captures(trimmed) {
            if let Some(m) = caps.get(1) {
                if let Ok(val) = Self::parse_or_repair(m.as_str()) {
                    if let Some(extracted) = Self::resolve_call_from_value(&val) {
                        return Some(extracted);
                    }
                }
            }
        }

        // 模式 2: Markdown 代码块内的 JSON
        let code_re = Regex::new(r"(?s)```(?:json)?\s*(\{.*?\})\s*```").unwrap();
        if let Some(caps) = code_re.captures(trimmed) {
            if let Some(m) = caps.get(1) {
                if let Ok(val) = Self::parse_or_repair(m.as_str()) {
                    if let Some(extracted) = Self::resolve_call_from_value(&val) {
                        return Some(extracted);
                    }
                }
            }
        }

        // 模式 3: 全局查找合法的 JSON 工具字典对象
        if let Some(candidate) = Self::extract_outermost_object(trimmed) {
            if let Ok(val) = Self::parse_or_repair(&candidate) {
                if let Some(extracted) = Self::resolve_call_from_value(&val) {
                    return Some(extracted);
                }
            }
        }

        None
    }

    /// 从 JSON Value 中提取规范的 (tool_name, arguments_object)
    fn resolve_call_from_value(val: &Value) -> Option<(String, Value)> {
        if !val.is_object() {
            return None;
        }

        // 形式 A: { "name": "view_file", "arguments": { ... } }
        if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
            let args = val.get("arguments").cloned().unwrap_or(json!({}));
            let final_args = if args.is_string() {
                Self::parse_or_repair(args.as_str().unwrap()).unwrap_or(json!({}))
            } else {
                args
            };
            return Some((name.to_string(), final_args));
        }

        // 形式 B: { "tool": "view_file", "parameters": { ... } } 或 { "tool": "view_file", "args": { ... } }
        if let Some(tool) = val.get("tool").and_then(|v| v.as_str()) {
            let args = val.get("parameters")
                .or_else(|| val.get("args"))
                .or_else(|| val.get("arguments"))
                .cloned()
                .unwrap_or_else(|| {
                    // 若直接平铺字段：{ "tool": "view_file", "path": "src/main.rs" }
                    let mut obj = val.as_object().unwrap().clone();
                    obj.remove("tool");
                    Value::Object(obj)
                });
            return Some((tool.to_string(), args));
        }

        // 形式 C: { "function": "view_file", ... }
        if let Some(func) = val.get("function").and_then(|v| v.as_str()) {
            let args = val.get("arguments").or_else(|| val.get("parameters")).cloned().unwrap_or(json!({}));
            return Some((func.to_string(), args));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repair_trailing_commas_and_unclosed_braces() {
        // 典型小模型输出：尾部带逗号且缺少闭合括号
        let broken = r#"{"name": "test_rule", "url": "https://api.example.com", "headers": {"User-Agent": "Hub/1.0",}"#;
        let res = FuzzyJsonRepair::parse_or_repair(broken);
        assert!(res.is_ok(), "Should successfully repair unclosed braces");
        let val = res.unwrap();
        assert_eq!(val["name"], "test_rule");
        assert_eq!(val["headers"]["User-Agent"], "Hub/1.0");
    }

    #[test]
    fn test_strip_markdown_code_block() {
        let md_wrapped = "根据要求，调用工具参数如下：\n```json\n{\n  \"command\": \"cargo test\",\n  \"timeout\": 30\n}\n```\n请尽快执行。";
        let res = FuzzyJsonRepair::parse_or_repair(md_wrapped);
        assert!(res.is_ok());
        let val = res.unwrap();
        assert_eq!(val["command"], "cargo test");
        assert_eq!(val["timeout"], 30);
    }

    #[test]
    fn test_chinese_punctuation_repair() {
        let chinese_json = r#"{“tool”：“spider_runner”， “target”：“wanou”}"#;
        let res = FuzzyJsonRepair::parse_or_repair(chinese_json);
        assert!(res.is_ok());
        let val = res.unwrap();
        assert_eq!(val["tool"], "spider_runner");
        assert_eq!(val["target"], "wanou");
    }

    #[test]
    fn test_extract_embedded_tool_call_xml() {
        let content = "我需要先查看当前文件列表。\n<tool_call>\n{\"name\": \"list_dir\", \"arguments\": {\"path\": \".\"}}\n</tool_call>";
        let res = FuzzyJsonRepair::extract_embedded_tool_call(content);
        assert!(res.is_some());
        let (tool_name, args) = res.unwrap();
        assert_eq!(tool_name, "list_dir");
        assert_eq!(args["path"], ".");
    }

    #[test]
    fn test_extract_embedded_tool_call_flat_object() {
        let content = "好的，正在为你读取配置文件：\n```json\n{\n  \"tool\": \"read_file\",\n  \"path\": \"package.json\"\n}\n```";
        let res = FuzzyJsonRepair::extract_embedded_tool_call(content);
        assert!(res.is_some());
        let (tool_name, args) = res.unwrap();
        assert_eq!(tool_name, "read_file");
        assert_eq!(args["path"], "package.json");
    }
}
