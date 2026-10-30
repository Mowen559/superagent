//! 蜂群双轨心跳监视系统 (Dual-Aspect Swarm Heartbeat & Pulse Monitor)
//! 轨道 A：实时检测子智能体存活状态、TTL 假死/失联超时、任务防死锁；
//! 轨道 B：实时侦测统一群聊信息区的未决阻断告警、多智能体任务收敛性，并主动唤醒中枢大脑。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::swarm::SwarmNode;
use super::swarm_channel::SwarmChannel;

/// 子智能体上报的心跳载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeartbeat {
    pub node_id: String,
    pub status: String, // "idle", "working", "stuck", "error"
    pub current_task: Option<String>,
    pub progress: f32,
    pub timestamp: DateTime<Utc>,
}

/// 心跳脉冲全身体检报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatPulseResult {
    pub timestamp: DateTime<Utc>,
    pub total_nodes: usize,
    pub active_nodes: usize,
    /// 假死/失联超时被标记淘汰的节点 ID
    pub zombie_nodes: Vec<String>,
    /// 统一群聊信息区中未解决的紧急求助
    pub urgent_blockers: Vec<String>,
    /// 是否需要主动唤醒中枢大脑介入仲裁或调控
    pub needs_brain_awakening: bool,
    /// 综合体检诊断概要
    pub executive_summary: String,
}

/// 双轨心跳监视控制器
pub struct SwarmHeartbeatMonitor {
    worker_timeout_secs: i64,
    last_heartbeats: HashMap<String, NodeHeartbeat>,
}

impl Default for SwarmHeartbeatMonitor {
    fn default() -> Self {
        Self::new(30) // 默认 30 秒无心跳视为失联/假死
    }
}

impl SwarmHeartbeatMonitor {
    pub fn new(worker_timeout_secs: i64) -> Self {
        Self {
            worker_timeout_secs,
            last_heartbeats: HashMap::new(),
        }
    }

    /// 接收子智能体发射的心跳
    pub fn record_heartbeat(&mut self, hb: NodeHeartbeat) {
        self.last_heartbeats.insert(hb.node_id.clone(), hb);
    }

    /// 执行一次完整双轨心跳脉冲巡检 (Dual-Aspect Pulse Check)
    pub fn pulse(
        &mut self,
        nodes: &mut HashMap<String, SwarmNode>,
        channel: &SwarmChannel,
    ) -> HeartbeatPulseResult {
        let now = Utc::now();
        let mut zombie_nodes = Vec::new();

        // ================= 轨道 A：子智能体活跃度与假死侦测 =================
        for (node_id, node) in nodes.iter_mut() {
            // 跳过中枢大脑自身检测
            if node_id == "agent-leader" {
                continue;
            }

            let last_seen = self
                .last_heartbeats
                .get(node_id)
                .map(|h| h.timestamp)
                .unwrap_or(node.last_active);

            let elapsed_secs = (now - last_seen).num_seconds();

            // 若节点处于工作状态但超过 TTL 未发送心跳，判定为假死 (Zombie)
            if node.status == "working" || node.status == "busy" {
                if elapsed_secs > self.worker_timeout_secs {
                    node.status = "zombie".to_string();
                    zombie_nodes.push(node_id.clone());
                    eprintln!(
                        "\x1b[31;1m[Swarm Heartbeat] ⚠️ 侦测到工蜂子节点 [{}] 假死 (已无响应 {} 秒)，自动标记为 Zombie 并释放任务！\x1b[0m",
                        node_id, elapsed_secs
                    );
                }
            }
        }

        // ================= 轨道 B：统一群聊信息区动态侦测 =================
        let blockers = channel.get_active_blockers();
        let mut urgent_blocker_summaries = Vec::new();
        for b in &blockers {
            urgent_blocker_summaries.push(format!("[{}] {}", b.sender, b.summary));
        }

        // 判定是否需要主动唤醒中枢智能体大脑
        // 条件：存在未处理的求助阻断，或者有工蜂假死需要重新派工
        let needs_brain_awakening = !zombie_nodes.is_empty() || !urgent_blockers_empty(&urgent_blocker_summaries);

        let active_nodes = nodes
            .values()
            .filter(|n| n.status != "offline" && n.status != "zombie")
            .count();

        let executive_summary = if !zombie_nodes.is_empty() {
            format!(
                "警报：发现 {} 个假死节点 ({:?})，已触发中枢故障转移！",
                zombie_nodes.len(),
                zombie_nodes
            )
        } else if !urgent_blocker_summaries.is_empty() {
            format!(
                "关注：统一信息区存在 {} 起紧急求助/阻断，需中枢介入仲裁！",
                urgent_blocker_summaries.len()
            )
        } else {
            format!(
                "平稳：全部 {} 个子节点与群聊信息区运行健康，活跃工蜂数: {}。",
                nodes.len(),
                active_nodes
            )
        };

        HeartbeatPulseResult {
            timestamp: now,
            total_nodes: nodes.len(),
            active_nodes,
            zombie_nodes,
            urgent_blockers: urgent_blocker_summaries,
            needs_brain_awakening,
            executive_summary,
        }
    }
}

fn urgent_blockers_empty(blockers: &[String]) -> bool {
    blockers.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::swarm_intelligence::swarm::{SwarmNode, SwarmRole};
    use crate::domains::swarm_intelligence::swarm_channel::{ChannelMessageType, SwarmChannel};

    #[test]
    fn test_pulse_detects_healthy_swarm() {
        let mut monitor = SwarmHeartbeatMonitor::new(10);
        let mut nodes = HashMap::new();
        let mut channel = SwarmChannel::default();

        nodes.insert(
            "worker-1".to_string(),
            SwarmNode {
                id: "worker-1".to_string(),
                name: "Worker 1".to_string(),
                role: SwarmRole::Coder,
                status: "idle".to_string(),
                capabilities: vec![],
                load: 0.0,
                last_active: Utc::now(),
                model_override: None,
            },
        );

        let report = monitor.pulse(&mut nodes, &channel);
        assert_eq!(report.active_nodes, 1);
        assert!(report.zombie_nodes.is_empty());
        assert!(!report.needs_brain_awakening);
        assert!(report.executive_summary.contains("运行健康"));
    }

    #[test]
    fn test_pulse_detects_zombie_and_awakens_brain() {
        let mut monitor = SwarmHeartbeatMonitor::new(5); // 5秒超时
        let mut nodes = HashMap::new();
        let channel = SwarmChannel::default();

        let dead_time = Utc::now() - chrono::Duration::seconds(10);
        nodes.insert(
            "worker-stuck".to_string(),
            SwarmNode {
                id: "worker-stuck".to_string(),
                name: "Stuck Worker".to_string(),
                role: SwarmRole::Coder,
                status: "working".to_string(), // 假死中
                capabilities: vec![],
                load: 1.0,
                last_active: dead_time,
                model_override: None,
            },
        );

        let report = monitor.pulse(&mut nodes, &channel);
        assert_eq!(report.zombie_nodes, vec!["worker-stuck"]);
        assert!(report.needs_brain_awakening);
        assert_eq!(nodes.get("worker-stuck").unwrap().status, "zombie");
    }

    #[test]
    fn test_pulse_detects_channel_blockers_and_awakens_brain() {
        let mut monitor = SwarmHeartbeatMonitor::new(30);
        let mut nodes = HashMap::new();
        let mut channel = SwarmChannel::default();

        nodes.insert(
            "worker-1".to_string(),
            SwarmNode {
                id: "worker-1".to_string(),
                name: "Worker 1".to_string(),
                role: SwarmRole::Coder,
                status: "idle".to_string(),
                capabilities: vec![],
                load: 0.0,
                last_active: Utc::now(),
                model_override: None,
            },
        );

        // 群聊信息区收到求助
        channel.post_message(
            "worker-1",
            None,
            ChannelMessageType::BlockerAlert,
            "逆向 JS 混淆失败，请求 Leader 调度大模型求解",
            serde_json::json!({ "error": "deobfuscation_failed" }),
            true,
        );

        let report = monitor.pulse(&mut nodes, &channel);
        assert!(report.zombie_nodes.is_empty());
        assert_eq!(report.urgent_blockers.len(), 1);
        assert!(report.needs_brain_awakening);
        assert!(report.executive_summary.contains("紧急求助/阻断"));
    }
}
