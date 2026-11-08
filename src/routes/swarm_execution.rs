//! Managed per-Run control. Projections use existing task events; they do not
//! create a second lifecycle store or treat queued controls as started work.
use crate::domains::project_management::task_manager::TaskControl;
use crate::services::{agent::AgentMessage, brain::subagent_runtime::SubagentControl};
use crate::{
    AppState,
    models::task::{Task, TaskStatus},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

type Response = (StatusCode, Json<Value>);

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/swarm/runs/{task_id}/children",
            get(list_children).post(launch_child),
        )
        .route(
            "/api/swarm/runs/{task_id}/children/{child_id}/cancel",
            post(cancel_child),
        )
        .route("/api/swarm/runs/{task_id}/children/{child_id}/retry",post(retry_child))
        .route(
            "/api/swarm/runs/{task_id}/children/{child_id}/changes",
            post(collect_child),
        )
}

fn failure(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"ok":false,"error_code":code,"error":message,"executionStarted":false})),
    )
}

fn projection(state: &AppState, task: &Task) -> Value {
    let mut view = raw_projection(state, task);
    if state.task_manager.owns_active_run(&task.id) {
        return view;
    }
    let Some(session) = view["parent"]["parentSessionId"].as_str() else {
        return view;
    };
    if !crate::services::brain::subagent_journal::root()
        .join(format!("{session}.jsonl"))
        .is_file()
    {
        return view;
    }
    let reason = if task.status == TaskStatus::Running {
        "interrupted"
    } else {
        task.status.as_str()
    };
    match crate::services::brain::subagent_journal::after_parent_exit(session, reason) {
        Ok(frames) => {
            for frame in frames {
                let snapshot = &frame["snapshot"];
                let already = view["children"]
                    .as_array()
                    .is_some_and(|list| list.iter().any(|s| s == snapshot));
                if !already {
                    state.task_manager.append_event(
                        &task.id,
                        &AgentMessage::Log {
                            level: "subagent".into(),
                            content: frame.to_string(),
                        },
                    );
                }
            }
            if task.status == TaskStatus::Running {
                state.task_manager.update_status(
                    &task.id,
                    TaskStatus::Failed,
                    Some(
                        "managed parent interrupted after restart; child work will not be replayed"
                            .into(),
                    ),
                );
            }
            let task = state
                .task_manager
                .get_task(&task.id)
                .unwrap_or_else(|| task.clone());
            view = raw_projection(state, &task);
        }
        Err(error) => {
            view["reconciliationError"] = json!(error);
            view["observedOnly"] = json!(true);
            view["parentActive"] = json!(false);
        }
    }
    view
}

fn raw_projection(state: &AppState, task: &Task) -> Value {
    let mut parent = Value::Null;
    let mut children = std::collections::BTreeMap::<String, Value>::new();
    let mut receipts = Vec::new();
    for (_, event) in state.task_manager.get_events(&task.id) {
        let AgentMessage::Log { level, content } = event else {
            continue;
        };
        if level != "subagent" {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        if frame["type"] != "subagent_runtime" || frame["schemaVersion"] != 1 {
            continue;
        }
        match frame["event"].as_str() {
            Some("bound") => {
                parent = frame["parent"].clone();
                children.clear();
                receipts.clear();
            }
            Some("snapshot")
                if frame["snapshot"]["handle"]["parentRunId"] == parent["parentRunId"]
                    && frame["snapshot"]["handle"]["parentSessionId"]
                        == parent["parentSessionId"] =>
            {
                if let Some(id) = frame["snapshot"]["handle"]["subagentId"].as_str() {
                    children.insert(id.into(), frame["snapshot"].clone());
                }
            }
            Some("control_receipt") => receipts.push(frame),
            _ => {}
        }
    }
    let active = task.status == TaskStatus::Running;
    json!({"ok":true,"taskId":task.id,"parent":parent,"parentActive":active,
        "observedOnly":!active,"statusSource":"worker_events","children":children.into_values().collect::<Vec<_>>(),
        "controlReceipts":receipts})
}

fn managed_parent(state: &AppState, id: &str) -> Result<Task, Response> {
    let task = state.task_manager.get_task(id).ok_or_else(|| {
        failure(
            StatusCode::NOT_FOUND,
            "parent_not_found",
            "unknown parent task",
        )
    })?;
    if task.task_type != "agent"
        || task.payload.as_ref().and_then(|p| p["agent"].as_str()) != Some("superagent")
    {
        return Err(failure(
            StatusCode::CONFLICT,
            "parent_not_superagent",
            "parent is not a managed superagent task",
        ));
    }
    Ok(task)
}

fn active_parent(state: &AppState, task: &Task) -> Result<(), Response> {
    if task.status != TaskStatus::Running {
        return Err(failure(
            StatusCode::CONFLICT,
            "parent_not_active",
            "parent task is not running",
        ));
    }
    if projection(state, task)["parent"]["parentRunId"]
        .as_str()
        .is_none()
    {
        return Err(failure(
            StatusCode::CONFLICT,
            "parent_not_bound",
            "worker has not bound a parent Run",
        ));
    }
    Ok(())
}

async fn list_children(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match managed_parent(&state, &id) {
        Ok(task) => (StatusCode::OK, Json(projection(&state, &task))),
        Err(error) => error,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Launch {
    role: String,
    prompt: String,
    correlation_id: String,
    workspace_mode: Option<String>,
    timeout_seconds: Option<u64>,
}

async fn launch_child(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Launch>,
) -> Response {
    let task = match managed_parent(&state, &id) {
        Ok(task) => task,
        Err(error) => return error,
    };
    if let Err(error) = active_parent(&state, &task) {
        return error;
    }
    if body.role.trim().is_empty()
        || body.role.chars().count() > 120
        || body.prompt.trim().is_empty()
        || body.prompt.chars().count() > 16_000
        || body.correlation_id.trim().is_empty()
        || body.correlation_id.chars().count() > 200
        || !matches!(
            body.workspace_mode.as_deref(),
            None | Some("inherit" | "worktree")
        )
        || body
            .timeout_seconds
            .is_some_and(|v| !(1..=86400).contains(&v))
    {
        return failure(
            StatusCode::BAD_REQUEST,
            "invalid_subagent_request",
            "invalid role, prompt, correlationId, workspaceMode or timeoutSeconds",
        );
    }
    let correlation = body.correlation_id.clone();
    queued(
        &state,
        &id,
        TaskControl::Subagent(SubagentControl::Launch {
            parent_run_id: projection(&state, &task)["parent"]["parentRunId"]
                .as_str()
                .unwrap()
                .into(),
            correlation_id: body.correlation_id,
            role: body.role,
            prompt: body.prompt,
            workspace_mode: body.workspace_mode,
            timeout_seconds: body.timeout_seconds,
        }),
        &correlation,
    )
}

async fn cancel_child(
    State(state): State<Arc<AppState>>,
    Path((id, child)): Path<(String, String)>,
) -> Response {
    let task = match managed_parent(&state, &id) {
        Ok(task) => task,
        Err(error) => return error,
    };
    if let Err(error) = active_parent(&state, &task) {
        return error;
    }
    if !projection(&state, &task)["children"]
        .as_array()
        .is_some_and(|list| list.iter().any(|s| s["handle"]["subagentId"] == child))
    {
        return failure(
            StatusCode::NOT_FOUND,
            "child_not_found",
            "child not observed in this parent Run",
        );
    }
    queued(
        &state,
        &id,
        TaskControl::Subagent(SubagentControl::Cancel {
            parent_run_id: projection(&state, &task)["parent"]["parentRunId"]
                .as_str()
                .unwrap()
                .into(),
            subagent_id: child.clone(),
        }),
        &format!("cancel:{child}"),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetryRequest {}
async fn retry_child(State(state):State<Arc<AppState>>,Path((id,child)):Path<(String,String)>,Json(_body):Json<RetryRequest>)->Response {
    let task=match managed_parent(&state,&id){Ok(t)=>t,Err(e)=>return e};
    if let Err(e)=active_parent(&state,&task){return e;}
    let view=projection(&state,&task);
    if !view["children"].as_array().is_some_and(|list|list.iter().any(|s|s["handle"]["subagentId"]==child)) {return failure(StatusCode::NOT_FOUND,"child_not_found","predecessor not owned by this parent Run");}
    queued(&state,&id,TaskControl::Subagent(SubagentControl::Retry{parent_run_id:view["parent"]["parentRunId"].as_str().unwrap().into(),subagent_id:child.clone()}),&format!("retry:{child}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Collection {
    action: String,
    expected_digest: Option<String>,
}
async fn collect_child(
    State(state): State<Arc<AppState>>,
    Path((id, child)): Path<(String, String)>,
    Json(body): Json<Collection>,
) -> Response {
    let task = match managed_parent(&state, &id) {
        Ok(task) => task,
        Err(error) => return error,
    };
    if let Err(error) = active_parent(&state, &task) {
        return error;
    }
    let view = projection(&state, &task);
    if !view["children"]
        .as_array()
        .is_some_and(|list| list.iter().any(|s| s["handle"]["subagentId"] == child))
    {
        return failure(
            StatusCode::NOT_FOUND,
            "child_not_found",
            "child not observed in this parent Run",
        );
    }
    if !matches!(body.action.as_str(), "inspect" | "apply")
        || body.action == "apply"
            && !body
                .expected_digest
                .as_ref()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return failure(
            StatusCode::BAD_REQUEST,
            "invalid_collection_request",
            "inspect or apply with matching expectedDigest required",
        );
    }
    let correlation = format!("changes:{child}:{}", body.action);
    queued(
        &state,
        &id,
        TaskControl::Subagent(SubagentControl::Collect {
            parent_run_id: view["parent"]["parentRunId"].as_str().unwrap().into(),
            subagent_id: child,
            operation: body.action,
            expected_digest: body.expected_digest,
        }),
        &correlation,
    )
}

fn queued(state: &AppState, id: &str, control: TaskControl, correlation: &str) -> Response {
    if !state.task_manager.send_control(id, control) {
        return failure(
            StatusCode::CONFLICT,
            "parent_control_unavailable",
            "parent control channel is unavailable",
        );
    }
    (
        StatusCode::ACCEPTED,
        Json(
            json!({"ok":true,"state":"control_queued","correlationId":correlation,
        "executionStarted":false,"statusSource":"worker_events"}),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    async fn call(
        state: Arc<AppState>,
        method: &str,
        url: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let response = router()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(url)
                    .header("content-type", "application/json")
                    .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"body":String::from_utf8_lossy(&bytes)})),
        )
    }

    fn setup() -> (tempfile::TempDir, Arc<AppState>, Task) {
        let root = tempfile::tempdir().unwrap();
        let mut state = crate::server::test_state();
        let paths = crate::config::app_paths::AppPaths {
            bundle_root: root.path().into(),
            app_data_root: root.path().into(),
        };
        Arc::get_mut(&mut state).unwrap().task_manager =
            Arc::new(crate::domains::project_management::task_manager::TaskManager::new(&paths));
        let (task, _) = state.task_manager.create_task(
            "agent",
            root.path().to_str().unwrap(),
            Some(json!({"agent":"superagent"})),
        );
        state
            .task_manager
            .update_status(&task.id, TaskStatus::Running, None);
        (root, state, task)
    }

    fn event(state: &AppState, task: &Task, body: Value) {
        state.task_manager.append_event(
            &task.id,
            &AgentMessage::Log {
                level: "subagent".into(),
                content: body.to_string(),
            },
        );
    }

    fn bound(state: &AppState, task: &Task) {
        event(
            state,
            task,
            json!({"type":"subagent_runtime","schemaVersion":1,"event":"bound",
            "parent":{"parentRunId":"run-A","parentSessionId":"session-A","workspacePath":task.project_path,"toolPolicy":"yolo"}}),
        );
    }

    #[tokio::test]
    async fn control_requires_existing_active_bound_connected_parent() {
        let (_root, state, task) = setup();
        let body = json!({"role":"reviewer","prompt":"inspect","correlationId":"r1"});
        assert_eq!(
            call(
                state.clone(),
                "GET",
                "/api/swarm/runs/missing/children",
                None
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        let url = format!("/api/swarm/runs/{}/children", task.id);
        assert_eq!(
            call(state.clone(), "POST", &url, Some(body.clone()))
                .await
                .1["error_code"],
            "parent_not_bound"
        );
        bound(&state, &task);
        assert_eq!(
            call(state.clone(), "POST", &url, Some(body.clone()))
                .await
                .1["error_code"],
            "parent_control_unavailable"
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        state.task_manager.register_control(&task.id, tx);
        let (status, queued) = call(state.clone(), "POST", &url, Some(body.clone())).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(queued["executionStarted"], false);
        assert!(
            matches!(rx.try_recv().unwrap(),TaskControl::Subagent(SubagentControl::Launch{correlation_id,..}) if correlation_id=="r1")
        );
        let mut escaped = body.clone();
        escaped["workspacePath"] = json!("another-project");
        assert_eq!(
            call(state.clone(), "POST", &url, Some(escaped)).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let mut invalid = body.clone();
        invalid["timeoutSeconds"] = json!(0);
        assert_eq!(
            call(state.clone(), "POST", &url, Some(invalid)).await.0,
            StatusCode::BAD_REQUEST
        );
        assert!(rx.try_recv().is_err());

        state.task_manager.update_status(
            &task.id,
            TaskStatus::Failed,
            Some("test finished".into()),
        );
        assert_eq!(
            call(state.clone(), "POST", &url, Some(body)).await.1["error_code"],
            "parent_not_active"
        );
        let view = call(state, "GET", &url, None).await.1;
        assert_eq!(view["parentActive"], false);
        assert_eq!(view["observedOnly"], true);
    }

    #[tokio::test]
    async fn views_reject_other_runs_and_cancel_only_observed_owned_children() {
        let (_root, state, task) = setup();
        bound(&state, &task);
        for (run, id) in [("other-run", "foreign"), ("run-A", "owned")] {
            event(
                &state,
                &task,
                json!({"type":"subagent_runtime","schemaVersion":1,"event":"snapshot",
                "snapshot":{"handle":{"parentRunId":run,"parentSessionId":"session-A","subagentId":id},"state":"running","processId":123}}),
            );
        }
        let url = format!("/api/swarm/runs/{}/children", task.id);
        let (_, view) = call(state.clone(), "GET", &url, None).await;
        assert_eq!(view["children"].as_array().unwrap().len(), 1);
        assert_eq!(view["children"][0]["handle"]["subagentId"], "owned");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        state.task_manager.register_control(&task.id, tx);
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &format!("{url}/foreign/cancel"),
                None
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(state.clone(), "POST", &format!("{url}/owned/cancel"), None)
                .await
                .0,
            StatusCode::ACCEPTED
        );
        assert!(
            matches!(rx.try_recv().unwrap(),TaskControl::Subagent(SubagentControl::Cancel{subagent_id,..}) if subagent_id=="owned")
        );
        assert!(rx.try_recv().is_err());
        assert_eq!(call(state.clone(),"POST",&format!("{url}/foreign/retry"),Some(json!({}))).await.0,StatusCode::NOT_FOUND);
        assert!(call(state.clone(),"POST",&format!("{url}/owned/retry"),Some(json!({"prompt":"override task"}))).await.0.is_client_error());
        assert!(rx.try_recv().is_err(),"invalid retry may not enqueue a control");
        let (status,body)=call(state.clone(),"POST",&format!("{url}/owned/retry"),Some(json!({}))).await;
        assert_eq!(status,StatusCode::ACCEPTED);assert_eq!(body["executionStarted"],false);
        assert!(matches!(rx.try_recv().unwrap(),TaskControl::Subagent(SubagentControl::Retry{subagent_id,..}) if subagent_id=="owned"));
    }

    #[tokio::test]
    async fn restart_reconciles_durable_child_once_without_replaying_work() {
        let (_root, state, task) = setup();
        let session = format!("recover_{}", uuid::Uuid::new_v4().simple());
        event(
            &state,
            &task,
            json!({"type":"subagent_runtime","schemaVersion":1,"event":"bound","parent":{"parentRunId":"run-A","parentSessionId":session,"workspacePath":task.project_path}}),
        );
        let snapshot:crate::services::brain::subagent_runtime::SubagentSnapshot=serde_json::from_value(json!({"handle":{"subagentId":"11111111-1111-4111-8111-111111111111","parentSessionId":session,"parentRunId":"run-A","role":"coder","depth":1,"correlationId":"old","createdAt":0,"workspacePath":task.project_path,"originProject":task.project_path,"toolPolicy":"yolo"},"state":"running","updatedAt":0,"result":null,"error":null,"recentEvents":[],"processId":null,"executionWorkspace":null,"lastHeartbeatAt":null,"processReaped":false,"businessVerified":false})).unwrap();
        let ledger = crate::services::brain::subagent_journal::ChildJournal::open(
            &crate::services::brain::subagent_journal::root(),
            &session,
        )
        .unwrap();
        ledger.append("running", "old-request", &snapshot).unwrap();
        drop(ledger);
        let url = format!("/api/swarm/runs/{}/children", task.id);
        let view = call(state.clone(), "GET", &url, None).await.1;
        assert_eq!(view["children"][0]["state"], "failed");
        assert_eq!(view["children"][0]["processReaped"], true);
        assert_eq!(view["parentActive"], false);
        assert_eq!(
            state.task_manager.get_task(&task.id).unwrap().status,
            TaskStatus::Failed
        );
        let count = state.task_manager.get_events(&task.id).len();
        assert_eq!(call(state.clone(), "GET", &url, None).await.1, view);
        assert_eq!(state.task_manager.get_events(&task.id).len(), count);
    }

    #[tokio::test]
    async fn collection_controls_are_owned_bounded_and_do_not_claim_application() {
        let (_root, state, task) = setup();
        bound(&state, &task);
        event(
            &state,
            &task,
            json!({"type":"subagent_runtime","schemaVersion":1,"event":"snapshot","snapshot":{"handle":{"parentRunId":"run-A","parentSessionId":"session-A","subagentId":"owned"},"state":"succeeded","processReaped":true}}),
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        state.task_manager.register_control(&task.id, tx);
        let url = format!("/api/swarm/runs/{}/children/owned/changes", task.id);
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &url,
                Some(json!({"action":"apply","expectedDigest":"stale"}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &url,
                Some(json!({"action":"inspect","workspacePath":"other"}))
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let (status, result) = call(
            state.clone(),
            "POST",
            &url,
            Some(json!({"action":"inspect"})),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(result["executionStarted"], false);
        assert!(
            matches!(rx.try_recv().unwrap(),TaskControl::Subagent(SubagentControl::Collect{parent_run_id,operation,..}) if parent_run_id=="run-A"&&operation=="inspect")
        );
        assert!(rx.try_recv().is_err());
    }
}
