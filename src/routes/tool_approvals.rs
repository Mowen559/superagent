use crate::services::brain::tool_approval_store::{ToolApprovalRecord, ToolApprovalStore};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::io;
use std::sync::Arc;

type ApiError = (StatusCode, Json<Value>);

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/tool-approvals", get(list_approvals))
        .route("/api/tool-approvals/session-rules", get(list_session_rules))
        .route("/api/tool-approvals/{id}", get(get_approval))
        .route("/api/tool-approvals/{id}/approve", post(approve))
        .route("/api/tool-approvals/{id}/approve-session", post(approve_session))
        .route("/api/tool-approvals/{id}/deny", post(deny))
        .route("/api/tool-approvals/{id}/revoke", post(revoke_session_rule))
}

async fn list_approvals(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let approvals = state.approval_store.list().map_err(store_error)?;
    Ok(Json(json!({
        "approvals": approvals,
        "count": approvals.len(),
    })))
}

async fn get_approval(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    match state.approval_store.get(&id).map_err(store_error)? {
        Some(record) => Ok(record_response(record)),
        None => Err(not_found()),
    }
}

async fn approve(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_control_plane_header(&headers, "approve")?;
    state
        .approval_store
        .approve(&id)
        .map(record_response)
        .map_err(transition_error)
}

async fn deny(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_control_plane_header(&headers, "deny")?;
    state
        .approval_store
        .deny(&id)
        .map(record_response)
        .map_err(transition_error)
}

async fn approve_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_control_plane_header(&headers, "approve-session")?;
    state
        .approval_store
        .approve_session(&id)
        .map(record_response)
        .map_err(transition_error)
}

async fn revoke_session_rule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_control_plane_header(&headers, "revoke")?;
    state
        .approval_store
        .revoke_session_rule(&id)
        .map(record_response)
        .map_err(transition_error)
}

async fn list_session_rules(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let rules = state.approval_store.list_session_rules().map_err(store_error)?;
    Ok(Json(json!({ "rules": rules, "count": rules.len() })))
}

fn require_control_plane_header(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    let supplied = headers
        .get("x-hub-approval")
        .and_then(|value| value.to_str().ok());
    if supplied == Some(expected) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "missing or invalid local approval header",
                "kind": "approval_header_required"
            })),
        ))
    }
}

fn record_response(record: ToolApprovalRecord) -> Json<Value> {
    Json(json!({ "approval": record }))
}

fn transition_error(error: io::Error) -> ApiError {
    match error.kind() {
        io::ErrorKind::NotFound => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "approval is missing or no longer pending",
                "kind": "invalid_approval_state"
            })),
        ),
        io::ErrorKind::InvalidInput => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string(), "kind": "invalid_approval_id" })),
        ),
        _ => store_error(error),
    }
}

fn store_error(error: io::Error) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string(), "kind": "approval_store_error" })),
    )
}

fn not_found() -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "approval not found", "kind": "not_found" })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::brain::tool_policy::ToolRisk;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use serde_json::json;
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[tokio::test]
    async fn approval_route_transitions_pending_record() {
        let directory = tempdir().unwrap();
        let store = ToolApprovalStore::new(directory.path().to_path_buf());
        let pending = store
            .request(
                "write_to_file",
                &json!({ "path": "a.txt" }),
                ToolRisk::WorkspaceMutation,
            )
            .unwrap();

        let mut state = crate::server::test_state();
        Arc::get_mut(&mut state).unwrap().approval_store = Arc::new(store);
        let app = router().with_state(state);
        let response = app
            .oneshot(
                Request::post(format!("/api/tool-approvals/{}/approve", pending.id))
                    .header("x-hub-approval", "approve")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["approval"]["status"], "approved");
    }

    #[test]
    fn mutation_routes_require_local_control_plane_header() {
        let error = require_control_plane_header(&HeaderMap::new(), "approve").unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }
}
