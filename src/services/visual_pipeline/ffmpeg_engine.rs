use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
// Preserve every selected pixel; only add at most one pixel on the right/bottom.
const EVEN_SIZE_FILTER: &str = "pad=ceil(iw/2)*2:ceil(ih/2)*2";

fn ffmpeg_candidates(executable: Option<&std::path::Path>, cwd: &std::path::Path) -> Vec<PathBuf> {
    let filename = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut candidates = Vec::new();
    if let Some(parent) = executable.and_then(|p| p.parent()) {
        candidates.push(parent.join(filename));
        // Cargo development binaries live under <hub>/target/{debug,release}.
        if let Some(target) = parent
            .parent()
            .filter(|p| p.file_name().is_some_and(|n| n == "target"))
        {
            if let Some(project) = target.parent() {
                candidates.push(project.join(filename));
            }
        }
    }
    candidates.push(cwd.join(filename));
    candidates.push(cwd.join("hub-rs").join(filename));
    candidates
}

pub fn resolve_ffmpeg_binary() -> PathBuf {
    ffmpeg_candidates(
        std::env::current_exe().ok().as_deref(),
        &std::env::current_dir().unwrap_or_default(),
    )
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

pub fn get_audio_devices() -> Vec<String> {
    let mut cmd = Command::new(resolve_ffmpeg_binary());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd
        .args(["-list_devices", "true", "-f", "dshow", "-i", "dummy"])
        .output();
    
    let mut devices = Vec::new();
    if let Ok(output) = output {
        let stderr = String::from_utf8_lossy(&output.stderr);
        for line in stderr.lines() {
            if line.contains("(audio)") {
                if let Some(start) = line.find('"') {
                    if let Some(end) = line[start + 1..].find('"') {
                        devices.push(line[start + 1..start + 1 + end].to_string());
                    }
                }
            }
        }
    }
    devices
}

#[derive(Clone, Debug, PartialEq)]
pub enum AudioConfig {
    None,
    Mic(String),
    System(String),
    Both { mic: String, sys: String },
}


pub fn get_default_mic() -> Option<String> {
    let devices = get_audio_devices();
    devices.into_iter().find(|d| d.contains("麦克风") || d.to_lowercase().contains("mic"))
}

pub fn get_default_sys_audio() -> Option<String> {
    let devices = get_audio_devices();
    devices.into_iter().find(|d| {
        let lower = d.to_lowercase();
        lower.contains("立体声混音") || lower.contains("stereo mix") || lower.contains("virtual-audio-capturer") || lower.contains("cable output")
    })
}

pub struct FfmpegEngine {
    process: Option<Child>,
    output_path: Option<PathBuf>,
    diagnostics: Arc<Mutex<VecDeque<u8>>>,
    stderr_reader: Option<std::thread::JoinHandle<()>>,
}

impl FfmpegEngine {
    pub fn new() -> Self {
        Self {
            process: None,
            output_path: None,
            stderr_reader: None,
            diagnostics: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn start(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        output_path: PathBuf,
        audio_config: AudioConfig,
    ) -> Result<(), String> {
        if self.process.is_some() {
            return Err("A recording is already in progress.".into());
        }
        if width <= 0
            || height <= 0
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
        {
            return Err(
                "Invalid recording region: positive dimensions and valid coordinates are required."
                    .into(),
            );
        }
        if !cfg!(windows) {
            return Err("Desktop recording currently requires Windows.".into());
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create recording directory: {e}"))?;
        }
        let mut cmd = Command::new(resolve_ffmpeg_binary());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostats",
            "-progress",
            "pipe:1",
            "-y",
        ]);
        
        match &audio_config {
            AudioConfig::None => {}
            AudioConfig::Mic(dev) => {
                let dev_name = if dev.is_empty() { get_default_mic().unwrap_or_default() } else { dev.clone() };
                if !dev_name.is_empty() {
                    cmd.args(["-f", "dshow", "-i", &format!("audio={dev_name}")]);
                }
            }
            AudioConfig::System(dev) => {
                let dev_name = if dev.is_empty() { get_default_sys_audio().unwrap_or_default() } else { dev.clone() };
                if !dev_name.is_empty() {
                    cmd.args(["-f", "dshow", "-i", &format!("audio={dev_name}")]);
                }
            }
            AudioConfig::Both { mic, sys } => {
                let mic_name = if mic.is_empty() { get_default_mic().unwrap_or_default() } else { mic.clone() };
                let sys_name = if sys.is_empty() { get_default_sys_audio().unwrap_or_default() } else { sys.clone() };
                let mut inputs = 0;
                if !sys_name.is_empty() {
                    cmd.args(["-f", "dshow", "-i", &format!("audio={sys_name}")]);
                    inputs += 1;
                }
                if !mic_name.is_empty() {
                    cmd.args(["-f", "dshow", "-i", &format!("audio={mic_name}")]);
                    inputs += 1;
                }
                if inputs == 2 {
                    cmd.args(["-filter_complex", "amix=inputs=2:duration=longest"]);
                }
            }
        }

        cmd.args([
            "-f",
            "gdigrab",
            "-framerate",
            "30",
            "-offset_x",
            &x.to_string(),
            "-offset_y",
            &y.to_string(),
            "-video_size",
            &format!("{width}x{height}"),
            "-i",
            "desktop",
        ]);
        Self::configure_output(&mut cmd);
        cmd.arg(&output_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.launch(cmd, output_path)
    }

    fn configure_output(cmd: &mut Command) {
        cmd.args([
            "-vf",
            EVEN_SIZE_FILTER,
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ]);
    }

    fn launch(&mut self, mut cmd: Command, output_path: PathBuf) -> Result<(), String> {
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Cannot start FFmpeg; check installation: {e}"))?;
        let stderr = child.stderr.take().ok_or("FFmpeg stderr unavailable")?;
        let stdout = child.stdout.take().ok_or("FFmpeg progress unavailable")?;
        let diagnostics = Arc::clone(&self.diagnostics);
        self.stderr_reader = Some(std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut buffer = [0u8; 1024];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                if let Ok(mut tail) = diagnostics.lock() {
                    tail.extend(&buffer[..count]);
                    while tail.len() > 8192 {
                        tail.pop_front();
                    }
                }
            }
        }));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line
                    .strip_prefix("frame=")
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .is_some_and(|n| n > 0)
                {
                    let _ = ready_tx.try_send(());
                }
            }
        });
        self.process = Some(child);
        self.output_path = Some(output_path.clone());
        if ready_rx.recv_timeout(START_TIMEOUT).is_err() {
            self.terminate();
            let _ = std::fs::remove_file(&output_path);
            self.output_path = None;
            return Err(self.failure("FFmpeg did not produce a video frame"));
        }
        // Short completed test streams may already have exited successfully.
        if let Some(status) = self
            .process
            .as_mut()
            .unwrap()
            .try_wait()
            .map_err(|e| e.to_string())?
        {
            if !status.success() {
                self.terminate();
                let _ = std::fs::remove_file(&output_path);
                self.output_path = None;
                return Err(self.failure("FFmpeg exited during startup"));
            }
        }
        Ok(())
    }

    fn failure(&self, message: &str) -> String {
        let tail = self
            .diagnostics
            .lock()
            .map(|bytes| {
                String::from_utf8_lossy(&bytes.iter().copied().collect::<Vec<_>>()).into_owned()
            })
            .unwrap_or_default();
        format!("{message}. {}", tail.trim())
    }

    fn terminate(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }

    pub fn stop(&mut self) -> Result<PathBuf, String> {
        let child = self.process.as_mut().ok_or("No active recording found.")?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"q\n");
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        let status = loop {
            match self.process.as_mut().unwrap().try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                Ok(None) => {
                    self.terminate();
                    return Err(self.failure("FFmpeg timed out while saving"));
                }
                Err(e) => {
                    self.terminate();
                    return Err(format!("Cannot wait for FFmpeg: {e}"));
                }
            }
        };
        self.process.take();
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        if !status.success() {
            return Err(self.failure("FFmpeg failed to save the recording"));
        }
        let output_path = self.output_path.take().ok_or("Recording path missing")?;
        if !std::fs::metadata(&output_path).is_ok_and(|m| m.is_file() && m.len() > 0) {
            return Err("FFmpeg did not save a non-empty video file.".into());
        }
        Ok(output_path)
    }
}

impl Drop for FfmpegEngine {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_region_before_launching_capture() {
        let mut engine = FfmpegEngine::new();
        for (x, width, height) in [(0, 0, 10), (0, 10, -1), (i32::MAX, 10, 10)] {
            assert!(
                engine
                    .start(x, 0, width, height, PathBuf::from("invalid.mp4"), AudioConfig::None)
                    .unwrap_err()
                    .contains("Invalid recording region")
            );
        }
    }

    #[test]
    #[ignore = "requires RECORDING_TEST_FFMPEG; does not capture the desktop"]
    fn startup_failure_reports_encoder_diagnostics_and_reaps_process() {
        let ffmpeg = std::env::var_os("RECORDING_TEST_FFMPEG").unwrap();
        let mut cmd = Command::new(ffmpeg);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.args([
            "-hide_banner",
            "-f",
            "not-a-real-input-format",
            "-i",
            "unused",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let output = std::env::temp_dir().join(format!(
            "deepcloud-failed-recording-{}.mp4",
            std::process::id()
        ));
        let mut engine = FfmpegEngine::new();
        let error = engine.launch(cmd, output.clone()).unwrap_err();
        assert!(error.contains("not-a-real-input-format"), "{error}");
        assert!(engine.process.is_none());
        assert!(!output.exists());
        assert!(engine.stop().is_err());
    }

    #[test]
    fn finds_project_encoder_when_electron_launches_hub_from_another_directory() {
        let paths = ffmpeg_candidates(
            Some(std::path::Path::new(
                "project/hub-rs/target/release/hub.exe",
            )),
            std::path::Path::new("project/deepcloud-ui"),
        );
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        assert!(paths.contains(&PathBuf::from("project/hub-rs").join(name)));
    }

    // A real encoder regression without accessing the user's desktop.
    #[test]
    #[ignore = "requires RECORDING_TEST_FFMPEG; encodes synthetic frames only"]
    fn odd_sized_frames_export_as_playable_mp4() {
        let ffmpeg = std::env::var_os("RECORDING_TEST_FFMPEG")
            .expect("Set RECORDING_TEST_FFMPEG to test with the shipped encoder");
        let output = std::env::temp_dir().join(format!(
            "deepcloud-recording-test-{}.mp4",
            std::process::id()
        ));
        let mut cmd = Command::new(&ffmpeg);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostats",
            "-progress",
            "pipe:1",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=size=101x99:rate=10,format=rgb24",
            "-frames:v",
            "10",
        ]);
        FfmpegEngine::configure_output(&mut cmd);
        cmd.arg(&output)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut engine = FfmpegEngine::new();
        engine.launch(cmd, output.clone()).unwrap();
        assert_eq!(engine.stop().unwrap(), output);
        let mut decode = Command::new(ffmpeg);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            decode.creation_flags(0x08000000);
        }
        let decoded = decode
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-f", "null", "-"])
            .output()
            .unwrap();
        let _ = std::fs::remove_file(output);
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
    }
}
