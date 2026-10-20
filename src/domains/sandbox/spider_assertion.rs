//! 爬虫与逆向规则断言守卫 (Spider Rule Assertion Guard)
//!
//! 提供面向逆向编写场景的工业级断言套件：
//! 1. 验证解析结果是否包含有效数据（非空数组/非空对象）；
//! 2. 校验关键字段提取完整性（title, url, pic 等）；
//! 3. 拦截未捕获的运行时异常（SyntaxError, ReferenceError, TypeError 等）；
//! 4. 给出结构化自愈诊断报告。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 爬虫解析断言配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiderAssertionSpec {
    /// 是否要求解析结果必须为非空集合 (records.len() > 0)
    #[serde(default = "default_true")]
    pub require_non_empty: bool,

    /// 预期必须存在的字段清单（例如 ["title", "url"]）
    #[serde(default)]
    pub required_fields: Vec<String>,

    /// 是否允许关键字段为 null 或空字符串
    #[serde(default = "default_false")]
    pub allow_empty_values: bool,

    /// 最小记录条数预期（例如最少提取到 1 条）
    #[serde(default = "default_min_records")]
    pub min_records: usize,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_min_records() -> usize {
    1
}

impl Default for SpiderAssertionSpec {
    fn default() -> Self {
        Self {
            require_non_empty: true,
            required_fields: vec!["title".to_string()],
            allow_empty_values: false,
            min_records: 1,
        }
    }
}

/// 断言判决结果
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AssertionVerdict {
    Passed {
        record_count: usize,
        sample: Option<Value>,
        message: String,
    },
    Failed {
        reason: String,
        error_type: String,
        line_number: Option<usize>,
        stack_trace: Option<String>,
        raw_output: String,
    },
}

impl AssertionVerdict {
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed { .. })
    }

    pub fn summary_message(&self) -> String {
        match self {
            Self::Passed { message, record_count, .. } => {
                format!("✓ 断言通过: {} (获取到 {} 条记录)", message, record_count)
            }
            Self::Failed { reason, error_type, line_number, .. } => {
                if let Some(line) = line_number {
                    format!("✗ 断言失败 [{} 于行 {}]: {}", error_type, line, reason)
                } else {
                    format!("✗ 断言失败 [{}]: {}", error_type, reason)
                }
            }
        }
    }
}

/// 评估沙箱执行输出与解析数据
pub fn evaluate_spider_output(
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    spec: &SpiderAssertionSpec,
) -> AssertionVerdict {
    // 1. 检查是否存在严重语法或运行时错误
    if exit_code != 0 || stderr.contains("SyntaxError") || stderr.contains("ReferenceError") || stderr.contains("TypeError") {
        let (err_type, line_num, stack) = parse_javascript_error(stderr, stdout);
        return AssertionVerdict::Failed {
            reason: if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                format!("进程异常退出，退出码: {}", exit_code)
            },
            error_type: err_type,
            line_number: line_num,
            stack_trace: stack,
            raw_output: format!("{}\n{}", stdout, stderr),
        };
    }

    // 2. 尝试从 stdout 中提取 JSON 结构化结果
    // 约定沙箱打印以 `__SPIDER_RESULT__:` 开头，或者整段输出为 JSON
    let parsed_json = extract_result_json(stdout);

    match parsed_json {
        Some(val) => {
            // 校验数组类型
            if let Some(arr) = val.as_array() {
                if spec.require_non_empty && arr.is_empty() {
                    return AssertionVerdict::Failed {
                        reason: "解析结果数组为空，未匹配到任何有效数据条目！".to_string(),
                        error_type: "EmptyResultError".to_string(),
                        line_number: None,
                        stack_trace: None,
                        raw_output: stdout.to_string(),
                    };
                }

                if arr.len() < spec.min_records {
                    return AssertionVerdict::Failed {
                        reason: format!("解析结果条数不足：预期至少 {} 条，实际仅获取到 {} 条", spec.min_records, arr.len()),
                        error_type: "InsufficientRecordsError".to_string(),
                        line_number: None,
                        stack_trace: None,
                        raw_output: stdout.to_string(),
                    };
                }

                // 校验关键字段存在性
                if !spec.required_fields.is_empty() {
                    for (idx, item) in arr.iter().enumerate() {
                        if let Some(obj) = item.as_object() {
                            for req_field in &spec.required_fields {
                                match obj.get(req_field) {
                                    None => {
                                        return AssertionVerdict::Failed {
                                            reason: format!("第 {} 项记录缺少必填字段 '{}'", idx + 1, req_field),
                                            error_type: "MissingFieldError".to_string(),
                                            line_number: None,
                                            stack_trace: None,
                                            raw_output: stdout.to_string(),
                                        };
                                    }
                                    Some(field_val) => {
                                        if !spec.allow_empty_values && is_value_empty(field_val) {
                                            return AssertionVerdict::Failed {
                                                reason: format!("第 {} 项记录的必填字段 '{}' 提取值为空！", idx + 1, req_field),
                                                error_type: "EmptyFieldValueError".to_string(),
                                                line_number: None,
                                                stack_trace: None,
                                                raw_output: stdout.to_string(),
                                            };
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                AssertionVerdict::Passed {
                    record_count: arr.len(),
                    sample: arr.first().cloned(),
                    message: format!("成功提取 {} 条结构化数据", arr.len()),
                }
            } else if let Some(obj) = val.as_object() {
                // 单个对象结果（如详情页逆向）
                for req_field in &spec.required_fields {
                    match obj.get(req_field) {
                        None => {
                            return AssertionVerdict::Failed {
                                reason: format!("解析详情对象缺少关键字段 '{}'", req_field),
                                error_type: "MissingFieldError".to_string(),
                                line_number: None,
                                stack_trace: None,
                                raw_output: stdout.to_string(),
                            };
                        }
                        Some(field_val) => {
                            if !spec.allow_empty_values && is_value_empty(field_val) {
                                return AssertionVerdict::Failed {
                                    reason: format!("解析详情字段 '{}' 提取值为空！", req_field),
                                    error_type: "EmptyFieldValueError".to_string(),
                                    line_number: None,
                                    stack_trace: None,
                                    raw_output: stdout.to_string(),
                                };
                            }
                        }
                    }
                }

                AssertionVerdict::Passed {
                    record_count: 1,
                    sample: Some(val),
                    message: "成功提取有效详情数据对象".to_string(),
                }
            } else {
                AssertionVerdict::Failed {
                    reason: format!("解析返回值类型非结构化集合（收到类型: {}）", val),
                    error_type: "InvalidResultTypeError".to_string(),
                    line_number: None,
                    stack_trace: None,
                    raw_output: stdout.to_string(),
                }
            }
        }
        None => {
            // 未能解析到 JSON 标记
            AssertionVerdict::Failed {
                reason: "沙箱未返回有效的 JSON 结构化数据或未调用 setResult".to_string(),
                error_type: "NoJsonOutputError".to_string(),
                line_number: None,
                stack_trace: None,
                raw_output: stdout.to_string(),
            }
        }
    }
}

fn is_value_empty(val: &Value) -> bool {
    match val {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

fn extract_result_json(stdout: &str) -> Option<Value> {
    for line in stdout.lines() {
        if let Some(stripped) = line.strip_prefix("__SPIDER_RESULT__:") {
            if let Ok(v) = serde_json::from_str::<Value>(stripped.trim()) {
                return Some(v);
            }
        }
    }

    // 尝试寻找整段输出里的 JSON
    if let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) {
        return Some(v);
    }

    None
}

fn parse_javascript_error(stderr: &str, stdout: &str) -> (String, Option<usize>, Option<String>) {
    let combined = format!("{}\n{}", stderr, stdout);
    let mut err_type = "RuntimeError".to_string();
    let mut line_num = None;

    for line in combined.lines() {
        if line.contains("SyntaxError:") {
            err_type = "SyntaxError".to_string();
        } else if line.contains("ReferenceError:") {
            err_type = "ReferenceError".to_string();
        } else if line.contains("TypeError:") {
            err_type = "TypeError".to_string();
        }

        // 尝试捕获行号例如 :23:15
        if let Some(pos) = line.find(':') {
            let rest = &line[pos + 1..];
            if let Some(colon2) = rest.find(':') {
                if let Ok(num) = rest[..colon2].parse::<usize>() {
                    line_num = Some(num);
                }
            }
        }
    }

    (err_type, line_num, Some(combined))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_assertion_passed_with_records() {
        let spec = SpiderAssertionSpec::default();
        let stdout = format!("__SPIDER_RESULT__: {}", json!([
            { "title": "斗破苍穹 第1集", "url": "https://example.com/play/1" },
            { "title": "斗破苍穹 第2集", "url": "https://example.com/play/2" }
        ]));

        let verdict = evaluate_spider_output(0, &stdout, "", &spec);
        assert!(verdict.is_passed());
        if let AssertionVerdict::Passed { record_count, .. } = verdict {
            assert_eq!(record_count, 2);
        }
    }

    #[test]
    fn test_assertion_failed_on_empty_array() {
        let spec = SpiderAssertionSpec::default();
        let stdout = "__SPIDER_RESULT__: []";
        let verdict = evaluate_spider_output(0, stdout, "", &spec);
        assert!(!verdict.is_passed());
        assert!(verdict.summary_message().contains("为空"));
    }

    #[test]
    fn test_assertion_failed_on_syntax_error() {
        let spec = SpiderAssertionSpec::default();
        let stderr = "SyntaxError: Unexpected token '}'\n    at eval:12:5";
        let verdict = evaluate_spider_output(1, "", stderr, &spec);
        assert!(!verdict.is_passed());
        if let AssertionVerdict::Failed { error_type, line_number, .. } = verdict {
            assert_eq!(error_type, "SyntaxError");
            assert_eq!(line_number, Some(12));
        }
    }
}
