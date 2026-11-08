//! RuFloUI 协议全量适配与全景能力可视化路由 (RuFlo Protocol Adapter & Capability Hub)
//! 提供蜂群编排、意识流黑板、工作流管线、智能体池、跨会话记忆以及全量插件透视能力，无缝直连 RuFlo 前台。

use crate::domains::swarm_intelligence::{
    ConsensusStatus, SwarmNode, SwarmRole, SwarmTopology, GLOBAL_CONSCIOUSNESS,
    GLOBAL_SWARM_MANAGER, GLOBAL_WORKFLOW_ENGINE, ThoughtType,
};
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(super::swarm_execution::router())
        // ── 1. 蜂群网络 (Swarm Intelligence) ──
        .route("/api/swarm/status", get(get_swarm_status))
        .route("/api/swarm/health", get(get_swarm_health))
        .route("/api/swarm/init", post(init_swarm))
        .route("/api/swarm/shutdown", post(shutdown_swarm))
        .route("/api/swarm/tasks/enqueue", post(enqueue_swarm_task))
        .route("/api/swarm/tasks/{queue}/stats", get(get_queue_stats))
        .route("/api/swarm/tasks/{queue}/dlq", get(get_queue_dlq))
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

fn registered_node(node: SwarmNode) -> serde_json::Value {
    let mut view = json!(node);
    view["status"] = json!("registered");
    view["executionReady"] = json!(false);
    view["executionState"] = json!("unavailable");
    view
}

fn unavailable_swarm_operation(operation: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "ok": false,
            "error_code": "swarm_execution_unavailable",
            "operation": operation,
            "executionStarted": false,
            "executionReady": false,
            "error": "Swarm execution runtime is not connected"
        })),
    )
}

async fn get_swarm_status(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    Json(json!({
        "status": "metadata_only",
        "managerReady": true,
        "executionReady": false,
        "topology": report.topology,
        "strategy": report.strategy,
        "messageBus": mgr.message_bus.bus_type(),
        "activeAgents": 0,
        "registeredAgents": report.total_nodes,
        "totalAgents": report.total_nodes,
        "uptime": report.uptime_seconds,
        "nodes": report.nodes.into_iter().map(registered_node).collect::<Vec<_>>(),
        "proposals": report.active_proposals,
    }))
}

async fn get_swarm_health(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    Json(json!({
        "health": "unverified",
        "healthy": false,
        "managerReady": true,
        "executionReady": false,
        "reason": "Swarm execution runtime is not connected",
        "node_count": report.total_nodes,
        "active_count": 0,
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
        "operation": "configuration_updated",
        "executionStarted": false,
        "executionReady": false,
        "topology": payload.topology.unwrap_or_else(|| "hierarchical".into())
    }))
}

async fn shutdown_swarm(
    State(_state): State<Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    unavailable_swarm_operation("shutdown")
}

#[derive(Deserialize)]
struct EnqueueTaskPayload {
    queue: String,
    role: String,
    payload: serde_json::Value,
    priority: Option<u8>,
}

async fn enqueue_swarm_task(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<EnqueueTaskPayload>,
) -> Json<serde_json::Value> {
    let bus = {
        let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
        mgr.message_bus.clone()
    };
    let mut task = crate::domains::swarm_intelligence::SwarmTaskItem::new(
        &payload.queue,
        &payload.role,
        payload.payload,
    );
    if let Some(p) = payload.priority {
        task = task.with_priority(p);
    }
    match bus.enqueue_task(&payload.queue, task).await {
        Ok(task_id) => Json(json!({ "ok": true, "taskId": task_id, "state": "queued", "executionStarted": false })),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

async fn get_queue_stats(
    State(_state): State<Arc<AppState>>,
    Path(queue): Path<String>,
) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let (pending, in_flight, dlq) = mgr.message_bus.queue_stats(&queue);
    Json(json!({
        "ok": true,
        "queue": queue,
        "pending": pending,
        "inFlight": in_flight,
        "dlq": dlq
    }))
}

async fn get_queue_dlq(
    State(_state): State<Arc<AppState>>,
    Path(queue): Path<String>,
) -> Json<serde_json::Value> {
    let bus = {
        let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
        mgr.message_bus.clone()
    };
    match bus.get_dlq(&queue).await {
        Ok(items) => Json(json!({ "ok": true, "items": items })),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Hive-Mind & Consciousness Handlers
// ─────────────────────────────────────────────────────────────────────────────

async fn get_hive_mind_status(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let consciousness = GLOBAL_CONSCIOUSNESS.read().unwrap();
    let stream = consciousness.get_recent_stream(20);
    let facts = consciousness.all_facts();
    Json(json!({
        "status": "memory_only",
        "protocol": "local_in_memory",
        "durable": false,
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
) -> (StatusCode, Json<serde_json::Value>) {
    let mut engine = GLOBAL_WORKFLOW_ENGINE.write().unwrap();
    match engine.advance_workflow(&id) {
        Ok(wf) => (StatusCode::OK, Json(json!({ "ok": true, "workflow": wf }))),
        Err(error) => {
            let missing = error == "Workflow not found";
            (
                if missing { StatusCode::NOT_FOUND } else { StatusCode::NOT_IMPLEMENTED },
                Json(json!({
                    "ok": false,
                    "error": error,
                    "error_code": if missing { "workflow_not_found" } else { "workflow_execution_unavailable" },
                    "executionStarted": false,
                    "executionReady": false
                })),
            )
        }
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
            let settings = crate::config::llm_settings::read_llm_settings();
            let effective_model = n.model_override.clone().unwrap_or_else(|| {
                settings.resolved_role_model(role_str)
            });
            json!({
                "id": n.id,
                "name": n.name,
                "type": role_str,
                "model": effective_model,
                "status": "registered",
                "executionReady": false,
                "executionState": "unavailable",
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
    model: Option<String>,
    capabilities: Option<Vec<String>>,
}

async fn spawn_agent(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<SpawnAgentPayload>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Keep validating the existing request shape without inventing a worker.
    drop((payload.agent_type, payload.name, payload.model, payload.capabilities));
    unavailable_swarm_operation("spawn")
}

async fn get_agent_status(
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let mgr = GLOBAL_SWARM_MANAGER.read().unwrap();
    let report = mgr.status_report();
    let found = report.nodes.into_iter().find(|n| n.id == id);
    match found {
        Some(node) => Json(json!({ "agent": registered_node(node), "status": "registered", "executionReady": false })),
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
        Some(_) => Json(json!({ "health": "unverified", "healthy": false, "status": "registered", "executionReady": false })),
        None => Json(json!({ "health": "unknown", "healthy": false, "status": "offline", "executionReady": false })),
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

    Json(json!({ "entries": memories, "total": memories.len(), "storage": "process_memory", "durable": false }))
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
        "status": "ready",
        "storage": "process_memory",
        "durable": false
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
            "status": "unverified",
            "executionReady": false,
            "description": "Registered roles and local messaging; task execution is not connected."
        }),
        json!({
            "name": "consciousness_stream",
            "category": "Cognitive Reflection",
            "status": "active",
            "storage": "process_memory",
            "durable": false,
            "description": "Hive-mind shared stream of consciousness and reactive blackboard."
        }),
        json!({
            "name": "workflow_pipeline",
            "category": "Orchestration",
            "status": "unavailable",
            "executionReady": false,
            "description": "Workflow definitions; real execution receipts are not connected."
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
            "active_count": 0,
            "registered_count": swarm.total_nodes,
            "managerReady": true,
            "executionReady": false
        },
        "workflows_count": workflows.len(),
        "certified_capabilities": cert.len(),
        "transparent_mode": true
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
        let app = router().with_state(crate::server::test_state());
        let mut builder = Request::builder().method(method).uri(uri);
        let bytes = if let Some(value) = body {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        } else { Body::empty() };
        let response = app.oneshot(builder.body(bytes).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn metadata_roles_do_not_report_healthy_running_workers() {
        let (status, health) = request("GET", "/api/swarm/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(health["health"], "unverified");
        assert_eq!(health["healthy"], false);
        assert_eq!(health["managerReady"], true);
        assert_eq!(health["executionReady"], false);
        assert_eq!(health["active_count"], 0);
        let (_, swarm) = request("GET", "/api/swarm/status", None).await;
        assert_eq!(swarm["status"], "metadata_only");
        assert_eq!(swarm["activeAgents"], 0);
        assert!(swarm["registeredAgents"].as_u64().unwrap() >= 4);
        assert!(swarm["nodes"].as_array().unwrap().iter().all(|n| n["status"] == "registered"));
        let (_, agents) = request("GET", "/api/swarm/agents", None).await;
        assert!(agents["agents"].as_array().unwrap().iter().all(|n| n["status"] == "registered" && n["executionReady"] == false));
        let (_, node) = request("GET", "/api/swarm/agents/agent-coder/status", None).await;
        assert_eq!(node["status"], "registered");
        assert_eq!(node["agent"]["status"], "registered");
        let (_, node_health) = request("GET", "/api/swarm/agents/agent-coder/health", None).await;
        assert_eq!(node_health["health"], "unverified");
        assert_eq!(node_health["healthy"], false);
        let (_, unknown) = request("GET", "/api/swarm/agents/missing/health", None).await;
        assert_eq!(unknown["health"], "unknown");
    }

    #[tokio::test]
    async fn unavailable_spawn_and_shutdown_have_no_registration_side_effect() {
        let (_, before) = request("GET", "/api/swarm/agents", None).await;
        let (status, spawn) = request("POST", "/api/swarm/agents/spawn", Some(json!({"type":"coder","name":"MUST_NOT_REGISTER"}))).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(spawn["ok"], false);
        assert_eq!(spawn["executionStarted"], false);
        assert_eq!(spawn["error_code"], "swarm_execution_unavailable");
        let (_, after) = request("GET", "/api/swarm/agents", None).await;
        assert_eq!(before["total"], after["total"]);
        assert!(!after["agents"].as_array().unwrap().iter().any(|n| n["name"] == "MUST_NOT_REGISTER"));
        let (status, shutdown) = request("POST", "/api/swarm/shutdown", Some(json!({}))).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(shutdown["ok"], false);
        assert_eq!(shutdown["operation"], "shutdown");
    }

    #[tokio::test]
    async fn unbound_workflow_execution_preserves_template_and_empty_definition() {
        let (_, before) = request("GET", "/api/workflows/wf-sdd-standard", None).await;
        assert_eq!(before["workflow"]["status"], "pending");
        let (status, execution) = request("POST", "/api/workflows/wf-sdd-standard/execute", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(execution["ok"], false);
        assert_eq!(execution["error_code"], "workflow_execution_unavailable");
        let (_, after) = request("GET", "/api/workflows/wf-sdd-standard", None).await;
        assert_eq!(before, after);
        let (_, created) = request("POST", "/api/workflows", Some(json!({"name":"empty audit"}))).await;
        let id = created["workflowId"].as_str().unwrap();
        let (_, empty_before) = request("GET", &format!("/api/workflows/{id}"), None).await;
        let (status, _) = request("POST", &format!("/api/workflows/{id}/execute"), None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let (_, empty_after) = request("GET", &format!("/api/workflows/{id}"), None).await;
        assert_eq!(empty_before, empty_after);
        let (status, missing) = request("POST", "/api/workflows/not-present/execute", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(missing["error_code"], "workflow_not_found");
    }

    #[tokio::test]
    async fn blackboard_and_capability_views_report_execution_and_storage_scope() {
        let (_, hive) = request("GET", "/api/hive-mind/status", None).await;
        assert_eq!(hive["protocol"], "local_in_memory");
        assert_eq!(hive["durable"], false);
        let (_, memory) = request("GET", "/api/memory", None).await;
        assert_eq!(memory["storage"], "process_memory");
        assert_eq!(memory["durable"], false);
        let (_, stats) = request("GET", "/api/memory/stats", None).await;
        assert_eq!(stats["durable"], false);
        let (_, plugins) = request("GET", "/api/plugins", None).await;
        for name in ["swarm_orchestrator", "workflow_pipeline"] {
            let item = plugins["plugins"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap();
            assert_ne!(item["status"], "active");
            assert_eq!(item["executionReady"], false);
        }
        let (_, capabilities) = request("GET", "/api/system/capabilities", None).await;
        assert_eq!(capabilities["swarm"]["executionReady"], false);
        assert_eq!(capabilities["swarm"]["active_count"], 0);
    }
}
