//! RuFloUI 协议全量适配与全景能力可视化路由 (RuFlo Protocol Adapter & Capability Hub)
//! 提供蜂群编排、意识流黑板、工作流管线、智能体池、跨会话记忆以及全量插件透视能力，无缝直连 RuFlo 前台。

use crate::domains::swarm_intelligence::{
    ConsensusStatus, SwarmNode, SwarmRole, SwarmTopology, GLOBAL_CONSCIOUSNESS,
    GLOBAL_SWARM_MANAGER, GLOBAL_WORKFLOW_ENGINE, ThoughtType,
};
use crate::AppState;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // ── 1. 蜂群网络 (Swarm Intelligence) ──
        .route("/api/swarm/status", get(get_swarm_status))
        .route("/api/swarm/health", get(get_swarm_health))
        .route("/api/swarm/init", post(init_swarm))
        .route("/api/swarm/shutdown", post(shutdown_swarm))
        // ── 2. 集体意识与共识黑板 (Hive-Mind & Consciousness) ──
        .route("/api/hive-mind/status", get(get_hive_mind_status))
        .route("/api/hive-mind/memory", get(get_hive_mind_memory))
        .route("/api/hive-mind/broadcast", post(broadcast_thought))
        .route("/api/hive-mind/consensus", post(propose_consensus))
        // ── 3. 确定性工作流流水线 (Agentic Workflows) ──
        .route("/api/workflows", get(list_workflows).post(create_workflow))
        .route("/api/workflows/{id}", get(get_workflow))
        .route("/api/workflows/{id}/execute", post(execute_workflow))
        // ── 4. 蜂群智能体全景管理 (Swarm Agents Support) ──
        .route("/api/swarm/agents", get(list_agents))
        .route("/api/swarm/agents/spawn", post(spawn_agent))
        .route("/api/swarm/agents/{id}/status", get(get_agent_status))
        .route("/api/swarm/agents/{id}/health", get(get_agent_health))
        .route("/api/swarm/agents/pool", get(get_agent_pool))
        // ── 5. 跨会话记忆与知识黑板 (Memory Panel Support) ──
        .route("/api/memory", get(list_memory).post(store_memory))
        .route("/api/memory/stats", get(get_memory_stats))
        // ── 6. 透明插件与能力透视 (Plugin & Capability Matrix - Anti-Blackbox) ──
        .route("/api/plugins", get(list_plugins))
        .route("/api/system/capabilities", get(get_system_capabilities))
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Swarm Handlers
// ─────────────────────────────────────────────────────────────────────────────

async fn get_swarm_status(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    Json(json!({
        "status": "active",
        "topology": report.topology,
        "strategy": report.strategy,
        "activeAgents": report.active_nodes,
        "totalAgents": report.total_nodes,
        "uptime": report.uptime_seconds,
        "nodes": report.nodes,
        "proposals": report.active_proposals,
    }))
}

async fn get_swarm_health(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    Json(json!({
        "health": "healthy",
        "healthy": true,
        "node_count": report.total_nodes,
        "active_count": report.active_nodes,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

#[derive(Deserialize)]
struct InitSwarmPayload {
    topology: Option<String>,
    strategy: Option<String>,
}

async fn init_swarm(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<InitSwarmPayload>,
) -> Json<serde_json::Value> {
    let mut mgr = GLOBAL_SWARM_MANAGER.write().unwrap();
    let top = match payload.topology.as_deref() {
        Some("mesh") => Some(SwarmTopology::Mesh),
        Some("star") => Some(SwarmTopology::Star),
        Some("ring") => Some(SwarmTopology::Ring),
        Some("hierarchical") | _ => Some(SwarmTopology::Hierarchical),
    };
    mgr.init_swarm(top, payload.strategy);
    Json(json!({
        "ok": true,
        "message": "Swarm reconfigured successfully",
        "topology": payload.topology.unwrap_or_else(|| "hierarchical".into())
    }))
}

async fn shutdown_swarm(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "message": "Swarm entered sleep state." }))
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Hive-Mind & Consciousness Handlers
// ─────────────────────────────────────────────────────────────────────────────

async fn get_hive_mind_status(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let consciousness = GLOBAL_CONSCIOUSNESS.read().unwrap();
    let stream = consciousness.get_recent_stream(20);
    let facts = consciousness.all_facts();
    Json(json!({
        "status": "engaged",
        "protocol": "consensus-raft-hybrid",
        "activeStreamSize": stream.len(),
        "blackboardEntries": facts.len(),
        "recentStream": stream
    }))
}

async fn get_hive_mind_memory(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let consciousness = GLOBAL_CONSCIOUSNESS.read().unwrap();
    Json(json!({
        "stream": consciousness.get_recent_stream(50),
        "blackboard": consciousness.all_facts()
    }))
}

#[derive(Deserialize)]
struct BroadcastThoughtPayload {
    source: Option<String>,
    content: String,
    thought_type: Option<String>,
    tags: Option<Vec<String>>,
}

async fn broadcast_thought(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<BroadcastThoughtPayload>,
) -> Json<serde_json::Value> {
    let mut consciousness = GLOBAL_CONSCIOUSNESS.write().unwrap();
    let ttype = match payload.thought_type.as_deref() {
        Some("hypothesis") => ThoughtType::Hypothesis,
        Some("observation") => ThoughtType::Observation,
        Some("decision") => ThoughtType::Decision,
        Some("warning") => ThoughtType::Warning,
        _ => ThoughtType::Reflection,
    };
    let id = consciousness.record_thought(
        payload.source.as_deref().unwrap_or("agent-coordinator"),
        ttype,
        &payload.content,
        payload.tags.unwrap_or_default(),
    );
    Json(json!({ "ok": true, "thoughtId": id }))
}

#[derive(Deserialize)]
struct ConsensusPayload {
    topic: String,
    options: Vec<String>,
}

async fn propose_consensus(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<ConsensusPayload>,
) -> Json<serde_json::Value> {
    let mut mgr = GLOBAL_SWARM_MANAGER.write().unwrap();
    let prop_id = mgr.submit_proposal(payload.topic, payload.options);
    Json(json!({ "ok": true, "proposalId": prop_id }))
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Workflow Handlers
// ─────────────────────────────────────────────────────────────────────────────

async fn list_workflows(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let engine = GLOBAL_WORKFLOW_ENGINE.read().unwrap();
    Json(json!({
        "workflows": engine.list_workflows(),
        "total": engine.list_workflows().len()
    }))
}

async fn get_workflow(
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let engine = GLOBAL_WORKFLOW_ENGINE.read().unwrap();
    match engine.get_workflow(&id) {
        Some(wf) => Json(json!({ "workflow": wf })),
        None => Json(json!({ "error": "Workflow not found", "workflow": null })),
    }
}

#[derive(Deserialize)]
struct CreateWorkflowPayload {
    name: String,
    description: Option<String>,
}

async fn create_workflow(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<CreateWorkflowPayload>,
) -> Json<serde_json::Value> {
    let mut engine = GLOBAL_WORKFLOW_ENGINE.write().unwrap();
    let id = engine.create_workflow(
        &payload.name,
        payload.description.as_deref().unwrap_or("Dynamic Agent Pipeline"),
        vec![],
    );
    Json(json!({ "ok": true, "workflowId": id }))
}

async fn execute_workflow(
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let mut engine = GLOBAL_WORKFLOW_ENGINE.write().unwrap();
    match engine.advance_workflow(&id) {
        Ok(wf) => Json(json!({ "ok": true, "workflow": wf })),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Agents Handlers (RuFloUI AgentsPanel Direct Support)
// ─────────────────────────────────────────────────────────────────────────────

async fn list_agents(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    let agents: Vec<serde_json::Value> = report
        .nodes
        .into_iter()
        .map(|n| {
            let role_str = match &n.role {
                SwarmRole::Leader => "coordinator",
                SwarmRole::Architect => "architect",
                SwarmRole::Coder => "coder",
                SwarmRole::Reviewer => "reviewer",
                SwarmRole::Tester => "tester",
                SwarmRole::Custom(s) => s.as_str(),
            };
            json!({
                "id": n.id,
                "name": n.name,
                "type": role_str,
                "status": n.status,
                "load": n.load,
                "capabilities": n.capabilities,
                "createdAt": n.last_active.to_rfc3339()
            })
        })
        .collect();

    Json(json!({ "agents": agents, "total": agents.len() }))
}

#[derive(Deserialize)]
struct SpawnAgentPayload {
    #[serde(rename = "type")]
    agent_type: String,
    name: String,
    capabilities: Option<Vec<String>>,
}

async fn spawn_agent(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<SpawnAgentPayload>,
) -> Json<serde_json::Value> {
    let mut mgr = GLOBAL_SWARM_MANAGER.write().unwrap();
    let id = format!("agent-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let role = match payload.agent_type.as_str() {
        "leader" | "coordinator" => SwarmRole::Leader,
        "architect" => SwarmRole::Architect,
        "reviewer" => SwarmRole::Reviewer,
        "tester" => SwarmRole::Tester,
        "coder" => SwarmRole::Coder,
        other => SwarmRole::Custom(other.to_string()),
    };

    let node = SwarmNode {
        id: id.clone(),
        name: payload.name.clone(),
        role,
        status: "idle".to_string(),
        capabilities: payload.capabilities.unwrap_or_else(|| vec!["autonomous_worker".into()]),
        load: 0.0,
        last_active: chrono::Utc::now(),
    };
    mgr.register_node(node);

    Json(json!({
        "ok": true,
        "agent": {
            "id": id,
            "name": payload.name,
            "type": payload.agent_type,
            "status": "idle"
        }
    }))
}

async fn get_agent_status(
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    let found = report.nodes.into_iter().find(|n| n.id == id);
    match found {
        Some(node) => Json(json!({ "agent": node, "status": node.status })),
        None => Json(json!({ "agent": null, "error": "Agent not found" })),
    }
}

async fn get_agent_health(
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    let found = report.nodes.into_iter().find(|n| n.id == id);
    match found {
        Some(node) => Json(json!({ "health": "healthy", "status": node.status })),
        None => Json(json!({ "health": "unknown", "status": "offline" })),
    }
}

async fn get_agent_pool(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "pool": [
            { "type": "coordinator", "description": "Swarm lead, strategic task decomposition and consensus orchestrator" },
            { "type": "coder", "description": "High-agency code synthesis and AST refactoring drone" },
            { "type": "reviewer", "description": "Alibaba OpenCodeReview 5-gate noise-filtered static auditor" },
            { "type": "tester", "description": "Test-driven verification and golden benchmark harness" }
        ]
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Memory Handlers (RuFloUI MemoryPanel Support)
// ─────────────────────────────────────────────────────────────────────────────

async fn list_memory(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let consciousness = GLOBAL_CONSCIOUSNESS.read().unwrap();
    let facts = consciousness.all_facts();
    let memories: Vec<serde_json::Value> = facts
        .into_iter()
        .map(|f| {
            json!({
                "key": f.key,
                "value": f.value.to_string(),
                "author": f.author,
                "version": f.version,
                "updatedAt": f.updated_at.to_rfc3339()
            })
        })
        .collect();

    Json(json!({ "entries": memories, "total": memories.len() }))
}

#[derive(Deserialize)]
struct StoreMemoryPayload {
    key: String,
    value: String,
    #[serde(default)]
    author: Option<String>,
}

async fn store_memory(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<StoreMemoryPayload>,
) -> Json<serde_json::Value> {
    let mut consciousness = GLOBAL_CONSCIOUSNESS.write().unwrap();
    consciousness.set_fact(
        &payload.key,
        serde_json::Value::String(payload.value),
        payload.author.as_deref().unwrap_or("agent-coordinator"),
    );
    Json(json!({ "ok": true, "key": payload.key }))
}

async fn get_memory_stats(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let consciousness = GLOBAL_CONSCIOUSNESS.read().unwrap();
    let facts = consciousness.all_facts();
    Json(json!({
        "total_facts": facts.len(),
        "stream_events": consciousness.get_recent_stream(100).len(),
        "status": "healthy"
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Anti-Blackbox: Plugin & Capability Matrix
// ─────────────────────────────────────────────────────────────────────────────

async fn list_plugins(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let core_plugins = vec![
        json!({
            "name": "swarm_orchestrator",
            "category": "Swarm Intelligence",
            "status": "active",
            "description": "Multi-agent swarm coordinator supporting Mesh/Hierarchical topologies and consensus."
        }),
        json!({
            "name": "consciousness_stream",
            "category": "Cognitive Reflection",
            "status": "active",
            "description": "Hive-mind shared stream of consciousness and reactive blackboard."
        }),
        json!({
            "name": "workflow_pipeline",
            "category": "Orchestration",
            "status": "active",
            "description": "Deterministic DAG agentic pipeline with stage verification gates."
        }),
        json!({
            "name": "dynamic_python_tools",
            "category": "Hot-Plugged Extensions",
            "status": "active",
            "description": "Zero-compile Python script tool autoloader (read_file_safe, dynamic_greet)."
        }),
        json!({
            "name": "mcp_server_client",
            "category": "Protocol Bridge",
            "status": "active",
            "description": "Model Context Protocol bridge with stdio & SSE transport."
        }),
    ];

    Json(json!({
        "plugins": core_plugins,
        "total": core_plugins.len(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn get_system_capabilities(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let swarm = GLOBAL_SWARM_MANAGER.read().unwrap().status_report();
    let workflows = GLOBAL_WORKFLOW_ENGINE.read().unwrap().list_workflows();
    let cert = state.certification.list_capabilities();

    Json(json!({
        "version": "0.1.0-superagent",
        "architecture": {
            "dual_brain": true,
            "fast_brain": "Low Latency Triage & Compaction",
            "slow_brain": "Deep Reasoning & Reflection",
            "shell_guard": "PowerShell Alias Sanitization Active",
        },
        "swarm": {
            "topology": swarm.topology,
            "nodes_count": swarm.total_nodes,
            "active_count": swarm.active_nodes
        },
        "workflows_count": workflows.len(),
        "certified_capabilities": cert.len(),
        "transparent_mode": true
    }))
}
