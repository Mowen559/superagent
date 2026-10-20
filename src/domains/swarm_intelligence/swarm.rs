//! 蜂群智能协同系统 (Swarm Intelligence & Multi-Agent Orchestrator)
//! 实现多智能体拓扑编排、角色分工与共识仲裁协议，对齐 RuFlo 蜂群核心架构。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use crate::domains::swarm_intelligence::swarm_channel::{ChannelMessageType, SwarmChannel};
use crate::domains::swarm_intelligence::swarm_heartbeat::{HeartbeatPulseResult, NodeHeartbeat, SwarmHeartbeatMonitor};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SwarmTopology {
    Hierarchical,
    Mesh,
    Star,
    Ring,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwarmRole {
    Leader,
    Architect,
    Coder,
    Reviewer,
    Tester,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmNode {
    pub id: String,
    pub name: String,
    pub role: SwarmRole,
    pub status: String, // "idle", "busy", "offline"
    pub capabilities: Vec<String>,
    pub load: f32,
    pub last_active: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConsensusStatus {
    Voting,
    Reached(String),
    Tie,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusProposal {
    pub id: String,
    pub topic: String,
    pub options: Vec<String>,
    pub votes: HashMap<String, String>, // node_id -> option
    pub status: ConsensusStatus,
    pub created_at: DateTime<Utc>,
    pub quorum: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmStatusReport {
    pub active_nodes: usize,
    pub total_nodes: usize,
    pub topology: SwarmTopology,
    pub strategy: String,
    pub nodes: Vec<SwarmNode>,
    pub active_proposals: Vec<ConsensusProposal>,
    pub uptime_seconds: u64,
}

/// 全局蜂群状态管理器
pub struct SwarmManager {
    topology: SwarmTopology,
    strategy: String,
    nodes: HashMap<String, SwarmNode>,
    proposals: HashMap<String, ConsensusProposal>,
    start_time: DateTime<Utc>,
    pub channel: SwarmChannel,
    pub heartbeat_monitor: SwarmHeartbeatMonitor,
}

impl SwarmManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            topology: SwarmTopology::Hierarchical,
            strategy: "balanced".to_string(),
            nodes: HashMap::new(),
            proposals: HashMap::new(),
            start_time: Utc::now(),
            channel: SwarmChannel::default(),
            heartbeat_monitor: SwarmHeartbeatMonitor::default(),
        };

        // 预设默认蜂群核心角色节点 (Leader, Coder, Reviewer, Tester)
        mgr.register_node(SwarmNode {
            id: "agent-leader".to_string(),
            name: "Commander Prime".to_string(),
            role: SwarmRole::Leader,
            status: "idle".to_string(),
            capabilities: vec!["planning".into(), "orchestration".into(), "consensus".into()],
            load: 0.0,
            last_active: Utc::now(),
            model_override: None,
        });
        mgr.register_node(SwarmNode {
            id: "agent-coder".to_string(),
            name: "Fullstack Synthesizer".to_string(),
            role: SwarmRole::Coder,
            status: "idle".to_string(),
            capabilities: vec!["file_edit".into(), "ast_grep".into(), "code_gen".into()],
            load: 0.0,
            last_active: Utc::now(),
            model_override: None,
        });
        mgr.register_node(SwarmNode {
            id: "agent-reviewer".to_string(),
            name: "Audit Sentinel".to_string(),
            role: SwarmRole::Reviewer,
            status: "idle".to_string(),
            capabilities: vec!["code_review".into(), "impact_radar".into(), "security".into()],
            load: 0.0,
            last_active: Utc::now(),
            model_override: None,
        });
        mgr.register_node(SwarmNode {
            id: "agent-tester".to_string(),
            name: "Verifier Drone".to_string(),
            role: SwarmRole::Tester,
            status: "idle".to_string(),
            capabilities: vec!["verify_task".into(), "test_runner".into(), "benchmark".into()],
            load: 0.0,
            last_active: Utc::now(),
            model_override: None,
        });

        mgr
    }

    pub fn register_node(&mut self, node: SwarmNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn remove_node(&mut self, node_id: &str) -> Option<SwarmNode> {
        self.nodes.remove(node_id)
    }

    pub fn init_swarm(&mut self, topology: Option<SwarmTopology>, strategy: Option<String>) {
        if let Some(t) = topology {
            self.topology = t;
        }
        if let Some(s) = strategy {
            self.strategy = s;
        }
    }

    pub fn submit_proposal(&mut self, topic: String, options: Vec<String>) -> String {
        let id = format!("prop-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let quorum = (self.nodes.len() / 2) + 1;
        let proposal = ConsensusProposal {
            id: id.clone(),
            topic,
            options,
            votes: HashMap::new(),
            status: ConsensusStatus::Voting,
            created_at: Utc::now(),
            quorum,
        };
        self.proposals.insert(id.clone(), proposal);
        id
    }

    pub fn cast_vote(&mut self, proposal_id: &str, node_id: &str, option: &str) -> Result<ConsensusStatus, String> {
        let prop = self.proposals.get_mut(proposal_id).ok_or("Proposal not found")?;
        if !prop.options.contains(&option.to_string()) {
            return Err("Invalid option".into());
        }
        prop.votes.insert(node_id.to_string(), option.to_string());

        // 计票
        let mut tally: HashMap<String, usize> = HashMap::new();
        for choice in prop.votes.values() {
            *tally.entry(choice.clone()).or_insert(0) += 1;
        }

        for (opt, count) in &tally {
            if *count >= prop.quorum {
                prop.status = ConsensusStatus::Reached(opt.clone());
                return Ok(prop.status.clone());
            }
        }

        if prop.votes.len() >= self.nodes.len() {
            prop.status = ConsensusStatus::Tie;
        }

        Ok(prop.status.clone())
    }

    /// 子智能体向蜂群中枢上报心跳
    pub fn report_node_heartbeat(&mut self, node_id: &str, status: &str, current_task: Option<String>, progress: f32) {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.status = status.to_string();
            node.last_active = Utc::now();
        }
        self.heartbeat_monitor.record_heartbeat(NodeHeartbeat {
            node_id: node_id.to_string(),
            status: status.to_string(),
            current_task,
            progress,
            timestamp: Utc::now(),
        });
    }

    /// 执行双轨心跳脉冲巡检：检查子节点健康 + 检查群聊阻塞与收敛
    pub fn pulse_heartbeat(&mut self) -> HeartbeatPulseResult {
        self.heartbeat_monitor.pulse(&mut self.nodes, &self.channel)
    }

    /// 向统一群聊信息区发布消息
    pub fn post_channel_message(
        &mut self,
        sender: &str,
        target: Option<&str>,
        msg_type: ChannelMessageType,
        summary: &str,
        payload: serde_json::Value,
        is_urgent: bool,
    ) -> String {
        self.channel.post_message(sender, target, msg_type, summary, payload, is_urgent)
    }

    /// 获取群聊最新结构化归纳提炼（供中枢大脑无痛阅读）
    pub fn get_channel_digest(&self) -> String {
        self.channel.generate_digest()
    }

    pub fn status_report(&self) -> SwarmStatusReport {
        let active = self.nodes.values().filter(|n| n.status != "offline").count();
        let uptime = (Utc::now() - self.start_time).num_seconds().max(0) as u64;
        SwarmStatusReport {
            active_nodes: active,
            total_nodes: self.nodes.len(),
            topology: self.topology.clone(),
            strategy: self.strategy.clone(),
            nodes: self.nodes.values().cloned().collect(),
            active_proposals: self.proposals.values().cloned().collect(),
            uptime_seconds: uptime,
        }
    }
}

lazy_static::lazy_static! {
    pub static ref GLOBAL_SWARM_MANAGER: Arc<RwLock<SwarmManager>> = Arc::new(RwLock::new(SwarmManager::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swarm_channel_and_heartbeat_integration() {
        let mut mgr = SwarmManager::new();

        // 1. 测试子智能体心跳上报
        mgr.report_node_heartbeat("agent-coder", "working", Some("编写爬虫模块".into()), 0.4);
        assert_eq!(mgr.nodes.get("agent-coder").unwrap().status, "working");

        // 2. 测试群聊信息区发布
        mgr.post_channel_message(
            "agent-coder",
            None,
            ChannelMessageType::Progress,
            "解析器已适配抖音 HTML 结构",
            serde_json::json!({ "progress": 0.4 }),
            false,
        );

        let digest = mgr.get_channel_digest();
        assert!(digest.contains("解析器已适配抖音 HTML 结构"));

        // 3. 执行双轨脉冲
        let pulse_res = mgr.pulse_heartbeat();
        assert!(!pulse_res.needs_brain_awakening);
        assert!(pulse_res.executive_summary.contains("运行健康"));
    }
}
