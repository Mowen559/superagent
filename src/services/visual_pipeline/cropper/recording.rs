use std::path::PathBuf;
use std::time::Instant;
use crate::services::visual_pipeline::ffmpeg_engine::AudioConfig;
use crate::services::visual_pipeline::record_manager;

use super::types::{AppMode, AppState, RecordingState};

/// 在当前截取矩形选区内开启屏幕录像
pub fn start_crop_recording(state: &mut AppState) -> Result<(), String> {
    let rect = state.crop_rect.ok_or_else(|| "No crop region selected".to_string())?;
    let x = rect.left;
    let y = rect.top;
    let width = (rect.right - rect.left).max(16);
    let height = (rect.bottom - rect.top).max(16);

    let output_dir = if let Some(vp) = &state.vp {
        vp.artifact_dir().to_path_buf()
    } else {
        std::env::temp_dir()
    };

    let audio_config = AudioConfig::None;

    record_manager::start_recording(x, y, width, height, output_dir, audio_config)?;

    state.recording_session = Some(RecordingState {
        start_time: Instant::now(),
        is_paused: false,
        elapsed_secs: 0,
    });
    state.mode = AppMode::Recording;
    Ok(())
}

/// 暂停或恢复录像
pub fn toggle_pause_recording(state: &mut AppState) -> Result<(), String> {
    if let Some(session) = &mut state.recording_session {
        if session.is_paused {
            record_manager::resume_recording()?;
            session.is_paused = false;
        } else {
            record_manager::pause_recording()?;
            session.is_paused = true;
        }
        Ok(())
    } else {
        Err("No active recording session".to_string())
    }
}

/// 停止并导出当前录制的视频
pub fn stop_and_save_recording(state: &mut AppState) -> Result<PathBuf, String> {
    let session = state.recording_session.take().ok_or_else(|| "No active recording session".to_string())?;
    let export_dir = if let Some(vp) = &state.vp {
        vp.artifact_dir().to_path_buf()
    } else {
        std::env::temp_dir()
    };
    let filename = format!("recording_{}.mp4", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    let export_path = export_dir.join(filename);

    record_manager::stop_and_export(export_path.clone())?;
    state.mode = AppMode::ToolbarActive;
    Ok(export_path)
}

/// 放弃/取消当前录像
pub fn discard_crop_recording(state: &mut AppState) -> Result<(), String> {
    state.recording_session = None;
    record_manager::discard_recording()?;
    state.mode = AppMode::ToolbarActive;
    Ok(())
}
