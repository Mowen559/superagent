//! 蜂群拓扑自重构与动态选举系统 (Swarm Topology Reconfiguration & Dynamic Election)
//!
//! 核心设计：
//! 1. 任务复杂度与拓扑画像剖析 (Task Complexity & Topology Profiling)；
//! 2. 动态领袖竞选算法 (Lead Agent Dynamic Election)：根据专长匹配度、当前负载与健康状态动态选帅；
//! 3. 拓扑自重构引擎 (Adaptive Swarm Topology Engine)：按需自适应切换 Star/Hierarchical/Mesh/Ring 拓扑；
//! 4. 任务契约拆解与消息总线分派 (Swarm Task Decomposition & Bus Dispatching)。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use crate::domains::swarm_intelligence::message_bus::{SwarmMessageBus, SwarmTaskItem};
use crate::domains::swarm_intelligence::swarm::{SwarmNode, SwarmRole, SwarmTopology};

/// 任务复杂度等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskComplexityLevel {
    /// 简单局部聚焦任务 (单文件微调/查阅) -> 推荐 Star/单兵
    FocusedLocal,
    /// 中等单模块任务 (涉及接口与单元测试) -> 推荐 Pipeline 流水线
    ModularExecution,
    /// 高度复杂跨服务协同 (跨前后端/端云协同) -> 推荐 Hierarchical 分层多节点
    CrossDomainCollaborative,
    /// 疑难逆向反爬/排错攻坚 -> 推荐 Mesh 对等网状推敲
    DeepAdversarialExploration,
}

/// 任务拓扑特征画像
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTopologyProfile {
    pub complexity_score: f32, // 0.0 ~ 1.0
    pub complexity_level: TaskComplexityLevel,
    pub involved_services: Vec<String>,
    pub recommended_topology: SwarmTopology,
    pub recommended_lead_capabilities: Vec<String>,
    pub rationale: String,
}

/// 候选节点竞选评分明细
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateElectionScore {
    pub node_id: String,
    pub node_name: String,
    pub total_score: f32,
    pub capability_match_score: f32,
    pub load_factor_score: f32,
    pub status_readiness_score: f32,
}

/// 领袖选举结果收据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElectionReceipt {
    pub elected_lead_id: String,
    pub elected_lead_name: String,
    pub term_id: String,
    pub elected_at: DateTime<Utc>,
    pub candidate_scores: Vec<CandidateElectionScore>,
    pub election_reason: String,
}

/// 子任务执行状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubTaskStatus {
    Pending,
    Dispatched,
    Running,
    Completed { summary: String },
    Failed { reason: String },
}

/// 蜂群拆解的子任务单元
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmSubTask {
    pub id: String,
    pub parent_task_id: String,
    pub title: String,
    pub assigned_role: SwarmRole,
    pub assigned_node_id: Option<String>,
    pub dependencies: Vec<String>, // 依赖的前置 subtask_id
    pub status: SubTaskStatus,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// 蜂群拓扑分析与自重构引擎
pub struct SwarmTopologyEngine;

impl SwarmTopologyEngine {
    /// 评估自然语言任务需求，生成自适应拓扑画像
    pub fn profile_task(prompt: &str) -> TaskTopologyProfile {
        let norm = prompt.to_lowercase();
        let mut involved_services = Vec::new();

        let service_keywords = [
            ("engine", "engine-service"),
            ("scraper", "scraper-service"),
            ("notification", "notification-service"),
            ("iam", "iam-service"),
            ("system", "system-service"),
            ("social", "social-service"),
            ("media", "media-service"),
            ("model", "model-service"),
            ("gateway", "gateway"),
            ("前端", "frontend"),
            ("ui", "frontend"),
            ("爬虫", "scraper-service"),
            ("通知", "notification-service"),
        ];

        for (kw, service) in &service_keywords {
            if norm.contains(kw) && !involved_services.contains(&service.to_string()) {
                involved_services.push(service.to_string());
            }
        }

        // 复杂度评分计算
        let mut complexity_score: f32 = 0.2; // 基础基线

        // 服务跨度加权
        if involved_services.len() > 1 {
            complexity_score += 0.35 + 0.15 * ((involved_services.len() - 1) as f32).min(2.0) / 2.0;
        }

        // 逆向/风控/攻坚特征
        let is_adversarial = contains_any(
            &norm,
            &["反爬", "逆向", "风控", "403", "captcha", "签名", "waf", "decrypt", "reverse"],
        );
        if is_adversarial {
            complexity_score += 0.35;
        }

        // 重构与端云协同特征
        let is_collaborative = contains_any(
            &norm,
            &["端云协同", "重构", "解耦", "蜂群", "分布式", "跨模块", "全栈", "pipeline"],
        );
        if is_collaborative {
            complexity_score += 0.3;
        }

        // 判定级别与拓扑
        let (complexity_level, recommended_topology, lead_caps, rationale) = if is_adversarial {
            (
                TaskComplexityLevel::DeepAdversarialExploration,
                SwarmTopology::Mesh,
                vec!["reverse_engineering".into(), "probing".into(), "hypothesis".into()],
                "检测到深层逆向/风控攻坚场景，启用对等网状拓扑 (MeshTopology)，多节点协同探针验证。".to_string(),
            )
        } else if involved_services.len() >= 2 || complexity_score >= 0.7 {
            (
                TaskComplexityLevel::CrossDomainCollaborative,
                SwarmTopology::Hierarchical,
                vec!["orchestration".into(), "planning".into(), "architecture".into()],
                format!(
                    "检测到跨跨域复合业务（涉及 {} 个模块），启用层级树状拓扑 (HierarchicalTopology) 统揽分派。",
                    involved_services.len()
                ),
            )
        } else if contains_any(&norm, &["测试", "验收", "verify", "benchmark", "回归"]) {
            (
                TaskComplexityLevel::ModularExecution,
                SwarmTopology::Ring,
                vec!["verification".into(), "test_runner".into()],
                "检测到流水线验收与验证任务，启用环状流水线拓扑 (RingTopology)。".to_string(),
            )
        } else {
            (
                TaskComplexityLevel::FocusedLocal,
                SwarmTopology::Star,
                vec!["surgical_execution".into(), "file_edit".into()],
                "单模块或局部聚焦任务，启用星型主从拓扑 (StarTopology)，减少通信开销快速交付。".to_string(),
            )
        };

        TaskTopologyProfile {
            complexity_score: complexity_score.min(1.0),
            complexity_level,
            involved_services,
            recommended_topology,
            recommended_lead_capabilities: lead_caps,
            rationale,
        }
    }

    /// 动态领袖竞选：根据候选节点的专长契合度、负载反比与就绪状态进行民主打分
    pub fn elect_lead(
        nodes: &HashMap<String, SwarmNode>,
        profile: &TaskTopologyProfile,
    ) -> Result<ElectionReceipt, String> {
        if nodes.is_empty() {
            return Err("Cannot elect lead: swarm node pool is completely empty!".to_string());
        }

        let mut candidate_scores = Vec::new();

        for node in nodes.values() {
            // 离线节点淘汰直接计 0 分
            if node.status == "offline" {
                candidate_scores.push(CandidateElectionScore {
                    node_id: node.id.clone(),
                    node_name: node.name.clone(),
                    total_score: 0.0,
                    capability_match_score: 0.0,
                    load_factor_score: 0.0,
                    status_readiness_score: 0.0,
                });
                continue;
            }

            // 1. 能力匹配度评分 (0.0 ~ 50.0 分)
            let mut matched_caps = 0;
            for req in &profile.recommended_lead_capabilities {
                if node.capabilities.iter().any(|c| c.contains(req) || req.contains(c)) {
                    matched_caps += 1;
                }
            }
            let capability_match_score = if profile.recommended_lead_capabilities.is_empty() {
                30.0
            } else {
                (matched_caps as f32 / profile.recommended_lead_capabilities.len() as f32) * 50.0
            };

            // 原生具备 Leader / Architect 角色的获得加分
            let role_bonus = match node.role {
                SwarmRole::Leader => 15.0,
                SwarmRole::Architect => 10.0,
                _ => 0.0,
            };

            // 2. 负载因子评分 (0.0 ~ 30.0 分)：负载越低得分越高
            let load_factor_score = (1.0 - node.load.clamp(0.0, 1.0)) * 30.0;

            // 3. 状态就绪度 (0.0 ~ 20.0 分)：空闲状态得分最高
            let status_readiness_score = match node.status.as_str() {
                "idle" => 20.0,
                "working" | "busy" => 8.0,
                _ => 2.0,
            };

            let total_score = capability_match_score + role_bonus + load_factor_score + status_readiness_score;

            candidate_scores.push(CandidateElectionScore {
                node_id: node.id.clone(),
                node_name: node.name.clone(),
                total_score,
                capability_match_score: capability_match_score + role_bonus,
                load_factor_score,
                status_readiness_score,
            });
        }

        // 按得分倒序排序
        candidate_scores.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap_or(std::cmp::Ordering::Equal));

        let winner = candidate_scores
            .first()
            .ok_or_else(|| "Election failed: no eligible candidates found.".to_string())?;

        let term_id = format!("term-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let election_reason = format!(
            "节点 '{}' ({}) 以综合最高分 {:.1} 分当选新任 Lead Agent。优势：能力契合度 {:.1}，负载加权 {:.1}，状态就绪 {:.1}。",
            winner.node_name, winner.node_id, winner.total_score,
            winner.capability_match_score, winner.load_factor_score, winner.status_readiness_score
        );

        Ok(ElectionReceipt {
            elected_lead_id: winner.node_id.clone(),
            elected_lead_name: winner.node_name.clone(),
            term_id,
            elected_at: Utc::now(),
            candidate_scores,
            election_reason,
        })
    }

    /// 自动将复合任务拆解为带契约依赖的蜂群子任务流
    pub fn decompose_task(parent_task_id: &str, profile: &TaskTopologyProfile) -> Vec<SwarmSubTask> {
        let mut subtasks = Vec::new();
        let now = Utc::now();

        match profile.complexity_level {
            TaskComplexityLevel::DeepAdversarialExploration => {
                let t1_id = format!("{}-probe", parent_task_id);
                let t2_id = format!("{}-analyze", parent_task_id);
                let t3_id = format!("{}-synthesize", parent_task_id);

                subtasks.push(SwarmSubTask {
                    id: t1_id.clone(),
                    parent_task_id: parent_task_id.to_string(),
                    title: "编写隔离沙箱探针脚本检测目标站点/接口响应".to_string(),
                    assigned_role: SwarmRole::Coder,
                    assigned_node_id: None,
                    dependencies: vec![],
                    status: SubTaskStatus::Pending,
                    payload: serde_json::json!({ "phase": "sandbox_probing" }),
                    created_at: now,
                });

                subtasks.push(SwarmSubTask {
                    id: t2_id.clone(),
                    parent_task_id: parent_task_id.to_string(),
                    title: "分析探针返回证据，假说验证并推演核心算法".to_string(),
                    assigned_role: SwarmRole::Architect,
                    assigned_node_id: None,
                    dependencies: vec![t1_id],
                    status: SubTaskStatus::Pending,
                    payload: serde_json::json!({ "phase": "hypothesis_confirmation" }),
                    created_at: now,
                });

                subtasks.push(SwarmSubTask {
                    id: t3_id,
                    parent_task_id: parent_task_id.to_string(),
                    title: "固化解密/反爬解析器至业务模块并进行回归验收".to_string(),
                    assigned_role: SwarmRole::Tester,
                    assigned_node_id: None,
                    dependencies: vec![t2_id],
                    status: SubTaskStatus::Pending,
                    payload: serde_json::json!({ "phase": "verification_settle" }),
                    created_at: now,
                });
            }
            TaskComplexityLevel::CrossDomainCollaborative => {
                let t1_id = format!("{}-arch-spec", parent_task_id);
                let t2_id = format!("{}-backend-code", parent_task_id);
                let t3_id = format!("{}-verify-test", parent_task_id);

                subtasks.push(SwarmSubTask {
                    id: t1_id.clone(),
                    parent_task_id: parent_task_id.to_string(),
                    title: "勘测微服务边界，设计 API 契约与 DTO 实体".to_string(),
                    assigned_role: SwarmRole::Architect,
                    assigned_node_id: None,
                    dependencies: vec![],
                    status: SubTaskStatus::Pending,
                    payload: serde_json::json!({ "services": profile.involved_services }),
                    created_at: now,
                });

                subtasks.push(SwarmSubTask {
                    id: t2_id.clone(),
                    parent_task_id: parent_task_id.to_string(),
                    title: "后端 Application 编排与 MyBatis-Plus 数据访问实现".to_string(),
                    assigned_role: SwarmRole::Coder,
                    assigned_node_id: None,
                    dependencies: vec![t1_id],
                    status: SubTaskStatus::Pending,
                    payload: serde_json::json!({ "framework": "Spring Boot 3.2" }),
                    created_at: now,
                });

                subtasks.push(SwarmSubTask {
                    id: t3_id,
                    parent_task_id: parent_task_id.to_string(),
                    title: "单元测试与存量回归物理硬闸门验收".to_string(),
                    assigned_role: SwarmRole::Tester,
                    assigned_node_id: None,
                    dependencies: vec![t2_id],
                    status: SubTaskStatus::Pending,
                    payload: serde_json::json!({ "gate": "DoD_Gate" }),
                    created_at: now,
                });
            }
            _ => {
                // FocusedLocal / ModularExecution: 经典单兵手术式交付
                subtasks.push(SwarmSubTask {
                    id: format!("{}-exec", parent_task_id),
                    parent_task_id: parent_task_id.to_string(),
                    title: "精确外科手术式代码实现与局部构建验证".to_string(),
                    assigned_role: SwarmRole::Coder,
                    assigned_node_id: None,
                    dependencies: vec![],
                    status: SubTaskStatus::Pending,
                    payload: serde_json::json!({ "mode": "surgical" }),
                    created_at: now,
                });
            }
        }

        subtasks
    }

    /// 通过底层统一 SwarmMessageBus 异步分派子任务
    pub async fn dispatch_subtasks_via_bus(
        subtasks: &[SwarmSubTask],
        bus: Arc<dyn SwarmMessageBus>,
    ) -> Result<usize, String> {
        let mut dispatched_count = 0;

        for task in subtasks {
            let role_queue = match task.assigned_role {
                SwarmRole::Leader => "swarm.task.leader",
                SwarmRole::Architect => "swarm.task.architect",
                SwarmRole::Coder => "swarm.task.coder",
                SwarmRole::Reviewer => "swarm.task.reviewer",
                SwarmRole::Tester => "swarm.task.tester",
                SwarmRole::Custom(ref s) => &format!("swarm.task.{}", s),
            };

            let item = SwarmTaskItem::new(
                role_queue,
                format!("{:?}", task.assigned_role),
                serde_json::to_value(task).unwrap_or_default(),
            );

            bus.enqueue_task(role_queue, item)
                .await
                .map_err(|e| format!("Failed to dispatch task '{}' to bus: {}", task.id, e))?;

            dispatched_count += 1;
        }

        Ok(dispatched_count)
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::swarm_intelligence::message_bus::LocalQueueBus;

    #[test]
    fn test_task_profiling_cross_domain() {
        let prompt = "需要同时在 engine-service 增加任务调度并在 scraper-service 客户端边缘引擎增加对应抓取处理";
        let profile = SwarmTopologyEngine::profile_task(prompt);

        assert!(profile.complexity_score >= 0.5);
        assert_eq!(profile.complexity_level, TaskComplexityLevel::CrossDomainCollaborative);
        assert_eq!(profile.recommended_topology, SwarmTopology::Hierarchical);
        assert!(profile.involved_services.contains(&"engine-service".to_string()));
        assert!(profile.involved_services.contains(&"scraper-service".to_string()));
    }

    #[test]
    fn test_task_profiling_adversarial_exploration() {
        let prompt = "分析抖音网页端 403 Forbidden 报错，反爬风控签名生成与 JS 逆向解密";
        let profile = SwarmTopologyEngine::profile_task(prompt);

        assert_eq!(profile.complexity_level, TaskComplexityLevel::DeepAdversarialExploration);
        assert_eq!(profile.recommended_topology, SwarmTopology::Mesh);
        assert!(profile.recommended_lead_capabilities.contains(&"reverse_engineering".to_string()));
    }

    #[test]
    fn test_dynamic_lead_election() {
        let mut nodes = HashMap::new();

        nodes.insert(
            "node-coder".to_string(),
            SwarmNode {
                id: "node-coder".to_string(),
                name: "Coder Drone".to_string(),
                role: SwarmRole::Coder,
                status: "idle".to_string(),
                capabilities: vec!["file_edit".into(), "code_gen".into()],
                load: 0.1,
                last_active: Utc::now(),
                model_override: None,
            },
        );

        nodes.insert(
            "node-commander".to_string(),
            SwarmNode {
                id: "node-commander".to_string(),
                name: "Architect Prime".to_string(),
                role: SwarmRole::Architect,
                status: "idle".to_string(),
                capabilities: vec!["orchestration".into(), "planning".into(), "architecture".into()],
                load: 0.05,
                last_active: Utc::now(),
                model_override: None,
            },
        );

        let profile = TaskTopologyProfile {
            complexity_score: 0.8,
            complexity_level: TaskComplexityLevel::CrossDomainCollaborative,
            involved_services: vec!["engine-service".into(), "notification-service".into()],
            recommended_topology: SwarmTopology::Hierarchical,
            recommended_lead_capabilities: vec!["orchestration".into(), "planning".into()],
            rationale: "Cross domain".into(),
        };

        let election = SwarmTopologyEngine::elect_lead(&nodes, &profile).unwrap();
        assert_eq!(election.elected_lead_id, "node-commander");
        assert!(election.election_reason.contains("Architect Prime"));
    }

    #[tokio::test]
    async fn test_task_decomposition_and_bus_dispatch() {
        let profile = TaskTopologyProfile {
            complexity_score: 0.85,
            complexity_level: TaskComplexityLevel::CrossDomainCollaborative,
            involved_services: vec!["engine-service".into(), "scraper-service".into()],
            recommended_topology: SwarmTopology::Hierarchical,
            recommended_lead_capabilities: vec!["orchestration".into()],
            rationale: "test".into(),
        };

        let subtasks = SwarmTopologyEngine::decompose_task("task-1001", &profile);
        assert_eq!(subtasks.len(), 3);

        let bus = Arc::new(LocalQueueBus::default());
        let count = SwarmTopologyEngine::dispatch_subtasks_via_bus(&subtasks, bus.clone()).await.unwrap();
        assert_eq!(count, 3);

        // 验证消息已投递至总线
        let coder_task = bus.poll_task("swarm.task.coder", 5).await.unwrap();
        assert!(coder_task.is_some());
        let item = coder_task.unwrap();
        assert!(item.payload.to_string().contains("task-1001-backend-code"));
    }
}
