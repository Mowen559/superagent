//! 蜂群统一信令大厅与群聊信息区 (Swarm Channel / Shared Group Chat Hub)
//! 提供多智能体透明协作大厅、高信噪比进展同步、阻塞事件实时广播与自动信息归纳。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 蜂群群聊消息类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMessageType {
    /// 阶段进展汇报
    Progress,
    /// 遭遇阻塞/故障求助
    BlockerAlert,
    /// 技术假说/方案提议
    Hypothesis,
    /// 中枢大脑全局战略指令
    BrainDirective,
    /// 方案投票共识
    ConsensusCall,
    /// 节点健康/状态系统广播
    SystemNotice,
}

/// 群聊消息实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub id: String,
    pub sender: String,
    /// 目标接收方（None 为广播给所有智能体，Some 为定向指令）
    pub target: Option<String>,
    pub msg_type: ChannelMessageType,
    pub summary: String,
    pub payload: serde_json::Value,
    pub is_urgent: bool,
    pub created_at: DateTime<Utc>,
    pub consumed_by_brain: bool,
}

/// 统一信令大厅 / 群聊信息流容器
#[derive(Debug, Clone)]
pub struct SwarmChannel {
    messages: Vec<ChannelMessage>,
    max_history: usize,
}

impl Default for SwarmChannel {
    fn default() -> Self {
        Self::new(500)
    }
}

impl SwarmChannel {
    pub fn new(max_history: usize) -> Self {
        Self {
            messages: Vec::with_capacity(max_history),
            max_history,
        }
    }

    /// 发送消息进入群聊统一信息区
    pub fn post_message(
        &mut self,
        sender: &str,
        target: Option<&str>,
        msg_type: ChannelMessageType,
        summary: &str,
        payload: serde_json::Value,
        is_urgent: bool,
    ) -> String {
        let id = format!("msg-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let msg = ChannelMessage {
            id: id.clone(),
            sender: sender.to_string(),
            target: target.map(|s| s.to_string()),
            msg_type,
            summary: summary.to_string(),
            payload,
            is_urgent,
            created_at: Utc::now(),
            consumed_by_brain: false,
        };

        self.messages.push(msg);
        if self.messages.len() > self.max_history {
            self.messages.remove(0);
        }
        id
    }

    /// 获取所有尚未被中枢大脑消费/审阅的消息
    pub fn get_unread_for_brain(&self) -> Vec<ChannelMessage> {
        self.messages
            .iter()
            .filter(|m| !m.consumed_by_brain)
            .cloned()
            .collect()
    }

    /// 获取当前信息区中未解决的紧急阻断消息 (Blockers)
    pub fn get_active_blockers(&self) -> Vec<ChannelMessage> {
        self.messages
            .iter()
            .filter(|m| m.msg_type == ChannelMessageType::BlockerAlert && !m.consumed_by_brain)
            .cloned()
            .collect()
    }

    /// 标记一组消息已经被中枢大脑消费/消化
    pub fn mark_as_consumed(&mut self, ids: &[String]) {
        for msg in &mut self.messages {
            if ids.contains(&msg.id) {
                msg.consumed_by_brain = true;
            }
        }
    }

    /// 获取最近 N 条群聊消息
    pub fn recent_messages(&self, limit: usize) -> Vec<ChannelMessage> {
        let count = limit.min(self.messages.len());
        self.messages.iter().rev().take(count).cloned().collect()
    }

    /// 自动信息归纳提炼引擎 (Digest Engine)
    /// 将群聊信息流中的零散日志压缩为高信噪比的战略摘要，保护中枢大脑上下文
    pub fn generate_digest(&self) -> String {
        let unread = self.get_unread_for_brain();
        if unread.is_empty() {
            return "【蜂群统一信息区】当前无新消息，所有子智能体运行平稳。".to_string();
        }

        let mut blockers = Vec::new();
        let mut progress = Vec::new();
        let mut directives = Vec::new();
        let mut others = Vec::new();

        for m in &unread {
            match m.msg_type {
                ChannelMessageType::BlockerAlert => blockers.push(format!("- [!] [{}] {}", m.sender, m.summary)),
                ChannelMessageType::Progress => progress.push(format!("- [*] [{}] {}", m.sender, m.summary)),
                ChannelMessageType::BrainDirective => directives.push(format!("- [>] [{}] {}", m.sender, m.summary)),
                _ => others.push(format!("- [-] [{}] {}", m.sender, m.summary)),
            }
        }

        let mut lines = Vec::new();
        lines.push(format!("【蜂群统一信息区最新归纳 ({} 条未读)】", unread.len()));
        if !blockers.is_empty() {
            lines.push(format!("⚠️ 紧急求助/阻断 (共 {} 件，需中枢立即介入):", blockers.len()));
            lines.extend(blockers);
        }
        if !progress.is_empty() {
            lines.push(format!("🟢 阶段成果与进展 (共 {} 条):", progress.len()));
            lines.extend(progress);
        }
        if !directives.is_empty() {
            lines.push(format!("🎯 已下达中枢指令 (共 {} 条):", directives.len()));
            lines.extend(directives);
        }
        if !others.is_empty() {
            lines.push(format!("💬 其他交流/提案 (共 {} 条):", others.len()));
            lines.extend(others);
        }

        lines.join("\n")
    }

    pub fn total_messages(&self) -> usize {
        self.messages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_post_and_unread_flow() {
        let mut channel = SwarmChannel::default();
        assert_eq!(channel.total_messages(), 0);

        channel.post_message(
            "agent-coder",
            None,
            ChannelMessageType::Progress,
            "爬虫基础脚本完成 80%",
            serde_json::json!({ "progress": 0.8 }),
            false,
        );

        channel.post_message(
            "agent-tester",
            Some("agent-leader"),
            ChannelMessageType::BlockerAlert,
            "遇到 403 WAF 阻断，请求中枢仲裁",
            serde_json::json!({ "http_status": 403 }),
            true,
        );

        let unread = channel.get_unread_for_brain();
        assert_eq!(unread.len(), 2);

        let blockers = channel.get_active_blockers();
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].sender, "agent-tester");

        // 生成高信噪比摘要
        let digest = channel.generate_digest();
        assert!(digest.contains("紧急求助/阻断"));
        assert!(digest.contains("爬虫基础脚本完成 80%"));

        // 标记消费
        let ids: Vec<String> = unread.iter().map(|m| m.id.clone()).collect();
        channel.mark_as_consumed(&ids);

        assert_eq!(channel.get_unread_for_brain().len(), 0);
        assert_eq!(channel.get_active_blockers().len(), 0);
    }
}
