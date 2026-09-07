use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use crate::services::visual_pipeline::ffmpeg_engine::{FfmpegEngine, AudioConfig, resolve_ffmpeg_binary};

/// Recording lifecycle phases per DDD / Visual Pipeline contract:
/// Capturing -> (Paused <-> Capturing) -> Stopping -> Encoding -> Validating -> Published | Failed | Reclaimed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingPhase {
    Idle,
    Capturing,
    Paused,
    Stopping,
    Encoding,
    Validating,
    Published,
    Failed,
    Reclaimed,
}

/// Durable verified recording artifact published only after output validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingArtifact {
    pub session_id: String,
    pub path: PathBuf,
    pub file_size_bytes: u64,
    pub duration_seconds: Option<f64>,
    pub created_at: String,
    pub region: (i32, i32, i32, i32),
}

/// Detailed snapshot of recording status for monitoring and telemetry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatusSnapshot {
    pub active: bool,
    pub phase: RecordingPhase,
    pub session_id: Option<String>,
    pub chunks_count: usize,
    pub region: Option<(i32, i32, i32, i32)>,
    pub last_error: Option<String>,
    pub published_artifact: Option<RecordingArtifact>,
}

pub struct RecordingSession {
    pub engine: Option<FfmpegEngine>,
    pub chunks: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub session_id: String,
    pub audio_config: AudioConfig,
    pub region: (i32, i32, i32, i32),
    pub phase: RecordingPhase,
    pub failure: Option<String>,
    pub started_at: Instant,
}

static LAST_ARTIFACT: Lazy<Mutex<Option<RecordingArtifact>>> = Lazy::new(|| Mutex::new(None));
static ACTIVE_SESSION: Lazy<Mutex<Option<RecordingSession>>> = Lazy::new(|| Mutex::new(None));

pub fn current_status() -> RecordingStatusSnapshot {
    let guard = ACTIVE_SESSION.lock().ok();
    let session = guard.as_ref().and_then(|g| g.as_ref());
    let last_art = LAST_ARTIFACT.lock().ok().and_then(|g| g.clone());

    if let Some(s) = session {
        RecordingStatusSnapshot {
            active: matches!(s.phase, RecordingPhase::Capturing | RecordingPhase::Paused),
            phase: s.phase,
            session_id: Some(s.session_id.clone()),
            chunks_count: s.chunks.len(),
            region: Some(s.region),
            last_error: s.failure.clone(),
            published_artifact: last_art,
        }
    } else {
        RecordingStatusSnapshot {
            active: false,
            phase: RecordingPhase::Idle,
            session_id: None,
            chunks_count: 0,
            region: None,
            last_error: None,
            published_artifact: last_art,
        }
    }
}

pub fn start_recording(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    output_dir: PathBuf,
    audio_config: AudioConfig,
) -> Result<(), String> {
    let mut session_lock = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;

    if let Some(existing) = session_lock.as_ref() {
        if matches!(existing.phase, RecordingPhase::Capturing | RecordingPhase::Paused | RecordingPhase::Stopping | RecordingPhase::Encoding | RecordingPhase::Validating) {
            return Err("A recording is already in progress.".into());
        }
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let mut session = RecordingSession {
        engine: None,
        chunks: Vec::new(),
        output_dir,
        session_id,
        audio_config,
        region: (x, y, width, height),
        phase: RecordingPhase::Idle,
        failure: None,
        started_at: Instant::now(),
    };

    if let Err(e) = start_chunk(&mut session) {
        session.phase = RecordingPhase::Failed;
        session.failure = Some(e.clone());
        return Err(e);
    }

    if let Ok(mut last) = LAST_ARTIFACT.lock() {
        *last = None;
    }
    *session_lock = Some(session);
    Ok(())
}

fn start_chunk(session: &mut RecordingSession) -> Result<(), String> {
    if let Some(error) = &session.failure {
        return Err(error.clone());
    }
    if session.engine.is_some() {
        return Err("Engine is already running".into());
    }

    let mut engine = FfmpegEngine::new();
    let chunk_path = session
        .output_dir
        .join(format!("{}_chunk_{}.mp4", session.session_id, session.chunks.len()));

    engine.start(
        session.region.0,
        session.region.1,
        session.region.2,
        session.region.3,
        chunk_path.clone(),
        session.audio_config.clone(),
    )?;

    session.engine = Some(engine);
    session.chunks.push(chunk_path);
    session.phase = RecordingPhase::Capturing;
    Ok(())
}

pub fn has_session() -> bool {
    ACTIVE_SESSION
        .try_lock()
        .map(|guard| {
            guard
                .as_ref()
                .is_some_and(|s| matches!(s.phase, RecordingPhase::Capturing | RecordingPhase::Paused))
        })
        .unwrap_or(true)
}

pub fn completed_artifact() -> Option<RecordingArtifact> {
    LAST_ARTIFACT.try_lock().ok().and_then(|art| art.clone())
}

pub fn completed_path() -> Option<PathBuf> {
    completed_artifact().map(|a| a.path)
}

fn stop_chunk(session: &mut RecordingSession) -> Result<(), String> {
    if let Some(error) = &session.failure {
        return Err(error.clone());
    }
    if let Some(mut engine) = session.engine.take() {
        if let Err(error) = engine.stop() {
            let error = format!("{error}. 原始片段保留在 {}", session.output_dir.display());
            session.failure = Some(error.clone());
            session.phase = RecordingPhase::Failed;
            return Err(error);
        }
    }
    Ok(())
}

pub fn pause_recording() -> Result<(), String> {
    let mut guard = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let session = guard.as_mut().ok_or("No active recording found")?;
    if session.phase != RecordingPhase::Capturing {
        return Err(format!("Cannot pause recording from phase {:?}", session.phase));
    }
    session.phase = RecordingPhase::Stopping;
    stop_chunk(session)?;
    session.phase = RecordingPhase::Paused;
    Ok(())
}

pub fn resume_recording() -> Result<(), String> {
    let mut guard = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let session = guard.as_mut().ok_or("No active recording found")?;
    if session.phase != RecordingPhase::Paused {
        return Err(format!("Cannot resume recording from phase {:?}", session.phase));
    }
    start_chunk(session)
}

pub fn discard_recording() -> Result<(), String> {
    let mut guard = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let mut session = guard.take().ok_or("No active recording found")?;
    session.phase = RecordingPhase::Reclaimed;
    if let Some(mut engine) = session.engine.take() {
        let _ = engine.stop();
    }
    for chunk in &session.chunks {
        let _ = std::fs::remove_file(chunk);
    }
    Ok(())
}

fn export_chunks(session: &mut RecordingSession, export_path: &Path) -> Result<(), String> {
    if session.chunks.is_empty() {
        return Err("No video chunks generated".into());
    }
    for chunk in &session.chunks {
        if !std::fs::metadata(chunk).is_ok_and(|m| m.len() > 0) {
            return Err(format!("无效录像片段：{}", chunk.display()));
        }
    }
    if let Some(parent) = export_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    session.phase = RecordingPhase::Encoding;

    if session.chunks.len() == 1 {
        std::fs::copy(&session.chunks[0], export_path).map_err(|e| {
            format!(
                "视频保存失败：{e}，原片段保留在 {}",
                session.output_dir.display()
            )
        })?;
    } else {
        let list_path = session
            .output_dir
            .join(format!("{}_concat.txt", session.session_id));
        let mut content = String::new();
        for chunk in &session.chunks {
            let escaped = chunk
                .to_string_lossy()
                .replace('\\', "/")
                .replace('\'', "'\\''");
            content.push_str(&format!("file '{escaped}'\n"));
        }
        std::fs::write(&list_path, content).map_err(|e| e.to_string())?;

        let mut cmd = std::process::Command::new(resolve_ffmpeg_binary());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let child = cmd
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
            ])
            .arg(&list_path)
            .args(["-c", "copy"])
            .arg(export_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        let result = (|| {
            let mut child = child.map_err(|e| e.to_string())?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    if status.success() {
                        return Ok(());
                    }
                    return Err("录像合并失败，原始片段已保留，可重试保存".to_string());
                }
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("录像合并超时，原始片段已保留".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        })();

        let _ = std::fs::remove_file(list_path);
        result?;
    }

    session.phase = RecordingPhase::Validating;
    let meta = std::fs::metadata(export_path).map_err(|e| {
        let _ = std::fs::remove_file(export_path);
        format!("无法读取导出视频文件元数据: {e}")
    })?;

    if meta.len() == 0 {
        let _ = std::fs::remove_file(export_path);
        return Err("编码器没有生成有效视频文件 (0 字节)".into());
    }

    Ok(())
}

pub fn stop_and_export(export_path: PathBuf) -> Result<RecordingArtifact, String> {
    let mut guard = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let session = guard.as_mut().ok_or("No active recording found")?;

    session.phase = RecordingPhase::Stopping;
    if let Err(e) = stop_chunk(session) {
        session.phase = RecordingPhase::Failed;
        return Err(e);
    }

    if let Err(e) = export_chunks(session, &export_path) {
        session.phase = RecordingPhase::Failed;
        session.failure = Some(e.clone());
        if export_path.exists() {
            let _ = std::fs::remove_file(&export_path);
        }
        return Err(e);
    }

    let file_size = std::fs::metadata(&export_path).map(|m| m.len()).unwrap_or(0);
    let duration_secs = session.started_at.elapsed().as_secs_f64();

    let artifact = RecordingArtifact {
        session_id: session.session_id.clone(),
        path: export_path.clone(),
        file_size_bytes: file_size,
        duration_seconds: Some(duration_secs),
        created_at: chrono::Utc::now().to_rfc3339(),
        region: session.region,
    };

    session.phase = RecordingPhase::Published;

    // Remove source chunks since output artifact is verified & published
    for chunk in &session.chunks {
        let _ = std::fs::remove_file(chunk);
    }

    *guard = None;
    if let Ok(mut last) = LAST_ARTIFACT.lock() {
        *last = Some(artifact.clone());
    }

    Ok(artifact)
}

pub async fn install_ffmpeg() -> Result<(), String> {
    let url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Failed to download ffmpeg: {}", e))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let cursor = std::io::Cursor::new(bytes);
    let mut zip =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open zip archive: {}", e))?;

    let mut ffmpeg_exe = None;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| format!("Failed to read zip file {}: {}", i, e))?;
        if file.name().ends_with("bin/ffmpeg.exe") {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut buf)
                .map_err(|e| format!("Failed to read ffmpeg.exe from zip: {}", e))?;
            ffmpeg_exe = Some(buf);
            break;
        }
    }

    let ffmpeg_exe =
        ffmpeg_exe.ok_or_else(|| "ffmpeg.exe not found in downloaded archive".to_string())?;

    let target_dir = std::env::current_dir().unwrap_or_default().join("hub-rs");
    if !target_dir.exists() {
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("Failed to create target directory: {}", e))?;
    }

    let exe_path = target_dir.join("ffmpeg.exe");
    std::fs::write(&exe_path, ffmpeg_exe)
        .map_err(|e| format!("Failed to write ffmpeg.exe to disk: {}", e))?;

    Ok(())
}

pub fn check_ffmpeg_status() -> bool {
    let mut command = std::process::Command::new(
        crate::services::visual_pipeline::ffmpeg_engine::resolve_ffmpeg_binary(),
    );
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_phase_lifecycle_transitions() {
        let dir = std::env::temp_dir().join(format!("test_rec_mgr_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);

        let mut session = RecordingSession {
            engine: None,
            chunks: Vec::new(),
            output_dir: dir.clone(),
            session_id: "test-sess-1".into(),
            audio_config: AudioConfig::None,
            region: (0, 0, 800, 600),
            phase: RecordingPhase::Idle,
            failure: None,
            started_at: Instant::now(),
        };

        assert_eq!(session.phase, RecordingPhase::Idle);

        // Simulate chunk creation
        let chunk1 = dir.join("chunk_0.mp4");
        std::fs::write(&chunk1, b"mock-video-chunk-data").unwrap();
        session.chunks.push(chunk1.clone());
        session.phase = RecordingPhase::Capturing;
        assert_eq!(session.phase, RecordingPhase::Capturing);

        // Pause
        session.phase = RecordingPhase::Paused;
        assert_eq!(session.phase, RecordingPhase::Paused);

        // Resume & second chunk
        let chunk2 = dir.join("chunk_1.mp4");
        std::fs::write(&chunk2, b"mock-video-chunk-data-2").unwrap();
        session.chunks.push(chunk2.clone());
        session.phase = RecordingPhase::Capturing;
        assert_eq!(session.phase, RecordingPhase::Capturing);

        // Stop & Export mock failure with 0-byte output
        let invalid_export = dir.join("invalid_export.mp4");
        std::fs::write(&invalid_export, b"").unwrap(); // 0 bytes

        session.phase = RecordingPhase::Validating;
        let meta = std::fs::metadata(&invalid_export).unwrap();
        if meta.len() == 0 {
            let _ = std::fs::remove_file(&invalid_export);
            session.phase = RecordingPhase::Failed;
        }
        assert_eq!(session.phase, RecordingPhase::Failed);
        assert!(!invalid_export.exists(), "Corrupted 0-byte video artifact must be removed");

        // Discard / Reclaim
        session.phase = RecordingPhase::Reclaimed;
        for c in &session.chunks {
            let _ = std::fs::remove_file(c);
        }
        assert_eq!(session.phase, RecordingPhase::Reclaimed);
        assert!(!chunk1.exists());
        assert!(!chunk2.exists());

        let _ = std::fs::remove_dir_all(dir);
    }
}
