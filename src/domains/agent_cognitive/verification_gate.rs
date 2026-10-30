use serde::{Deserialize, Serialize};
use crate::domains::version_control::regression_guard::{RegressionAuditReport, RegressionRollbackGuard};

/// 验证状态凭据
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationVerdict {
    Passed,
    Failed { reason: String },
}

/// 某次验证执行的不可变证据回执
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReceipt {
    pub tool_name: String,
    pub command: Option<String>,
    pub verdict: VerificationVerdict,
    pub timestamp_ms: u64,
    pub output_snippet: String,
}

/// 门禁决策
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// 验证全绿通过或无需验证，允许交付
    Passed,
    /// 门禁打回并向智能体注入反思/行动指引
    RejectAndSteer {
        reason: String,
        suggested_action: String,
        current_attempt: usize,
        max_attempts: usize,
    },
    /// 自愈重试超限，强制转为透明诊断报告交付
    MaxRetriesDegraded {
        attempts: usize,
        diagnostic: String,
    },
}

/// 门禁策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationPolicy {
    /// 严格模式：修改过代码或涉及执行任务时，必须具备通过的验证回执
    Strict,
    /// 宽松模式：仅记录，不强行阻断
    Permissive,
}

/// 自主验证门禁守护器 (Definition-of-Done Gate Guard)
#[derive(Debug, Clone)]
pub struct VerificationGateGuard {
    pub policy: VerificationPolicy,
    pub max_retries: usize,
    mutation_count_since_last_verification: usize,
    total_mutation_count: usize,
    latest_receipt: Option<VerificationReceipt>,
    pub latest_regression_report: Option<RegressionAuditReport>,
    pub rejection_count: usize,
}

impl Default for VerificationGateGuard {
    fn default() -> Self {
        Self::new(VerificationPolicy::Strict, 3)
    }
}

impl VerificationGateGuard {
    pub fn new(policy: VerificationPolicy, max_retries: usize) -> Self {
        Self {
            policy,
            max_retries,
            mutation_count_since_last_verification: 0,
            total_mutation_count: 0,
            latest_receipt: None,
            latest_regression_report: None,
            rejection_count: 0,
        }
    }


    pub fn is_mutation_tool(tool_name: &str) -> bool {
        matches!(
            tool_name,
            "write_to_file"
                | "write_file"
                | "replace_in_file"
                | "multi_replace"
                | "apply_patch"
                | "patch_file"
                | "create_file"
                | "delete_file"
                | "edit_file"
        )
    }

    /// 判断是否属于验证/测试/执行类工具
    pub fn is_verification_tool(tool_name: &str) -> bool {
        matches!(
            tool_name,
            "run_command"
                | "run_spider"
                | "evaluate_codebase"
                | "verify_task"
                | "cargo_test"
                | "node_test"
                | "python_test"
                | "run_unit_test"
                | "spider_runner"
                | "run_benchmark"
        )
    }

    /// 记录工具调用，实时更新状态机
    pub fn observe_tool_execution(
        &mut self,
        tool_name: &str,
        command_arg: Option<String>,
        success: bool,
        output_snippet: &str,
    ) {
        if Self::is_mutation_tool(tool_name) {
            self.total_mutation_count += 1;
            self.mutation_count_since_last_verification += 1;
            // 铁律：任何新的代码改动，立即令先前的验证凭据失效！
            self.latest_receipt = None;
            self.latest_regression_report = None;
            return;
        }

        if Self::is_verification_tool(tool_name) {
            let reg_report = RegressionRollbackGuard::audit_regression_from_output(output_snippet);
            let has_regression = reg_report.has_regression;
            let verdict = if success && !has_regression {
                VerificationVerdict::Passed
            } else {
                let reason = if has_regression {
                    format!("存量回归硬拦截: {}", reg_report.summary)
                } else {
                    output_snippet.chars().take(200).collect()
                };
                VerificationVerdict::Failed { reason }
            };

            if has_regression {
                self.latest_regression_report = Some(reg_report);
            } else {
                self.latest_regression_report = None;
            }

            self.latest_receipt = Some(VerificationReceipt {
                tool_name: tool_name.to_string(),
                command: command_arg,
                verdict,
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                output_snippet: output_snippet.chars().take(300).collect(),
            });

            self.mutation_count_since_last_verification = 0;
        }
    }

    /// 在智能体企图输出最终文本结论结题时，进行门禁严审
    pub fn evaluate_conclusion_attempt(&mut self, is_conversational_only: bool) -> GateDecision {
        if self.policy == VerificationPolicy::Permissive {
            return GateDecision::Passed;
        }

        // 如果纯属闲聊问答，且从未改动过任何代码/文件，免检放行
        if is_conversational_only && self.total_mutation_count == 0 {
            return GateDecision::Passed;
        }

        // 核心检查 0：优先硬拦截存量测试回归 (Stock Regression)
        if let Some(ref reg) = self.latest_regression_report {
            if reg.has_regression {
                return self.trigger_rejection(
                    &format!(
                        "【致命存量回归事故 (Stock Regression Alert)】: 检测到改动导致既有存量测试失败！失败清单: {:?}。",
                        reg.failed_tests
                    ),
                    "严禁拆东墙补西墙！本次改动破坏了系统原有的存量功能。请立即排查由于代码修改带来的连带副作用，修复所有失败用例，直到全绿后再结题！",
                );
            }
        }

        // 核心检查 1：最近一次执行的验证是失败的
        if let Some(ref receipt) = self.latest_receipt {
            if let VerificationVerdict::Failed { ref reason } = receipt.verdict {
                return self.trigger_rejection(
                    &format!("最近一次执行的验证未通过，错误原因: {}", reason),
                    "请根据报错根因进行针对性修复，并重新运行验证工具，直到测试全绿通过后再交付。",
                );
            }
        }

        // 核心检查 2：有写盘改动，但改动之后从未执行过验证
        if self.mutation_count_since_last_verification > 0 || (self.total_mutation_count > 0 && self.latest_receipt.is_none()) {
            return self.trigger_rejection(
                "你修改了文件/代码，但在修改后尚未执行任何运行验证或测试！",
                "请立即调用验证工具（如 run_command / run_spider / evaluate_codebase）运行测试脚本或验证探针，确认改动真实有效且无报错。",
            );
        }

        // 核心检查 3：未发生任何改动且验证为 Passed（或无需验证的合规只读流程）
        GateDecision::Passed
    }

    fn trigger_rejection(&mut self, reason: &str, suggested_action: &str) -> GateDecision {
        self.rejection_count += 1;
        if self.rejection_count > self.max_retries {
            return GateDecision::MaxRetriesDegraded {
                attempts: self.rejection_count,
                diagnostic: format!(
                    "已达到最大自愈尝试次数 ({} 次)。当前阻断问题: {}。禁止假装完成！请向用户诚实汇报当前遇到的深层阻塞、已尝试的方案与失败现场日志。",
                    self.max_retries, reason
                ),
            };
        }

        GateDecision::RejectAndSteer {
            reason: reason.to_string(),
            suggested_action: suggested_action.to_string(),
            current_attempt: self.rejection_count,
            max_attempts: self.max_retries,
        }
    }

    pub fn reset_session(&mut self) {
        self.mutation_count_since_last_verification = 0;
        self.total_mutation_count = 0;
        self.latest_receipt = None;
        self.rejection_count = 0;
    }



    pub fn rejection_count(&self) -> usize {
        self.rejection_count
    }

    pub fn latest_receipt(&self) -> Option<&VerificationReceipt> {
        self.latest_receipt.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversational_task_passes_without_mutation() {
        let mut gate = VerificationGateGuard::default();
        let decision = gate.evaluate_conclusion_attempt(true);
        assert_eq!(decision, GateDecision::Passed);
    }

    #[test]
    fn test_mutation_requires_verification() {
        let mut gate = VerificationGateGuard::default();
        
        // 模拟修改了代码文件
        gate.observe_tool_execution("write_to_file", None, true, "File written successfully");
        
        // 试图草草交差
        let decision = gate.evaluate_conclusion_attempt(false);
        match decision {
            GateDecision::RejectAndSteer { current_attempt, max_attempts, .. } => {
                assert_eq!(current_attempt, 1);
                assert_eq!(max_attempts, 3);
            }
            _ => panic!("Expected rejection due to unverified mutation"),
        }
    }

    #[test]
    fn test_verification_success_allows_passage() {
        let mut gate = VerificationGateGuard::default();
        
        // 修改代码
        gate.observe_tool_execution("replace_in_file", None, true, "ok");
        
        // 执行验证并成功
        gate.observe_tool_execution("run_command", Some("cargo test".to_string()), true, "test result: ok");
        
        let decision = gate.evaluate_conclusion_attempt(false);
        assert_eq!(decision, GateDecision::Passed);
    }

    #[test]
    fn test_new_mutation_invalidates_previous_verification() {
        let mut gate = VerificationGateGuard::default();
        
        // 第一次修改与验证
        gate.observe_tool_execution("write_to_file", None, true, "ok");
        gate.observe_tool_execution("run_command", Some("python test.py".to_string()), true, "ok");
        assert_eq!(gate.evaluate_conclusion_attempt(false), GateDecision::Passed);

        // 又进行了第二次修改，但没有再次验证
        gate.observe_tool_execution("patch_file", None, true, "ok");
        
        // 再次试图交差，必须被打回！
        let decision = gate.evaluate_conclusion_attempt(false);
        match decision {
            GateDecision::RejectAndSteer { .. } => {}
            _ => panic!("Expected rejection because new mutation invalidated previous test!"),
        }
    }

    #[test]
    fn test_failed_verification_blocks_passage() {
        let mut gate = VerificationGateGuard::default();
        
        // 修改代码并执行测试，测试失败
        gate.observe_tool_execution("write_to_file", None, true, "ok");
        gate.observe_tool_execution(
            "run_command",
            Some("npm test".to_string()),
            false,
            "AssertionError: expected 200 to equal 404",
        );

        let decision = gate.evaluate_conclusion_attempt(false);
        match decision {
            GateDecision::RejectAndSteer { ref reason, .. } => {
                assert!(reason.contains("AssertionError"));
            }
            _ => panic!("Expected rejection due to failed test"),
        }
    }

    #[test]
    fn test_max_retries_degrades_gracefully() {
        let mut gate = VerificationGateGuard::new(VerificationPolicy::Strict, 2);
        
        gate.observe_tool_execution("write_to_file", None, true, "ok");
        gate.observe_tool_execution("run_command", None, false, "syntax error");

        // 第 1 次打回
        let d1 = gate.evaluate_conclusion_attempt(false);
        assert!(matches!(d1, GateDecision::RejectAndSteer { .. }));

        // 第 2 次打回
        let d2 = gate.evaluate_conclusion_attempt(false);
        assert!(matches!(d2, GateDecision::RejectAndSteer { .. }));

        // 第 3 次超限（> 2），转为降级透明报告
        let d3 = gate.evaluate_conclusion_attempt(false);
        match d3 {
            GateDecision::MaxRetriesDegraded { attempts, diagnostic } => {
                assert_eq!(attempts, 3);
                assert!(diagnostic.contains("禁止假装完成"));
            }
            _ => panic!("Expected MaxRetriesDegraded"),
        }
    }

    #[test]
    fn test_stock_regression_interception() {
        let mut gate = VerificationGateGuard::default();
        gate.observe_tool_execution("replace_in_file", None, true, "ok");

        let cargo_output = r#"
running 3 tests
test stock_login ... FAILED
test new_feature ... ok

failures:
    stock_login

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured
        "#;
        gate.observe_tool_execution("cargo_test", Some("cargo test".into()), false, cargo_output);

        let decision = gate.evaluate_conclusion_attempt(false);
        match decision {
            GateDecision::RejectAndSteer { ref reason, .. } => {
                assert!(reason.contains("致命存量回归事故"));
                assert!(reason.contains("stock_login"));
            }
            _ => panic!("Expected rejection due to stock regression"),
        }
    }
}
