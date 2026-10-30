//! 自愈卡死突围机制与认知熔断器 (Meta-Cognitive Circuit Breaker)
//!
//! 核心防护链路：
//! 1. 错误签名归一化指纹提取 (Error Signature Fingerprinting)：过滤随机行号与时间戳，锁定核心崩溃模式；
//! 2. 连续相同错误熔断状态机 (Tri-State Circuit Breaker)：
//!    - Closed（正常通行） -> HalfOpen（2次同类错误黄牌警告） -> Open（3次熔断红牌）；
//! 3. 破局动作协议 (Breakout Action Protocol)：
//!    - 强制证伪当前死胡同假说；
//!    - 注入“Aha 顿悟反思”提示词，打断重复盲目试错惯性；
//!    - 联动 `RegressionRollbackGuard` 秒级物理回滚脏改动；
//!    - 引导生成隔离诊断脚本（`scratch/probe_*.rs`）突围。

use serde::{Deserialize, Serialize};

/// 熔断器状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitBreakerState {
    /// 正常状态：系统运行平稳，无连续死循环
    Closed,
    /// 半开预警：已连续出现 2 次相同报错，亮黄牌警告
    HalfOpen,
    /// 熔断爆发：达到 3 次相同死循环，强行切断盲目试错
    Open,
}

/// 归一化错误签名
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorSignature {
    pub tool_name: String,
    pub signature_hash: String,
    pub core_summary: String,
    pub consecutive_count: usize,
}

/// 破局行动指令
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakoutDirective {
    pub state: CircuitBreakerState,
    pub consecutive_failures: usize,
    pub error_summary: String,
    pub trip_reason: String,
    pub aha_epiphany_prompt: String,
    pub suggested_isolated_probe: String,
    pub should_rollback_workspace: bool,
}

/// 认知熔断守卫器
#[derive(Debug, Clone)]
pub struct CircuitBreakerGuard {
    pub state: CircuitBreakerState,
    pub threshold: usize,
    current_signature: Option<ErrorSignature>,
    total_trips: usize,
}

impl Default for CircuitBreakerGuard {
    fn default() -> Self {
        Self::new(3)
    }
}

impl CircuitBreakerGuard {
    pub fn new(threshold: usize) -> Self {
        Self {
            state: CircuitBreakerState::Closed,
            threshold: threshold.max(2),
            current_signature: None,
            total_trips: 0,
        }
    }

    /// 提取错误输出的稳定特征签名（剔除时间戳、文件行号与指针地址等噪声）
    pub fn extract_signature(tool_name: &str, output: &str) -> (String, String) {
        let mut core_lines = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            // 过滤无意义空行或纯分隔符
            if trimmed.is_empty() || trimmed.chars().all(|c| c == '-' || c == '=' || c == '*') {
                continue;
            }
            // 提取关键异常或编译错误标志
            if trimmed.contains("error[E")
                || trimmed.contains("FAILED")
                || trimmed.contains("Exception")
                || trimmed.contains("Error:")
                || trimmed.contains("AssertionError")
                || trimmed.contains("panic")
                || trimmed.contains("BUILD FAILURE")
                || trimmed.contains("403")
                || trimmed.contains("cannot find")
                || trimmed.contains("not found")
            {
                let sanitized = sanitize_error_line(trimmed);
                core_lines.push(sanitized);
                if core_lines.len() >= 2 {
                    break;
                }
            }
        }

        let summary = if core_lines.is_empty() {
            output.chars().take(100).collect::<String>()
        } else {
            core_lines.join(" | ")
        };

        let raw_sig = format!("{}:{}", tool_name, summary);
        let hash = crate::shared_kernel::crypto::sha256_hex(raw_sig.as_bytes());
        (hash, summary)
    }

    /// 记录一次成功的工具执行：重置熔断器至 Closed 状态
    pub fn record_success(&mut self) {
        self.state = CircuitBreakerState::Closed;
        self.current_signature = None;
    }

    /// 记录一次失败的工具执行，并更新熔断状态机
    pub fn record_failure(&mut self, tool_name: &str, output: &str) -> Option<BreakoutDirective> {
        let (hash, summary) = Self::extract_signature(tool_name, output);

        let count = match &mut self.current_signature {
            Some(sig) if sig.signature_hash == hash => {
                sig.consecutive_count += 1;
                sig.consecutive_count
            }
            _ => {
                self.current_signature = Some(ErrorSignature {
                    tool_name: tool_name.to_string(),
                    signature_hash: hash.clone(),
                    core_summary: summary.clone(),
                    consecutive_count: 1,
                });
                1
            }
        };

        if count >= self.threshold {
            self.state = CircuitBreakerState::Open;
            self.total_trips += 1;

            let directive = BreakoutDirective {
                state: CircuitBreakerState::Open,
                consecutive_failures: count,
                error_summary: summary.clone(),
                trip_reason: format!(
                    "【认知熔断器红牌爆发 (Circuit Breaker Tripped)】: 相同错误特征已连续触发 {} 次！核心报错: {}",
                    count, summary
                ),
                aha_epiphany_prompt: format!(
                    "[AHA-MOMENT / META-COGNITIVE EPIPHANY REQUIRED]\n\
                    STOP! You have failed {} consecutive times with the exact same error pattern: '{}'.\n\
                    Your current hypothesis is definitively FALSIFIED! Continuing down this path will only produce more corrupted code.\n\
                    Mandatory actions:\n\
                    1. Discard your current mental model completely.\n\
                    2. Revert dirty changes if any, and DO NOT edit the same lines repeatedly.\n\
                    3. Pivot to an entirely different dimension (e.g., check underlying type definitions, service boundary, or configuration).",
                    count, summary
                ),
                suggested_isolated_probe: "建议停止在主工程盲目修改代码！使用 synthesize_tool 或在 scratch/ 目录下编写最小可复现隔离探针排查根因。".to_string(),
                should_rollback_workspace: true,
            };

            return Some(directive);
        } else if count == self.threshold - 1 {
            self.state = CircuitBreakerState::HalfOpen;

            let directive = BreakoutDirective {
                state: CircuitBreakerState::HalfOpen,
                consecutive_failures: count,
                error_summary: summary.clone(),
                trip_reason: format!(
                    "【认知熔断器黄牌预警 (Circuit Breaker Warning)】: 相同错误已重复出现 2 次 ({})，当前假设正在走向死胡同！",
                    summary
                ),
                aha_epiphany_prompt: format!(
                    "[AWARENESS WARNING] The same error occurred twice. Reflect on whether your assumptions are fundamentally flawed before attempting another identical fix."
                ),
                suggested_isolated_probe: "如无充分把握，请通过搜索代码图谱或调用感知工具进一步确认事实。".to_string(),
                should_rollback_workspace: false,
            };

            return Some(directive);
        }

        None
    }

    /// 获取历史总熔断触发次数
    pub fn total_trips(&self) -> usize {
        self.total_trips
    }
}

fn sanitize_error_line(line: &str) -> String {
    let mut s = line.trim();
    // 剔除前缀方括号时间戳/日志级别，如 `[2026-10-22 14:00:01]` 或 `[INFO]`
    if s.starts_with('[') {
        if let Some(pos) = s.find(']') {
            s = s[pos + 1..].trim();
        }
    }
    // 剔除后缀位置信息，如 `in src/main.rs:12:5` 或 `at line 34`
    if let Some(pos) = s.find(" in ") {
        s = &s[..pos];
    } else if let Some(pos) = s.find(" at ") {
        s = &s[..pos];
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_signature_filters_noise() {
        let out1 = "[2026-10-22 14:00:01] error[E0308]: mismatched types in src/main.rs:12:5";
        let out2 = "[2026-10-22 14:00:05] error[E0308]: mismatched types in src/main.rs:18:9";

        let (hash1, sum1) = CircuitBreakerGuard::extract_signature("cargo_test", out1);
        let (hash2, sum2) = CircuitBreakerGuard::extract_signature("cargo_test", out2);

        assert_eq!(sum1, sum2);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_circuit_breaker_transitions_to_open() {
        let mut breaker = CircuitBreakerGuard::new(3);

        let error = "error[E0425]: cannot find value `foo` in this scope";

        // 第 1 次失败
        let d1 = breaker.record_failure("cargo_test", error);
        assert!(d1.is_none());
        assert_eq!(breaker.state, CircuitBreakerState::Closed);

        // 第 2 次失败 -> HalfOpen 黄牌预警
        let d2 = breaker.record_failure("cargo_test", error);
        assert!(d2.is_some());
        let d2_val = d2.unwrap();
        assert_eq!(d2_val.state, CircuitBreakerState::HalfOpen);
        assert!(!d2_val.should_rollback_workspace);

        // 第 3 次失败 -> Open 熔断红牌
        let d3 = breaker.record_failure("cargo_test", error);
        assert!(d3.is_some());
        let d3_val = d3.unwrap();
        assert_eq!(d3_val.state, CircuitBreakerState::Open);
        assert!(d3_val.should_rollback_workspace);
        assert!(d3_val.aha_epiphany_prompt.contains("AHA-MOMENT"));
        assert_eq!(breaker.total_trips(), 1);

        // 成功后恢复
        breaker.record_success();
        assert_eq!(breaker.state, CircuitBreakerState::Closed);
    }
}
