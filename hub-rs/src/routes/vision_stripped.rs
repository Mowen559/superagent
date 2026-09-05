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
            payload.x, payload.y, payload.width, payload.height, output_dir, crate::services::visual_pipeline::ffmpeg_engine::AudioConfig::None,
        )
    }).await {
        Ok(Ok(())) => (StatusCode::OK, Json(json!({ "status": "started" }))).into_response(),
        Ok(Err(e)) => internal(e).into_response(),
        Err(e) => internal(format!("Recording worker failed: {e}")).into_response(),
    }
}

pub async fn interactive_capture(
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


async fn record_stop(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let id = uuid::Uuid::new_v4().to_string();
    let export_path = state.visual_pipeline.artifact_dir().join(format!("{id}.mp4"));
    let result = tokio::task::spawn_blocking(move || {
        crate::services::visual_pipeline::record_manager::stop_and_export(export_path)
    }).await.map_err(|e| internal(format!("Recording worker failed: {e}")))?;
    
    // Close the native cropper window if it is open
    unsafe {
        if let Ok(hwnd) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(windows::core::w!("NativeCropperClass"), None) {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hwnd), 
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE, 
                windows::Win32::Foundation::WPARAM(0), 
                windows::Win32::Foundation::LPARAM(0)
            );
        }
    }

    match result {
        Ok(path) => Ok(Json(json!({
            "success": true, "video_path": path
        }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("Failed to stop recording: {e}"), "active": false
        })))),
    }
}

pub async fn install_ffmpeg() -> Result<Json<Value>, ApiError> {
    match crate::services::visual_pipeline::record_manager::install_ffmpeg().await {
        Ok(_) => Ok(Json(json!({ "success": true }))),
        Err(e) => {
            tracing::error!("Failed to install ffmpeg: {}", e);
            Err(internal(format!("Failed to install ffmpeg: {}", e)))
        }
    }
}

pub async fn ffmpeg_status() -> Json<Value> {
    let installed = crate::services::visual_pipeline::record_manager::check_ffmpeg_status();
    Json(json!({ "installed": installed }))
}
fn bad_request(error: String) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "kind": "invalid_visual_request" })),
    )
}

fn not_found(error: String) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": error, "kind": "visual_artifact_not_found" })),
    )
}

fn internal(error: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error, "kind": "visual_internal_error" })),
    )
}

fn provider_error(error: String) -> ApiError {
    let failed_stage = ["screenshot", "ocr", "translation"]
        .into_iter()
        .find(|stage| error.starts_with(&format!("{stage} stage failed:")));
    let status = if error.contains("not ready")
        || error.contains("缺少环境变量")
        || error.contains("not configured")
    {
        StatusCode::CONFLICT
    } else if error.contains("unknown ")
        || error.contains("cannot be empty")
        || error.contains("exceeds")
        || error.contains("not found")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::BAD_GATEWAY
    };
    (
        status,
        Json(json!({
            "error": error,
            "kind": "visual_provider_error",
            "failedStage": failed_stage
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn provider_catalog_is_discoverable_without_executing_pipeline() {
        let app = router().with_state(crate::server::test_state());
        let response = app
            .oneshot(
                Request::get("/api/vision/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert!(payload["translation"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
    }

    #[tokio::test]
    async fn artifact_route_rejects_path_traversal() {
        let app = router().with_state(crate::server::test_state());
        let response = app
            .oneshot(
                Request::get("/api/vision/artifacts/not-a-uuid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StitchRequest {
    pub files: Vec<String>,
    pub output_path: String,
}

pub async fn stitch_images(Json(payload): Json<StitchRequest>) -> Result<Json<Value>, ApiError> {
    let result = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let paths: Vec<PathBuf> = payload.files.iter().map(PathBuf::from).collect();
        let options = snow_stitch_images::StitchOptions::default();
        let res = snow_stitch_images::stitch_files(&paths, options).map_err(|e| e.to_string())?;
        
        let frame = res.image;
        frame.encode(std::path::Path::new(&payload.output_path)).map_err(|e| e.to_string())?;
        Ok(payload.output_path)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    match result {
        Ok(path) => Ok(Json(json!({ "status": "success", "outputPath": path }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e })))),
    }
}
