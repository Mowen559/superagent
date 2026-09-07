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
    pub failure: Option<String>,
}

static LAST_EXPORT: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

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
        failure: None,
    };
    
    start_chunk(&mut session)?;
    if let Ok(mut last)=LAST_EXPORT.lock(){*last=None;}
    *session_lock = Some(session);
    Ok(())
}

fn start_chunk(session: &mut RecordingSession) -> Result<(), String> {
    if let Some(error)=&session.failure { return Err(error.clone()); }
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

pub fn has_session() -> bool {
    ACTIVE_SESSION.try_lock().map(|session|session.is_some()).unwrap_or(true)
}

pub fn completed_path() -> Option<PathBuf> {
    if has_session() {return None;}
    LAST_EXPORT.try_lock().ok().and_then(|path|path.clone())
}

fn stop_chunk(session: &mut RecordingSession) -> Result<(), String> {
    if let Some(error)=&session.failure { return Err(error.clone()); }
    if let Some(mut engine)=session.engine.take() {
        if let Err(error)=engine.stop() {
            let error=format!("{error}. 原始片段保留在 {}",session.output_dir.display());
            session.failure=Some(error.clone());
            return Err(error);
        }
    }
    Ok(())
}

pub fn pause_recording() -> Result<(), String> {
    let mut guard=ACTIVE_SESSION.lock().map_err(|e|e.to_string())?;
    let session=guard.as_mut().ok_or("No active recording found")?;
    stop_chunk(session)
}

pub fn resume_recording() -> Result<(), String> {
    let mut guard=ACTIVE_SESSION.lock().map_err(|e|e.to_string())?;
    let session=guard.as_mut().ok_or("No active recording found")?;
    start_chunk(session)
}

pub fn discard_recording() -> Result<(), String> {
    let mut guard=ACTIVE_SESSION.lock().map_err(|e|e.to_string())?;
    let mut session=guard.take().ok_or("No active recording found")?;
    if let Some(mut engine)=session.engine.take(){let _=engine.stop();}
    for chunk in &session.chunks {let _=std::fs::remove_file(chunk);}
    Ok(())
}

fn export_chunks(session: &RecordingSession, export_path: &std::path::Path) -> Result<(),String> {
    if session.chunks.is_empty(){return Err("No video chunks generated".into());}
    for chunk in &session.chunks {
        if !std::fs::metadata(chunk).is_ok_and(|m|m.len()>0) {return Err(format!("无效录像片段：{}",chunk.display()));}
    }
    if let Some(parent)=export_path.parent(){std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    if session.chunks.len()==1 {
        std::fs::copy(&session.chunks[0],export_path).map_err(|e|format!("视频保存失败：{e}，原片段保留在 {}",session.output_dir.display()))?;
    } else {
        let list_path=session.output_dir.join(format!("{}_concat.txt",session.session_id));
        let mut content=String::new();
        for chunk in &session.chunks {
            let escaped=chunk.to_string_lossy().replace('\\',"/").replace('\'',"'\\''");
            content.push_str(&format!("file '{escaped}'\n"));
        }
        std::fs::write(&list_path,content).map_err(|e|e.to_string())?;
        let mut cmd=std::process::Command::new(resolve_ffmpeg_binary());
        #[cfg(windows)] {use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
        let child=cmd.args(["-hide_banner","-loglevel","error","-y","-f","concat","-safe","0","-i"])
            .arg(&list_path).args(["-c","copy"]).arg(export_path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        let result=(|| {
            let mut child=child.map_err(|e|e.to_string())?;
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(60);
            loop {
                if let Some(status)=child.try_wait().map_err(|e|e.to_string())? {
                    if status.success(){return Ok(());}
                    return Err("录像合并失败，原始片段已保留，可重试保存".to_string());
                }
                if std::time::Instant::now()>deadline {let _=child.kill();let _=child.wait();return Err("录像合并超时，原始片段已保留".into());}
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        })();
        let _=std::fs::remove_file(list_path);
        result?;
    }
    if !std::fs::metadata(export_path).is_ok_and(|m|m.len()>0){return Err("编码器没有生成有效视频".into());}
    Ok(())
}

pub fn stop_and_export(export_path:PathBuf)->Result<String,String>{
    let mut guard=ACTIVE_SESSION.lock().map_err(|e|e.to_string())?;
    let session=guard.as_mut().ok_or("No active recording found")?;
    stop_chunk(session)?;
    export_chunks(session,&export_path)?;
    for chunk in &session.chunks {let _=std::fs::remove_file(chunk);}
    *guard=None;
    if let Ok(mut last)=LAST_EXPORT.lock(){*last=Some(export_path.clone());}
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
