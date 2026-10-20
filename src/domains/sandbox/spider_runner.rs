//! 爬虫沙箱驱动器 (Spider Sandbox Runner)
//!
//! 在隔离的工作区沙箱中安全运行待测试的数据解析函数或逆向脚本，
//! 注入轻量环境垫片并断言返回结果。

use super::local_sandbox::LocalSandbox;
use super::sandbox_runtime::SandboxRuntime;
use super::spider_assertion::{evaluate_spider_output, AssertionVerdict, SpiderAssertionSpec};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// 爬虫沙箱测试配置
#[derive(Debug, Clone)]
pub struct SpiderRunConfig {
    /// 目标执行环境 (node / python)
    pub runtime_engine: String,
    /// 待测试的解析代码或脚本文件路径
    pub script_code: String,
    /// 模拟输入的 HTML 源码、网络响应或 Mock JSON
    pub mock_input: String,
    /// 拟调用的函数名称（例如 "parseList", "fetchDetail", "find_rule"）
    pub target_function: Option<String>,
    /// 断言约束规格
    pub assertion_spec: SpiderAssertionSpec,
}

impl Default for SpiderRunConfig {
    fn default() -> Self {
        Self {
            runtime_engine: "node".to_string(),
            script_code: String::new(),
            mock_input: "<html><body><div class='item'><a href='/play/1'>第1集</a></div></body></html>".to_string(),
            target_function: None,
            assertion_spec: SpiderAssertionSpec::default(),
        }
    }
}

/// 爬虫沙箱测试运行器
pub struct SpiderSandboxRunner {
    sandbox: LocalSandbox,
}

impl SpiderSandboxRunner {
    pub fn new(sandbox_root: impl AsRef<Path>) -> Self {
        Self {
            sandbox: LocalSandbox::new(sandbox_root),
        }
    }

    /// 在沙箱内运行并验证代码片段
    pub async fn run_and_verify(&self, config: &SpiderRunConfig) -> Result<AssertionVerdict, String> {
        // 前置环境探针检查，防止无环境时假死或抛出乱码
        let is_python = config.runtime_engine.eq_ignore_ascii_case("python");
        let runtime_check = if is_python {
            super::env_bootstrapper::EnvBootstrapper::probe_python()
        } else {
            super::env_bootstrapper::EnvBootstrapper::probe_node()
        };

        if !runtime_check.available {
            return Ok(AssertionVerdict::Failed {
                reason: format!("本地运行环境缺失: 系统未检测到可用的 {} 运行时，请先安装或配置环境变量", config.runtime_engine),
                error_type: "EnvironmentNotFoundError".to_string(),
                line_number: None,
                stack_trace: None,
                raw_output: format!("Runtime not found: {}", config.runtime_engine),
            });
        }

        self.sandbox.ensure_root_dir().await?;

        let run_id = format!("run_{}", Uuid::new_v4().simple());
        let runner_filename = format!("{}_runner.js", run_id);
        let input_filename = format!("{}_input.txt", run_id);

        // 1. 将 mock 输入写入沙箱
        self.sandbox.write_file(&input_filename, &config.mock_input).await?;

        // 2. 组装包装后的 JavaScript 执行脚本（注入垫片）
        let runner_script = generate_hiker_node_shim(
            &config.script_code,
            &input_filename,
            config.target_function.as_deref(),
        );

        self.sandbox.write_file(&runner_filename, &runner_script).await?;

        // 3. 执行脚本
        let exec_result = self.sandbox
            .execute("node", &[&runner_filename], HashMap::new())
            .await;

        // 4. 清理临时 runner 文件
        let _ = self.sandbox.execute("node", &["-e", &format!("fs.unlinkSync('{}'); fs.unlinkSync('{}');", runner_filename, input_filename)], HashMap::new()).await;

        match exec_result {
            Ok(res) => {
                let verdict = evaluate_spider_output(
                    res.exit_code,
                    &res.stdout,
                    &res.stderr,
                    &config.assertion_spec,
                );
                Ok(verdict)
            }
            Err(e) => {
                // 超时或执行异常
                Ok(AssertionVerdict::Failed {
                    reason: e.clone(),
                    error_type: "ExecutionTimeoutOrCrash".to_string(),
                    line_number: None,
                    stack_trace: None,
                    raw_output: e,
                })
            }
        }
    }
}

/// 构造包含视界与通用爬虫垫片的 Node.js 运行包裹器
fn generate_hiker_node_shim(
    user_code: &str,
    input_file: &str,
    target_function: Option<&str>,
) -> String {
    let func_invoke = if let Some(func_name) = target_function {
        format!(
            "try {{
                let res = typeof {fname} === 'function' ? {fname}(HTML_INPUT) : eval('{fname}');
                if (res !== undefined) setResult(res);
            }} catch(err) {{
                console.error(err);
                process.exit(1);
            }}",
            fname = func_name
        )
    } else {
        "// 直接执行代码主体".to_string()
    };

    format!(
        r#"
const fs = require('fs');

// 读取测试输入
const HTML_INPUT = fs.readFileSync('{input_file}', 'utf-8');

// 注入视界与轻量爬虫全局垫片
globalThis.HTML = HTML_INPUT;
globalThis.setResult = function(data) {{
    console.log('__SPIDER_RESULT__:' + JSON.stringify(data));
}};
globalThis.print = console.log;
globalThis.request = function(url) {{
    return HTML_INPUT;
}};
globalThis.base64Decode = function(str) {{
    return Buffer.from(str, 'base64').toString('utf-8');
}};
globalThis.base64Encode = function(str) {{
    return Buffer.from(str, 'utf-8').toString('base64');
}};
globalThis.md5 = function(str) {{
    const crypto = require('crypto');
    return crypto.createHash('md5').update(str).digest('hex');
}};

// 用户逆向代码注入
{user_code}

// 执行目标函数或断言
{func_invoke}
"#,
        input_file = input_file,
        user_code = user_code,
        func_invoke = func_invoke
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_spider_sandbox_runner_success() {
        let dir = tempdir().unwrap();
        let runner = SpiderSandboxRunner::new(dir.path());

        let code = r#"
function parseEpisodes(html) {
    return [
        { title: "第1集 序章", url: "https://example.com/1" },
        { title: "第2集 出山", url: "https://example.com/2" }
    ];
}
"#;

        let config = SpiderRunConfig {
            runtime_engine: "node".to_string(),
            script_code: code.to_string(),
            mock_input: "test".to_string(),
            target_function: Some("parseEpisodes".to_string()),
            assertion_spec: SpiderAssertionSpec::default(),
        };

        let verdict = runner.run_and_verify(&config).await.unwrap();
        assert!(verdict.is_passed());
    }

    #[tokio::test]
    async fn test_spider_sandbox_runner_detects_error() {
        let dir = tempdir().unwrap();
        let runner = SpiderSandboxRunner::new(dir.path());

        let bad_code = r#"
function parseEpisodes(html) {
    throw new TypeError("Cannot read property 'find' of undefined");
}
"#;

        let config = SpiderRunConfig {
            runtime_engine: "node".to_string(),
            script_code: bad_code.to_string(),
            mock_input: "test".to_string(),
            target_function: Some("parseEpisodes".to_string()),
            assertion_spec: SpiderAssertionSpec::default(),
        };

        let verdict = runner.run_and_verify(&config).await.unwrap();
        assert!(!verdict.is_passed());
        if let AssertionVerdict::Failed { error_type, .. } = verdict {
            assert_eq!(error_type, "TypeError");
        }
    }
}
