use crate::config::persona_settings::{self, AgentPersona};
use crate::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;

/// GET /api/settings/persona
/// 读取当前智能体人设配置及生成的 System Prompt 预览
pub(crate) async fn get_persona(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let persona = persona_settings::load_persona(&state.app_paths);
    let prompt_preview = persona.render_prompt_section();
    Json(json!({
        "persona": persona,
        "promptPreview": prompt_preview
    }))
}

/// PUT /api/settings/persona
/// 保存更新智能体人设配置
pub(crate) async fn put_persona(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AgentPersona>,
) -> Response {
    if body.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "智能体姓名不能为空" })),
        )
            .into_response();
    }

    match persona_settings::save_persona(&state.app_paths, &body) {
        Ok(_) => {
            let prompt_preview = body.render_prompt_section();
            Json(json!({
                "success": true,
                "persona": body,
                "promptPreview": prompt_preview
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("保存人设配置失败: {}", e) })),
        )
            .into_response(),
    }
}

/// POST /api/settings/persona/reset
/// 恢复出厂设置（还原回初始出厂设定的莫智人设）
pub(crate) async fn reset_persona(State(state): State<Arc<AppState>>) -> Response {
    match persona_settings::reset_to_factory_defaults(&state.app_paths) {
        Ok(default_persona) => {
            let prompt_preview = default_persona.render_prompt_section();
            Json(json!({
                "success": true,
                "persona": default_persona,
                "promptPreview": prompt_preview,
                "message": "已成功恢复智能体出厂设定 (莫智)"
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("恢复出厂设置失败: {}", e) })),
        )
            .into_response(),
    }
}

