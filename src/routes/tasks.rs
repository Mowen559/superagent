use crate::AppState;
use crate::services::brain::session_checkpoint_store::SessionCheckpointStore;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{
    extract::{Path, Query, State},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/{task_id}", get(get_task))
        .route("/api/tasks/{task_id}/logs", get(get_task_logs))
        .route(
            "/api/tasks/{task_id}/events",
            get(crate::routes::agents::stream_events),
        )
        .route(
            "/api/tasks/{task_id}/events/list",
            get(get_task_events_list),
        )
        .route("/api/tasks/{task_id}/cancel", post(cancel_task))
        .route("/api/tasks/{task_id}/traces", get(get_task_traces))
        .route("/api/tasks/{task_id}/traces/{turn}", get(get_task_trace_turn))
        // 智能体断点快照与端点续跑 (Session Checkpoints & Resume)
        .route("/api/agent/sessions", get(list_agent_sessions))
        .route("/api/agent/sessions/last-interrupted", get(get_last_interrupted_session))
        .route("/api/agent/sessions/{session_id}", get(get_agent_session))
        .route("/api/agent/sessions/{session_id}/traces", get(get_session_traces))
        .route("/api/agent/sessions/{session_id}/traces/{turn}", get(get_session_trace_turn))
        .route("/api/agent/sessions/{session_id}/resume", post(resume_agent_session))
        .route("/api/agent/sessions/{session_id}/complete", post(complete_agent_session))
        .route("/api/agent/sessions/{session_id}", delete(delete_agent_session))
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    pub limit: Option<usize>,
}

async fn list_agent_sessions(Query(query): Query<ListSessionsQuery>) -> Response {
    let limit = query.limit.unwrap_or(50);
    match SessionCheckpointStore::list_checkpoints(limit) {
        Ok(sessions) => Json(json!({ "sessions": sessions })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn get_last_interrupted_session() -> Response {
    match SessionCheckpointStore::get_last_interrupted_session() {
        Ok(session) => Json(json!({ "session": session })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn get_agent_session(Path(session_id): Path<String>) -> Response {
    match SessionCheckpointStore::get_checkpoint(&session_id) {
        Ok(Some(session)) => Json(json!({ "session": session })).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Session checkpoint not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn resume_agent_session(Path(session_id): Path<String>) -> Response {
    match SessionCheckpointStore::resume_checkpoint(&session_id) {
        Ok(session) => Json(json!({ "resumed": true, "session": session })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn complete_agent_session(Path(session_id): Path<String>) -> Response {
    match SessionCheckpointStore::mark_completed(&session_id) {
        Ok(()) => Json(json!({ "completed": true, "sessionId": session_id })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn delete_agent_session(Path(session_id): Path<String>) -> Response {
    match SessionCheckpointStore::delete_checkpoint(&session_id) {
        Ok(()) => Json(json!({ "deleted": true, "sessionId": session_id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn list_tasks(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let tasks = state.task_manager.list_tasks();
    Json(json!({ "tasks": tasks }))
}

async fn get_task(State(state): State<Arc<AppState>>, Path(task_id): Path<String>) -> Response {
    match state.task_manager.get_task(&task_id) {
        Some(task) => Json(json!(task)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response(),
    }
}

async fn get_task_logs(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let logs = state.task_manager.get_logs(&task_id);
    Json(json!({ "taskId": task_id, "logs": logs })).into_response()
}

async fn cancel_task(State(state): State<Arc<AppState>>, Path(task_id): Path<String>) -> Response {
    if state.task_manager.get_task(&task_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    }
    if state.task_manager.cancel_task(&task_id) {
        Json(json!({ "taskId": task_id, "status": "cancelled" })).into_response()
    } else {
        let status = state
            .task_manager
            .get_task(&task_id)
            .map(|task| task.status);
        (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "task is already terminal",
                "kind": "terminal_task",
                "status": status,
            })),
        )
            .into_response()
    }
}

pub(crate) async fn get_task_events_list(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    if state.task_manager.get_task(&task_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    }
    let events = state.task_manager.get_events(&task_id);
    let msgs: Vec<serde_json::Value> = events
        .into_iter()
        .map(|(seq, msg)| {
            let mut v = serde_json::to_value(&msg).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("sequence".to_string(), json!(seq));
            }
            v
        })
        .collect();
    Json(json!({ "taskId": task_id, "events": msgs })).into_response()
}


// ==========================================
// 白盒化决策链路追踪接口 (Trace Tree & Raw Snapshot)
// ==========================================

async fn get_session_traces(Path(session_id): Path<String>) -> Response {
    let traces_dir = std::path::PathBuf::from(".superagent")
        .join("traces")
        .join(&session_id);
    if !traces_dir.exists() {
        return Json(json!({ "sessionId": session_id, "traces": [] })).into_response();
    }

    let mut traces = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&traces_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        let turn = val.get("turn").and_then(|v| v.as_u64()).unwrap_or(0);
                        let brain_tier = val.get("brain_tier").and_then(|v| v.as_str()).unwrap_or("Unknown");
                        let timestamp = val.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                        let usage = val.get("usage");
                        let prompt_tokens = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
                        let completion_tokens = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
                        let total_tokens = usage.and_then(|u| u.get("total_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
                        let duration_ms = usage.and_then(|u| u.get("duration_ms")).and_then(|v| v.as_u64()).unwrap_or(0);
                        let tools_count = val.get("tools_schema").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                        let has_response = val.get("response").is_some();

                        traces.push(json!({
                            "turn": turn,
                            "timestamp": timestamp,
                            "brainTier": brain_tier,
                            "promptTokens": prompt_tokens,
                            "completionTokens": completion_tokens,
                            "totalTokens": total_tokens,
                            "durationMs": duration_ms,
                            "toolsCount": tools_count,
                            "hasResponse": has_response,
                            "snapshotFile": path.to_string_lossy().to_string(),
                        }));
                    }
                }
            }
        }
    }

    traces.sort_by_key(|t| t.get("turn").and_then(|v| v.as_u64()).unwrap_or(0));
    Json(json!({ "sessionId": session_id, "traces": traces })).into_response()
}

async fn get_session_trace_turn(Path((session_id, turn)): Path<(String, usize)>) -> Response {
    let snapshot_path = std::path::PathBuf::from(".superagent")
        .join("traces")
        .join(&session_id)
        .join(format!("turn_{}.json", turn));

    if !snapshot_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Trace turn {turn} not found for session {session_id}") })),
        )
            .into_response();
    }

    match std::fs::read_to_string(&snapshot_path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(json_val) => Json(json_val).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to parse snapshot: {e}") })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to read snapshot: {e}") })),
        )
            .into_response(),
    }
}

async fn get_task_traces(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    // 1. 直查同名会话目录
    let direct_dir = std::path::PathBuf::from(".superagent")
        .join("traces")
        .join(&task_id);
    if direct_dir.exists() {
        return get_session_traces(Path(task_id)).await;
    }

    // 2. 从任务事件中提取关联的 session_id
    let events = state.task_manager.get_events(&task_id);
    let session_id = events.iter().find_map(|(_, msg)| match msg {
        crate::services::agent::AgentMessage::TraceSnapshot { session_id, .. } if !session_id.is_empty() => {
            Some(session_id.clone())
        }
        crate::services::agent::AgentMessage::Status { session_id, .. } if !session_id.is_empty() => {
            Some(session_id.clone())
        }
        _ => None,
    });

    if let Some(sid) = session_id {
        return get_session_traces(Path(sid)).await;
    }

    // 3. 兜底回退：直接从内存事件中的 TraceSnapshot 聚合元数据
    let mut traces = Vec::new();
    for (_, msg) in &events {
        if let crate::services::agent::AgentMessage::TraceSnapshot {
            turn,
            session_id: _,
            brain_tier,
            prompt_tokens,
            completion_tokens,
            duration_ms,
            tools_count,
            snapshot_file,
        } = msg
        {
            traces.push(json!({
                "turn": turn,
                "brainTier": brain_tier,
                "promptTokens": prompt_tokens,
                "completionTokens": completion_tokens,
                "totalTokens": prompt_tokens + completion_tokens,
                "durationMs": duration_ms,
                "toolsCount": tools_count,
                "snapshotFile": snapshot_file,
            }));
        }
    }

    Json(json!({ "taskId": task_id, "traces": traces })).into_response()
}

async fn get_task_trace_turn(
    State(state): State<Arc<AppState>>,
    Path((task_id, turn)): Path<(String, usize)>,
) -> Response {
    let direct_path = std::path::PathBuf::from(".superagent")
        .join("traces")
        .join(&task_id)
        .join(format!("turn_{}.json", turn));
    if direct_path.exists() {
        return get_session_trace_turn(Path((task_id, turn))).await;
    }

    let events = state.task_manager.get_events(&task_id);
    let session_id = events.iter().find_map(|(_, msg)| match msg {
        crate::services::agent::AgentMessage::TraceSnapshot { session_id, .. } if !session_id.is_empty() => {
            Some(session_id.clone())
        }
        crate::services::agent::AgentMessage::Status { session_id, .. } if !session_id.is_empty() => {
            Some(session_id.clone())
        }
        _ => None,
    });

    if let Some(sid) = session_id {
        return get_session_trace_turn(Path((sid, turn))).await;
    }

    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": format!("Trace turn {turn} not found for task {task_id}") })),
    )
        .into_response()
}
