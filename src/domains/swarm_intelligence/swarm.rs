//! 蜂群智能协同系统 (Swarm Intelligence & Multi-Agent Orchestrator)
//! 实现多智能体拓扑编排、角色分工与共识仲裁协议，对齐 RuFlo 蜂群核心架构。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

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
}

impl SwarmManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            topology: SwarmTopology::Hierarchical,
            strategy: "balanced".to_string(),
            nodes: HashMap::new(),
            proposals: HashMap::new(),
            start_time: Utc::now(),
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
        });
        mgr.register_node(SwarmNode {
            id: "agent-coder".to_string(),
            name: "Fullstack Synthesizer".to_string(),
            role: SwarmRole::Coder,
            status: "idle".to_string(),
            capabilities: vec!["file_edit".into(), "ast_grep".into(), "code_gen".into()],
            load: 0.0,
            last_active: Utc::now(),
        });
        mgr.register_node(SwarmNode {
            id: "agent-reviewer".to_string(),
            name: "Audit Sentinel".to_string(),
            role: SwarmRole::Reviewer,
            status: "idle".to_string(),
            capabilities: vec!["code_review".into(), "impact_radar".into(), "security".into()],
            load: 0.0,
            last_active: Utc::now(),
        });
        mgr.register_node(SwarmNode {
            id: "agent-tester".to_string(),
            name: "Verifier Drone".to_string(),
            role: SwarmRole::Tester,
            status: "idle".to_string(),
            capabilities: vec!["verify_task".into(), "test_runner".into(), "benchmark".into()],
            load: 0.0,
            last_active: Utc::now(),
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
