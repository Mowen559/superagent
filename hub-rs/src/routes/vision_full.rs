use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::services::visual_pipeline::types::{
    CaptureRequest, OcrRequest, PipelineRequest, TranslationRequest, VisualSettings,
};
use crate::AppState;

#[cfg(target_os = "windows")]
use std::mem;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{HWND, RECT};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};

type ApiError = (StatusCode, Json<Value>);

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/vision/providers", get(providers))
        .route("/api/vision/providers/reload", post(reload_providers))
        .route("/api/vision/settings", get(get_settings).put(update_settings))
        .route("/api/vision/monitors", get(monitors))
        .route("/api/vision/windows", get(windows))
        .route("/api/vision/capture", post(capture))
        .route("/api/vision/interactive-capture", post(interactive_capture))
        .route(
            "/api/vision/artifacts/{artifact_id}",
            get(artifact).delete(delete_artifact),
        )
        .route("/api/vision/ocr", post(ocr))
        .route("/api/vision/translate", post(translate))
        .route("/api/vision/pipeline", post(pipeline))
        .route("/api/vision/element", get(element))
        .route("/api/vision/stitch", post(stitch_images))
        .route("/api/vision/tts", post(tts_speak))
        .route("/api/vision/record/start", post(record_start))
        .route("/api/vision/record/stop", post(record_stop))
        .route("/api/vision/ffmpeg/install", post(install_ffmpeg))
        .route("/api/vision/ffmpeg/status", get(ffmpeg_status))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MonitorQuery {
    provider_id: Option<String>,
}

async fn providers(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(state.visual_pipeline.providers()))
}

async fn reload_providers(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(state.visual_pipeline.reload_providers()))
}

async fn get_settings(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(state.visual_pipeline.settings()))
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(settings): Json<VisualSettings>,
) -> Result<Json<Value>, ApiError> {
    state
        .visual_pipeline
        .update_settings(settings)
        .map(|settings| Json(json!(settings)))
        .map_err(bad_request)
}

async fn monitors(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MonitorQuery>,
) -> Result<Json<Value>, ApiError> {
    state
        .visual_pipeline
        .monitors(query.provider_id.as_deref())
        .await
        .map(|monitors| Json(json!({ "monitors": monitors })))
        .map_err(provider_error)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WindowInfo {
    id: u32,
    title: String,
    app_name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    is_minimized: bool,
}

async fn windows() -> Result<Json<Value>, ApiError> {
    let mut win_list = Vec::new();
    #[cfg(feature = "vision-xcap")]
    if let Ok(windows) = xcap::Window::all() {
        for w in windows {
            if let (Ok(mut width), Ok(mut height)) = (w.width(), w.height()) {
                let title = w.title().unwrap_or_default();
                if title.is_empty() {
                    continue;
                }
                
                let mut x = w.x().unwrap_or_default();
                let mut y = w.y().unwrap_or_default();

                #[cfg(target_os = "windows")]
                {
                    let hwnd = w.id().unwrap_or_default() as HWND;
                    let mut rect: RECT = unsafe { mem::zeroed() };
                    if unsafe {
                        DwmGetWindowAttribute(
                            hwnd,
                            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                            &mut rect as *mut _ as *mut _,
                            mem::size_of::<RECT>() as u32,
                        )
                    } == 0
                    {
                        x = rect.left;
                        y = rect.top;
                        width = (rect.right - rect.left) as u32;
                        height = (rect.bottom - rect.top) as u32;
                    }
                }

                if width > 5 && height > 5 {
                    win_list.push(WindowInfo {
                        id: w.id().unwrap_or_default(),
                        title: w.title().unwrap_or_default(),
                        app_name: w.app_name().unwrap_or_default(),
                        x,
                        y,
                        width,
                        height,
                        is_minimized: w.is_minimized().unwrap_or(false),
                    });
                }
            }
        }
    }
    Ok(Json(json!({ "windows": win_list })))
}

async fn interactive_capture(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let result = state
        .visual_pipeline
        .clone()
        .interactive_capture()
        .await
        .map_err(provider_error)?;

    match result {
        Some((artifact, action, x, y, width, height)) => {
            Ok(Json(json!({ "artifact": artifact, "action": action, "x": x, "y": y, "width": width, "height": height })))
        },
        None => Ok(Json(json!({ "artifact": null, "action": "cancel" })))
    }
}

async fn capture(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CaptureRequest>,
) -> Result<Json<Value>, ApiError> {
    state
        .visual_pipeline
        .capture(request)
        .await
        .map(|artifact| Json(json!({ "artifact": artifact })))
        .map_err(provider_error)
}

async fn artifact(
    State(state): State<Arc<AppState>>,
    Path(artifact_id): Path<String>,
) -> Result<Response, ApiError> {
    let path = state
        .visual_pipeline
        .artifact_path(&artifact_id)
        .map_err(|error| {
            if error.contains("not found") {
                not_found(error)
            } else {
                bad_request(error)
            }
        })?;
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| internal(format!("failed to read visual artifact: {error}")))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

async fn delete_artifact(
    State(state): State<Arc<AppState>>,
    Path(artifact_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    state
        .visual_pipeline
        .delete_artifact(&artifact_id)
        .map(|_| Json(json!({ "deleted": true, "artifactId": artifact_id })))
        .map_err(|error| {
            if error.contains("not found") || error.contains("expired") {
                not_found(error)
            } else {
                bad_request(error)
            }
        })
}

async fn ocr(
    State(state): State<Arc<AppState>>,
    Json(request): Json<OcrRequest>,
) -> Result<Json<Value>, ApiError> {
    state
        .visual_pipeline
        .ocr(request)
        .await
        .map(|receipt| Json(json!({ "receipt": receipt })))
        .map_err(provider_error)
}

async fn tts_speak(
    Json(request): Json<crate::services::visual_pipeline::tts::TtsRequest>,
) -> Result<Json<crate::services::visual_pipeline::tts::TtsResponse>, ApiError> {
    match crate::services::visual_pipeline::tts::speak_text(&request.text) {
        Ok(_) => Ok(Json(crate::services::visual_pipeline::tts::TtsResponse {
            success: true,
            error: None,
        })),
        Err(e) => Ok(Json(crate::services::visual_pipeline::tts::TtsResponse {
            success: false,
            error: Some(e),
        })),
    }
}

async fn translate(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TranslationRequest>,
) -> Result<Json<Value>, ApiError> {
    state
        .visual_pipeline
        .translate(request)
        .await
        .map(|receipt| Json(json!({ "receipt": receipt })))
        .map_err(provider_error)
}

async fn pipeline(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PipelineRequest>,
) -> Result<Json<Value>, ApiError> {
    state
        .visual_pipeline
        .pipeline(request)
        .await
        .map(|receipt| Json(json!({ "receipt": receipt })))
        .map_err(provider_error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElementRequest {
    x: i32,
    y: i32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ElementRectData {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

async fn element(
    State(_state): State<Arc<AppState>>,
    Query(request): Query<ElementRequest>,
) -> Result<Json<Value>, ApiError> {
    let rects = tokio::task::spawn_blocking(move || -> Result<Option<Vec<crate::services::ui_selector::ElementRect>>, String> {
        let mut service = crate::services::ui_selector::ElementRegionService::new().map_err(|e| e.to_string())?;
        
        // Use the windows crate's POINT directly since ElementRegionService depends on it
        let p = windows::Win32::Foundation::POINT { x: request.x, y: request.y };
        service.hit_test_point(p, crate::services::ui_selector::HitTestMode::UiElement).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| internal(e.to_string()))?
    .map_err(|e| internal(e))?;
    
    let mut results = Vec::new();
    if let Some(rs) = rects {
        for r in rs {
            results.push(ElementRectData {
                x: r.left() as f32,
                y: r.top() as f32,
                width: r.width() as f32,
                height: r.height() as f32,
            });
        }
    }
    
    Ok(Json(json!({ "elements": results })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
struct RecordStartRequest {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[utoipa::path(
    post,
    path = "/api/vision/record/start",
    responses((status = 200, description = "Recording started"))
)]
pub async fn record_start(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RecordStartRequest>,
) -> impl IntoResponse {
    if payload.width <= 0 || payload.height <= 0 || payload.x.checked_add(payload.width).is_none() || payload.y.checked_add(payload.height).is_none() {
        return bad_request("Invalid recording region".into()).into_response();
    }
    let output_dir = state.visual_pipeline.artifact_dir().to_path_buf();
    match tokio::task::spawn_blocking(move || {
        crate::services::visual_pipeline::record_manager::start_recording(
            payload.x, payload.y, payload.width, payload.height, output_dir,
        )
    }).await {
        Ok(Ok(())) => (StatusCode::OK, Json(json!({ "status": "started" }))).into_response(),
        Ok(Err(e)) => internal(e).into_response(),
        Err(e) => internal(format!("Recording worker failed: {e}")).into_response(),
    }
}

