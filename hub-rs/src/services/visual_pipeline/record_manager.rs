use std::sync::Mutex;
use std::path::PathBuf;
use once_cell::sync::Lazy;
use crate::services::visual_pipeline::ffmpeg_engine::{FfmpegEngine, AudioConfig, resolve_ffmpeg_binary};

pub struct RecordingSession {
    pub engine: Option<FfmpegEngine>,
    pub chunks: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub session_id: String,
    pub audio_config: AudioConfig,
    pub region: (i32, i32, i32, i32),
}

static ACTIVE_SESSION: Lazy<Mutex<Option<RecordingSession>>> = Lazy::new(|| Mutex::new(None));

pub fn start_recording(x: i32, y: i32, width: i32, height: i32, output_dir: PathBuf, audio_config: AudioConfig) -> Result<(), String> {
    let mut session_lock = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    
    if session_lock.is_some() {
        return Err("A recording is already in progress.".into());
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let mut session = RecordingSession {
        engine: None,
        chunks: Vec::new(),
        output_dir,
        session_id,
        audio_config,
        region: (x, y, width, height),
    };
    
    start_chunk(&mut session)?;
    
    *session_lock = Some(session);
    Ok(())
}

fn start_chunk(session: &mut RecordingSession) -> Result<(), String> {
    if session.engine.is_some() {
        return Err("Engine is already running".into());
    }
    
    let mut engine = FfmpegEngine::new();
    let chunk_path = session.output_dir.join(format!("{}_chunk_{}.mp4", session.session_id, session.chunks.len()));
    
    engine.start(
        session.region.0,
        session.region.1,
        session.region.2,
        session.region.3,
        chunk_path.clone(),
        session.audio_config.clone()
    )?;
    
    session.engine = Some(engine);
    session.chunks.push(chunk_path);
    Ok(())
}

pub fn pause_recording() -> Result<(), String> {
    let mut session_lock = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let session = session_lock.as_mut().ok_or_else(|| "No active recording found.".to_string())?;
    
    if let Some(mut engine) = session.engine.take() {
        let _ = engine.stop();
    }
    Ok(())
}

pub fn resume_recording() -> Result<(), String> {
    let mut session_lock = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let session = session_lock.as_mut().ok_or_else(|| "No active recording found.".to_string())?;
    
    start_chunk(session)?;
    Ok(())
}

pub fn discard_recording() -> Result<(), String> {
    let mut session_lock = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let mut session = session_lock.take().ok_or_else(|| "No active recording found.".to_string())?;
    
    if let Some(mut engine) = session.engine.take() {
        let _ = engine.stop();
    }
    
    for chunk in &session.chunks {
        let _ = std::fs::remove_file(chunk);
    }
    Ok(())
}

pub fn stop_and_export(export_path: PathBuf) -> Result<String, String> {
    let mut session_lock = ACTIVE_SESSION.lock().map_err(|e| e.to_string())?;
    let mut session = session_lock.take().ok_or_else(|| "No active recording found.".to_string())?;
    
    if let Some(mut engine) = session.engine.take() {
        let _ = engine.stop(); // Stop current chunk
    }
    
    if session.chunks.is_empty() {
        return Err("No video chunks generated.".into());
    }

    if session.chunks.len() == 1 {
        // Just rename the single chunk
        let actual_output = &session.chunks[0];
        if let Err(e) = std::fs::rename(actual_output, &export_path) {
            if let Err(copy_err) = std::fs::copy(actual_output, &export_path) {
                return Err(format!("Failed to export output file: {}, {}. Recording retained at {}", e, copy_err, actual_output.display()));
            }
            let _ = std::fs::remove_file(actual_output);
        }
        return Ok(export_path.to_string_lossy().to_string());
    }
    
    // Concatenate multiple chunks
    let list_file_path = session.output_dir.join(format!("{}_concat.txt", session.session_id));
    let mut list_content = String::new();
    for chunk in &session.chunks {
        // FFmpeg concat demuxer expects forward slashes and escaped single quotes
        let path_str = chunk.to_string_lossy().replace("\\", "/");
        list_content.push_str(&format!("file '{}'\n", path_str));
    }
    
    std::fs::write(&list_file_path, list_content).map_err(|e| format!("Failed to write concat list: {}", e))?;
    
    let mut cmd = std::process::Command::new(resolve_ffmpeg_binary());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.args(["-y", "-f", "concat", "-safe", "0", "-i"]);
    cmd.arg(&list_file_path);
    cmd.args(["-c", "copy"]);
    cmd.arg(&export_path);
    
    let status = cmd.status().map_err(|e| format!("Failed to run concat command: {}", e))?;
    
    // Cleanup temporary files
    let _ = std::fs::remove_file(&list_file_path);
    for chunk in &session.chunks {
        let _ = std::fs::remove_file(chunk);
    }
    
    if !status.success() {
        return Err("Failed to concatenate video chunks.".into());
    }

    Ok(export_path.to_string_lossy().to_string())
}

pub async fn install_ffmpeg() -> Result<(), String> {
    let url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
    
    let response = reqwest::get(url).await.map_err(|e| format!("Failed to download ffmpeg: {}", e))?;
    let bytes = response.bytes().await.map_err(|e| format!("Failed to read response: {}", e))?;
    
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open zip archive: {}", e))?;
    
    let mut ffmpeg_exe = None;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| format!("Failed to read zip file {}: {}", i, e))?;
        if file.name().ends_with("bin/ffmpeg.exe") {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut buf).map_err(|e| format!("Failed to read ffmpeg.exe from zip: {}", e))?;
            ffmpeg_exe = Some(buf);
            break;
        }
    }
    
    let ffmpeg_exe = ffmpeg_exe.ok_or_else(|| "ffmpeg.exe not found in downloaded archive".to_string())?;
    
    let target_dir = std::env::current_dir().unwrap_or_default().join("hub-rs");
    if !target_dir.exists() {
        std::fs::create_dir_all(&target_dir).map_err(|e| format!("Failed to create target directory: {}", e))?;
    }
    
    let exe_path = target_dir.join("ffmpeg.exe");
    std::fs::write(&exe_path, ffmpeg_exe).map_err(|e| format!("Failed to write ffmpeg.exe to disk: {}", e))?;
    
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
    command.arg("-version").output().is_ok_and(|output| output.status.success())
}
