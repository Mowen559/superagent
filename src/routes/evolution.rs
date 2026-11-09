use crate::AppState;
use axum::extract::{State,Query,Path};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/evolution/candidates",get(list_candidates))
        .route("/api/evolution/candidates/{id}/actions",post(candidate_action))
        .route("/api/evolution/strategies", get(list_strategies))
        .route("/api/evolution/lessons", get(list_lessons))
        .route("/api/evolution/lessons/feedback", post(lesson_feedback))
}

async fn list_lessons(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let lessons = state.evolution_memory.list_lessons().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": error.to_string(),
                "kind": "evolution_store_error"
            })),
        )
    })?;
    Ok(Json(json!({
        "total": lessons.len(),
        "lessons": lessons,
        "activation":"historical_records_not_runtime_guidance",
    })))
}

async fn list_strategies(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let strategies = state.evolution_memory.list().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": error.to_string(),
                "kind": "evolution_store_error"
            })),
        )
    })?;
    let promoted = strategies
        .iter()
        .filter(|item| {
            item.state == crate::services::brain::evolution_memory::StrategyState::Promoted
        })
        .count();
    Ok(Json(json!({
        "strategies": strategies,
        "summary": {
            "total": strategies.len(),
            "promoted": 0,
            "nonPromoted": strategies.len(),
            "historicalMarkedPromoted":promoted,
            "guidanceSource":"governed_candidates_only",
        }
    })))
}

#[derive(Deserialize)]
pub struct LessonFeedbackPayload {
    pub lesson_id: String,
    pub positive: bool,
}

async fn lesson_feedback(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LessonFeedbackPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .evolution_memory
        .update_lesson_feedback(&payload.lesson_id, payload.positive)
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": error.to_string(),
                    "kind": "evolution_store_error"
                })),
            )
        })?;
    let _ = state.evolution_memory.prune_stale_lessons();
    Ok(Json(json!({
        "success": true,
        "lesson_id": payload.lesson_id,
        "positive": payload.positive,
    })))
}


#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct CandidateQuery {project_path:String}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct CandidateAction {project_path:String,action:String,expected_revision:u64,suite_id:Option<String>}
fn candidate_error(reason:String)->(StatusCode,Json<Value>) {
    let status=if reason=="workspace_not_open"{StatusCode::FORBIDDEN}else if reason=="candidate_not_found"{StatusCode::NOT_FOUND}else if reason.starts_with("holdout_")&&reason!="holdout_pass_required"{StatusCode::UNPROCESSABLE_ENTITY}else{StatusCode::CONFLICT};
    (status,Json(json!({"ok":false,"kind":reason})))
}
async fn list_candidates(State(state):State<Arc<AppState>>,Query(query):Query<CandidateQuery>)->Result<Json<Value>,(StatusCode,Json<Value>)> {
    let workspace=state.workspace_capabilities.resolve(&query.project_path).map_err(|_|candidate_error("workspace_not_open".into()))?;
    let store=crate::services::brain::candidate_governance::Governance::new(&state.runtime_config,&workspace);
    let entries=store.list().map_err(candidate_error)?;
    let active:Vec<_>=store.active().into_iter().map(|c|c.id).collect();
    Ok(Json(json!({"candidates":entries,"effectiveActiveIds":active})))
}
async fn candidate_action(State(state):State<Arc<AppState>>,Path(id):Path<String>,Json(action):Json<CandidateAction>)->Result<Json<Value>,(StatusCode,Json<Value>)> {
    let workspace=state.workspace_capabilities.resolve(&action.project_path).map_err(|_|candidate_error("workspace_not_open".into()))?;
    let store=crate::services::brain::candidate_governance::Governance::new(&state.runtime_config,&workspace);
    if action.action!="validate"&&action.suite_id.is_some(){return Err(candidate_error("unexpected_suite_id".into()));}
    let candidate=match action.action.as_str(){
        "validate"=>{let suite=action.suite_id.ok_or_else(||candidate_error("holdout_suite_required".into()))?;
            tokio::task::spawn_blocking(move||store.validate(&id,action.expected_revision,&suite)).await.map_err(|_|candidate_error("holdout_runner_failed".into()))?.map_err(candidate_error)?},
        "promote"=>store.promote(&id,action.expected_revision).map_err(candidate_error)?,
        "revoke"=>store.revoke(&id,action.expected_revision).map_err(candidate_error)?,
        _=>return Err(candidate_error("candidate_action_invalid".into())),
    };
    if candidate.last_result.starts_with("holdout_check_failed")||candidate.last_result.starts_with("holdout_timed_out"){
        return Err((StatusCode::UNPROCESSABLE_ENTITY,Json(json!({"ok":false,"kind":candidate.last_result,"candidate":candidate}))));
    }
    Ok(Json(json!({"ok":true,"candidate":candidate,"scope":"trusted held-out profile only; not general capability certification"})))
}
