//! 科学假设-证伪推演工作流引擎 (Hypothesis-Driven Debugging Engine)
//! 杜绝在遇到 403、反爬风控、逆向失败或复杂报错时盲目试错污染主代码。
//! 强制执行：冻结主代码 -> 提出假说树 -> 隔离沙箱探针求证 -> 剪枝淘汰 -> 精准手术修补。

use serde::{Deserialize, Serialize};

/// 假说验证状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisStatus {
    /// 已提出待求证
    Formulated,
    /// 探针正在执行验证
    Probing { probe_file: String },
    /// 探针证实为真（确凿证据链成立）
    ConfirmedTrue { evidence: String },
    /// 探针证伪（已被淘汰剪枝）
    FalsifiedPruned { reason: String },
}

/// 单项假说实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisItem {
    pub id: String,
    pub title: String,
    pub rationale: String,
    pub minimal_probe_plan: String,
    pub status: HypothesisStatus,
}

/// 假说推演引擎
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisEngine {
    pub active: bool,
    pub trigger_error: Option<String>,
    pub hypotheses: Vec<HypothesisItem>,
    pub active_hypothesis_id: Option<String>,
    pub main_codebase_frozen: bool,
}

impl Default for HypothesisEngine {
    fn default() -> Self {
        Self {
            active: false,
            trigger_error: None,
            hypotheses: Vec::new(),
            active_hypothesis_id: None,
            main_codebase_frozen: false,
        }
    }
}

impl HypothesisEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 判定错误是否属于应触发科学假说推演的典型复杂阻断
    pub fn should_trigger(error_message: &str) -> bool {
        let msg = error_message.to_lowercase();
        msg.contains("403")
            || msg.contains("forbidden")
            || msg.contains("waf")
            || msg.contains("cloudflare")
            || msg.contains("sign")
            || msg.contains("signature")
            || msg.contains("token invalid")
            || msg.contains("anti-spider")
            || msg.contains("anti-bot")
            || msg.contains("captcha")
            || msg.contains("decrypt")
            || msg.contains("reverse engineering")
            || msg.contains("逆向")
            || msg.contains("签名")
            || msg.contains("反爬")
            || msg.contains("风控")
            || msg.contains("未授权")
    }

    /// 触发科学假说推演排错工作流，立即冻结主代码库
    pub fn trigger_investigation(&mut self, error_message: &str) {
        self.active = true;
        self.trigger_error = Some(error_message.to_string());
        // 铁律：一旦触发复杂深层错误，立即冻结主生产目录修改，防止盲目试错污染主代码
        self.main_codebase_frozen = true;
    }

    /// 注册一组互斥科学假说
    pub fn register_hypotheses(&mut self, hypotheses: Vec<HypothesisItem>) {
        if let Some(first) = hypotheses.first() {
            self.active_hypothesis_id = Some(first.id.clone());
        }
        self.hypotheses = hypotheses;
    }

    /// 校验即将修改的文件路径是否合法
    /// 若当前主代码被冻结，且修改路径不在 scratch/ 或 probes/ 下，则予以阻断！
    pub fn validate_mutation_path(&self, target_path: &str) -> Result<(), String> {
        if !self.main_codebase_frozen {
            return Ok(());
        }

        let normalized = target_path.replace('\\', "/").to_lowercase();
        let is_probe_file = normalized.contains("/scratch/")
            || normalized.contains("/probes/")
            || normalized.contains("/tmp/")
            || normalized.contains("probe_")
            || normalized.ends_with("_probe.py")
            || normalized.ends_with("_test.py");

        if !is_probe_file {
            return Err(format!(
                "【科学推演安全阻断 (Freeze Active)】：当前遭遇深度阻断（{}），主代码库已安全冻结！禁止直接在业务源码 '{}' 上盲目乱改！你必须先在 scratch/probes/ 编写并运行轻量独立探针脚本求证假说，证实有效后方可解锁修改主代码。",
                self.trigger_error.as_deref().unwrap_or("未知阻断"),
                target_path
            ));
        }

        Ok(())
    }

    /// 证实某个假说为真：解冻主代码库，允许实施针对性手术修补
    pub fn confirm_hypothesis(&mut self, hypothesis_id: &str, evidence: &str) {
        if let Some(hyp) = self.hypotheses.iter_mut().find(|h| h.id == hypothesis_id) {
            hyp.status = HypothesisStatus::ConfirmedTrue {
                evidence: evidence.to_string(),
            };
            self.main_codebase_frozen = false;
        }
    }

    /// 证伪某个假说：剪枝淘汰，自动推进到下一个待求证假说
    pub fn falsify_hypothesis(&mut self, hypothesis_id: &str, reason: &str) {
        if let Some(hyp) = self.hypotheses.iter_mut().find(|h| h.id == hypothesis_id) {
            hyp.status = HypothesisStatus::FalsifiedPruned {
                reason: reason.to_string(),
            };
        }

        // 切换到下一个未决假说
        self.active_hypothesis_id = self
            .hypotheses
            .iter()
            .find(|h| matches!(h.status, HypothesisStatus::Formulated))
            .map(|h| h.id.clone());
    }

    /// 生成针对模型的科学推演引导指令 (System Directive)
    pub fn render_guidance_prompt(&self) -> Option<String> {
        if !self.active {
            return None;
        }

        let mut lines = Vec::new();
        lines.push("\n[HYPOTHESIS-DRIVEN SCIENTIFIC DEBUGGING PROTOCOL]".to_string());
        if let Some(ref err) = self.trigger_error {
            lines.push(format!("Current Blocker: {}", err));
        }
        lines.push(format!(
            "Main Codebase Status: {}",
            if self.main_codebase_frozen {
                "🔒 FROZEN (No direct edits to production code allowed)"
            } else {
                "🔓 UNLOCKED (Hypothesis proven by probe evidence)"
            }
        ));

        if self.hypotheses.is_empty() {
            lines.push("Directive: Do NOT guess. Formulate 2~3 mutually exclusive hypotheses and design a minimal probe script in `scratch/probes/` to test Hypothesis #1.".to_string());
        } else {
            lines.push("Active Hypotheses Tree:".to_string());
            for h in &self.hypotheses {
                let status_str = match &h.status {
                    HypothesisStatus::Formulated => "⏳ Formulated (Pending)",
                    HypothesisStatus::Probing { probe_file } => &format!("🔍 Probing with {}", probe_file),
                    HypothesisStatus::ConfirmedTrue { evidence } => &format!("✅ PROVEN TRUE ({})", evidence),
                    HypothesisStatus::FalsifiedPruned { reason } => &format!("❌ FALSIFIED & PRUNED ({})", reason),
                };
                lines.push(format!("  - [{}] {}: {} -> {}", h.id, h.title, h.rationale, status_str));
            }
        }

        Some(lines.join("\n"))
    }

    /// 重置状态机
    pub fn reset(&mut self) {
        self.active = false;
        self.trigger_error = None;
        self.hypotheses.clear();
        self.active_hypothesis_id = None;
        self.main_codebase_frozen = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_detection() {
        assert!(HypothesisEngine::should_trigger("HTTP 403 Forbidden: Cloudflare WAF challenged"));
        assert!(HypothesisEngine::should_trigger("逆向失败：动态签名算法校验不匹配"));
        assert!(!HypothesisEngine::should_trigger("File not found"));
    }

    #[test]
    fn test_main_codebase_freeze_and_probe_exemption() {
        let mut engine = HypothesisEngine::new();
        engine.trigger_investigation("HTTP 403 Forbidden");

        // 试图直接修改主业务代码，必须被严格阻断
        let res_main = engine.validate_mutation_path("src/spider/douyin_collector.py");
        assert!(res_main.is_err());
        assert!(res_main.unwrap_err().contains("科学推演安全阻断"));

        // 在 scratch 或 probes 目录下编写探针脚本，允许放行
        let res_probe = engine.validate_mutation_path("scratch/probes/test_headers_probe.py");
        assert!(res_probe.is_ok());
    }

    #[test]
    fn test_hypothesis_falsify_and_confirm_workflow() {
        let mut engine = HypothesisEngine::new();
        engine.trigger_investigation("403 WAF Triggered");

        engine.register_hypotheses(vec![
            HypothesisItem {
                id: "hyp-1".into(),
                title: "缺少动态User-Agent与Referer".into(),
                rationale: "WAF依据默认请求头识别爬虫".into(),
                minimal_probe_plan: "scratch/probes/probe_headers.py".into(),
                status: HypothesisStatus::Formulated,
            },
            HypothesisItem {
                id: "hyp-2".into(),
                title: "请求参数缺少时间戳加盐签名".into(),
                rationale: "接口要求携带动态 _signature".into(),
                minimal_probe_plan: "scratch/probes/probe_sig.py".into(),
                status: HypothesisStatus::Formulated,
            },
        ]);

        // 探针测试假说1失败，证伪剪枝
        engine.falsify_hypothesis("hyp-1", "添加请求头后仍返回 403");
        assert!(matches!(engine.hypotheses[0].status, HypothesisStatus::FalsifiedPruned { .. }));
        assert_eq!(engine.active_hypothesis_id, Some("hyp-2".into()));
        assert!(engine.main_codebase_frozen);

        // 探针测试假说2成功，证实为真，主代码解冻！
        engine.confirm_hypothesis("hyp-2", "生成 _signature 探针成功拿到 200 OK 数据");
        assert!(matches!(engine.hypotheses[1].status, HypothesisStatus::ConfirmedTrue { .. }));
        assert!(!engine.main_codebase_frozen);

        // 此时允许修改主代码
        assert!(engine.validate_mutation_path("src/spider/douyin_collector.py").is_ok());
    }
}
