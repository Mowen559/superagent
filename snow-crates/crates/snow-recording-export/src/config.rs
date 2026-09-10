use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use snow_recording_model::{
    IntermediateRecordingProfile, VideoEncodeConfig, VideoEncodingSpeed,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExportAudioTrackRequest {
    pub track_id: String,
    pub enabled: bool,
    pub volume: f32,
}

impl Default for ExportAudioTrackRequest {
    fn default() -> Self {
        Self {
            track_id: String::new(),
            enabled: true,
            volume: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportAudioOutputConfig {
    pub bitrate_kbps: u16,
}

impl Default for ExportAudioOutputConfig {
    fn default() -> Self {
        Self { bitrate_kbps: 192 }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct MouseEditConfig {
    pub visible: bool,
    pub trail_enabled: bool,
    pub trail_window_ms: u64,
    pub trail_smooth_step_px: f32,
    pub trail_max_alpha: u8,
    pub trail_color: [u8; 3],
    pub trail_thickness: i32,
    pub click_enabled: bool,
}

impl Default for MouseEditConfig {
    fn default() -> Self {
        Self {
            visible: false,
            trail_enabled: false,
            trail_window_ms: 128,
            trail_smooth_step_px: 2.0,
            trail_max_alpha: 180,
            trail_color: [255, 32, 32],
            trail_thickness: 2,
            click_enabled: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExportFormat {
    #[default]
    Mp4,
    Avi,
    Gif,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExportExecutionMode {
    #[default]
    HardwarePreferred,
    HardwareOnly,
    SoftwareOnly,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SoftwareH264Priority {
    OpenH264First,
    #[default]
    X264First,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportPerformanceConfig {
    pub mode: ExportExecutionMode,
    pub software_h264_priority: SoftwareH264Priority,
    pub queue_depth: u16,
    pub memory_budget_mb: u32,
    pub decode_threads: u8,
    pub process_threads: u8,
    pub encode_threads: u8,
}

impl Default for ExportPerformanceConfig {
    fn default() -> Self {
        Self {
            mode: ExportExecutionMode::HardwarePreferred,
            software_h264_priority: SoftwareH264Priority::X264First,
            queue_depth: 96,
            memory_budget_mb: 8192,
            decode_threads: 0,
            process_threads: 0,
            encode_threads: 0,
        }
    }
}

impl ExportPerformanceConfig {
    pub fn validate(&self, prefix: &str) -> Result<(), String> {
        if self.queue_depth == 0 {
            return Err(format!("{prefix}.queue_depth must be > 0"));
        }
        if self.queue_depth > 1024 {
            return Err(format!("{prefix}.queue_depth must be <= 1024"));
        }
        if self.memory_budget_mb < 256 {
            return Err(format!("{prefix}.memory_budget_mb must be >= 256"));
        }
        if self.memory_budget_mb > 262_144 {
            return Err(format!("{prefix}.memory_budget_mb must be <= 262144"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExportRequest {
    pub playback_speed: f32,
    pub audio_tracks: Vec<ExportAudioTrackRequest>,
    pub audio_output: ExportAudioOutputConfig,
    pub mouse: MouseEditConfig,
    pub format: ExportFormat,
    pub output_path: PathBuf,
    pub video: VideoEncodeConfig,
    pub performance: ExportPerformanceConfig,
}

impl Default for ExportRequest {
    fn default() -> Self {
        Self {
            playback_speed: 1.0,
            audio_tracks: Vec::new(),
            audio_output: ExportAudioOutputConfig::default(),
            mouse: MouseEditConfig::default(),
            format: ExportFormat::Mp4,
            output_path: PathBuf::from("output.mp4"),
            video: VideoEncodeConfig::default(),
            performance: ExportPerformanceConfig::default(),
        }
    }
}

impl ExportRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.25..=4.0).contains(&self.playback_speed) {
            return Err("playback_speed must be in 0.25..=4.0".to_string());
        }

        for track in &self.audio_tracks {
            if track.track_id.is_empty() {
                return Err("audio_tracks[].track_id must not be empty".to_string());
            }
            if !(0.0..=2.0).contains(&track.volume) {
                return Err(format!(
                    "audio_tracks[{}].volume must be in 0.0..=2.0",
                    track.track_id
                ));
            }
        }

        if self.audio_output.bitrate_kbps == 0 {
            return Err("audio_output.bitrate_kbps must be > 0".to_string());
        }

        if self.mouse.trail_window_ms == 0 {
            return Err("mouse.trail_window_ms must be > 0".to_string());
        }

        if !self.mouse.trail_smooth_step_px.is_finite() || self.mouse.trail_smooth_step_px <= 0.0 {
            return Err("mouse.trail_smooth_step_px must be finite and > 0".to_string());
        }

        if self.mouse.trail_thickness < 0 {
            return Err("mouse.trail_thickness must be >= 0".to_string());
        }

        if self.output_path.as_os_str().is_empty() {
            return Err("output_path must not be empty".to_string());
        }

        self.video.validate("video")?;
        self.performance.validate("performance")?;
        Ok(())
    }
}
