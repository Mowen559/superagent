//! Reflexion 自愈引擎与行级事务闭环 (Reflexion Self-Healing Engine)
//!
//! 专为 Spider Reverse Copilot 打造的闭环自愈中枢：
//! 1. 驱动沙箱进行规则功能试运行；
//! 2. 断言失败秒级调用 LineLevelTransaction 进行行级无损回滚；
//! 3. 提取异常堆栈与错误行号，形成自愈反馈提示词（Reflexion Feedback）；
//! 4. 控制自愈重试次数（默认 3 次熔断上限），防止无限递归消耗 Token。

use super::spider_assertion::{AssertionVerdict, SpiderAssertionSpec};
use super::spider_runner::{SpiderRunConfig, SpiderSandboxRunner};
use crate::domains::version_control::diff_transaction::{LineLevelTransaction, TransactionStatus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DEFAULT_MAX_ATTEMPTS: usize = 3;

/// 单次自愈尝试的审计记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflexionAttempt {
    pub attempt_number: usize,
    pub proposed_code_snippet: String,
    pub verdict: AssertionVerdict,
    pub rolled_back: bool,
    pub timestamp: i64,
}

/// 自愈反思报告（供智能体思考并生成下一轮修复代码）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflexionFeedback {
    pub attempt_number: usize,
    pub max_attempts: usize,
    pub can_retry: bool,
    pub error_type: String,
    pub error_message: String,
    pub error_line: Option<usize>,
    pub stack_trace: Option<String>,
    pub guidance_prompt: String,
}

/// 自愈执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReflexionOutcome {
    Success {
        total_attempts: usize,
        record_count: usize,
        sample: Option<serde_json::Value>,
        message: String,
    },
    FailedAndRolledBack {
        total_attempts: usize,
        final_verdict: AssertionVerdict,
        feedback: ReflexionFeedback,
    },
}

/// 爬虫逆向自愈会话管理器
pub struct SpiderReflexionSession {
    pub target_file: PathBuf,
    pub sandbox_runner: SpiderSandboxRunner,
    pub max_attempts: usize,
    pub current_attempt: usize,
    pub history: Vec<ReflexionAttempt>,
}

impl SpiderReflexionSession {
    pub fn new(target_file: impl AsRef<Path>, sandbox_dir: impl AsRef<Path>) -> Self {
        Self {
            target_file: target_file.as_ref().to_path_buf(),
            sandbox_runner: SpiderSandboxRunner::new(sandbox_dir),
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            current_attempt: 0,
            history: Vec::new(),
        }
    }

    pub fn with_max_attempts(mut self, max: usize) -> Self {
        self.max_attempts = max;
        self
    }

    /// 执行一次代码修改并驱动沙箱验证；若验证失败自动秒级行级回滚！
    pub async fn apply_and_verify(
        &mut self,
        new_code: &str,
        mock_input: &str,
        target_function: Option<&str>,
        assertion_spec: &SpiderAssertionSpec,
    ) -> Result<ReflexionOutcome, String> {
        self.current_attempt += 1;
        let attempt_num = self.current_attempt;

        // 1. 开启行级事务并暂存变更
        let mut tx = LineLevelTransaction::begin(&self.target_file, Some("Spider Copilot 逆向自愈测试"))?;
        let _payload = tx.stage_change(new_code)?;

        // 2. 组装沙箱执行配置
        let run_config = SpiderRunConfig {
            runtime_engine: "node".to_string(),
            script_code: new_code.to_string(),
            mock_input: mock_input.to_string(),
            target_function: target_function.map(ToString::to_string),
            assertion_spec: assertion_spec.clone(),
        };

        // 3. 驱动沙箱试运行
        let verdict = self.sandbox_runner.run_and_verify(&run_config).await?;

        // 4. 根据断言判决分支处理
        if verdict.is_passed() {
            // 验证通过：固化提交变更
            tx.commit()?;

            self.history.push(ReflexionAttempt {
                attempt_number: attempt_num,
                proposed_code_snippet: summarize_snippet(new_code),
                verdict: verdict.clone(),
                rolled_back: false,
                timestamp: chrono::Utc::now().timestamp(),
            });

            let (count, sample, msg) = match verdict {
                AssertionVerdict::Passed { record_count, sample, message } => (record_count, sample, message),
                _ => (0, None, String::new()),
            };

            Ok(ReflexionOutcome::Success {
                total_attempts: attempt_num,
                record_count: count,
                sample,
                message: msg,
            })
        } else {
            // 验证失败：立即行级秒级回滚！
            tx.rollback()?;

            self.history.push(ReflexionAttempt {
                attempt_number: attempt_num,
                proposed_code_snippet: summarize_snippet(new_code),
                verdict: verdict.clone(),
                rolled_back: true,
                timestamp: chrono::Utc::now().timestamp(),
            });

            // 生成反思提示词 (Reflexion Guidance)
            let feedback = self.generate_feedback(&verdict, attempt_num);

            Ok(ReflexionOutcome::FailedAndRolledBack {
                total_attempts: attempt_num,
                final_verdict: verdict,
                feedback,
            })
        }
    }

    /// 生成自愈引导提示词
    fn generate_feedback(&self, verdict: &AssertionVerdict, attempt_num: usize) -> ReflexionFeedback {
        let can_retry = attempt_num < self.max_attempts;

        let (err_type, reason, line, stack) = match verdict {
            AssertionVerdict::Failed { error_type, reason, line_number, stack_trace, .. } => {
                (error_type.clone(), reason.clone(), *line_number, stack_trace.clone())
            }
            _ => ("UnknownError".to_string(), "断言未通过".to_string(), None, None),
        };

        let guidance_prompt = format!(
            "[Spider Copilot Reflexion 反思报告 - 第 {}/{} 次尝试]\n\
             - 异常类型: {}\n\
             - 出错原因: {}\n\
             - 错误行号: {}\n\
             - 状态: ⚠️ 触发断言守卫失败！系统已利用反向补丁自动将代码行级无损回滚至安全基线，未污染磁盘。\n\
             - 逆向建议: 检查正则表达式捕获组、DOM选择器是否匹配模拟HTML、或返回值是否被 setResult 正确捕获。\n\
             - 下一步: {}",
            attempt_num,
            self.max_attempts,
            err_type,
            reason,
            line.map(|l| l.to_string()).unwrap_or_else(|| "未知/全局".to_string()),
            if can_retry {
                "请根据以上报错堆栈调整解析逻辑，重新提交修正代码。"
            } else {
                "已达到最大自愈重试上限 (3次)，请向用户汇报瓶颈并请求人工介入指引。"
            }
        );

        ReflexionFeedback {
            attempt_number: attempt_num,
            max_attempts: self.max_attempts,
            can_retry,
            error_type: err_type,
            error_message: reason,
            error_line: line,
            stack_trace: stack,
            guidance_prompt,
        }
    }
}

fn summarize_snippet(code: &str) -> String {
    let lines: Vec<&str> = code.lines().take(5).collect();
    let mut s = lines.join("\n");
    if code.lines().count() > 5 {
        s.push_str("\n... [truncated]");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    #[tokio::test]
    async fn test_reflexion_self_healing_success_flow() {
        let dir = tempdir().unwrap();
        let target_file = dir.path().join("rule.js");
        let initial_code = "// 初始版本\nfunction fetchList() { return []; }\n";
        fs::write(&target_file, initial_code).unwrap();

        let sandbox_dir = dir.path().join("sandbox");
        let mut session = SpiderReflexionSession::new(&target_file, &sandbox_dir);

        let good_code = r#"
function fetchList(html) {
    return [{ title: "第1集", url: "https://example.com/1" }];
}
"#;

        let assertion = SpiderAssertionSpec::default();
        let outcome = session
            .apply_and_verify(good_code, "test", Some("fetchList"), &assertion)
            .await
            .unwrap();

        match outcome {
            ReflexionOutcome::Success { record_count, .. } => {
                assert_eq!(record_count, 1);
                // 验证文件已被固化提交
                assert_eq!(fs::read_to_string(&target_file).unwrap(), good_code);
            }
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_reflexion_auto_rollback_on_failure() {
        let dir = tempdir().unwrap();
        let target_file = dir.path().join("spider.js");
        let safe_initial = "// 初始安全基线代码\nvar x = 1;\n";
        fs::write(&target_file, safe_initial).unwrap();

        let sandbox_dir = dir.path().join("sandbox");
        let mut session = SpiderReflexionSession::new(&target_file, &sandbox_dir);

        // 模拟提交会引发 TypeError 崩溃的代码
        let broken_code = r#"
function fetchList(html) {
    let item = null;
    return item.missingProperty.split(",");
}
"#;

        let assertion = SpiderAssertionSpec::default();
        let outcome = session
            .apply_and_verify(broken_code, "test", Some("fetchList"), &assertion)
            .await
            .unwrap();

        match outcome {
            ReflexionOutcome::FailedAndRolledBack { feedback, .. } => {
                assert_eq!(feedback.error_type, "TypeError");
                assert!(feedback.guidance_prompt.contains("回滚"));

                // 核心断言：磁盘文件必须被秒级无损回退至 safe_initial，绝无残留报错代码！
                let current_content = fs::read_to_string(&target_file).unwrap();
                assert_eq!(current_content, safe_initial);
            }
            _ => panic!("Expected FailedAndRolledBack"),
        }
    }
}
