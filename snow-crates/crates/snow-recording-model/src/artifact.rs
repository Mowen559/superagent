use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::read_recording_bundle_footer;
use crate::shared::{IntermediateRecordingProfile, VideoEncodeConfig};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AudioTrackRole {
    SystemOutput,
    MicrophoneInput,
    Auxiliary,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AudioSampleFormat {
    PcmS16Le,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PauseInterval {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioTrackManifest {
    pub track_id: String,
    pub role: AudioTrackRole,
    pub asset_id: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub sample_format: AudioSampleFormat,
    pub duration_frames: u64,
    pub recorded: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionManifest {
    pub session_id: String,
    pub output_dir: PathBuf,
    pub keep_temp_files: bool,
    pub fps: u32,
    pub intermediate_profile: IntermediateRecordingProfile,
    pub recording_video: VideoEncodeConfig,
    pub width: u32,
    pub height: u32,
    pub capture_origin_x: i32,
    pub capture_origin_y: i32,
    pub audio_tracks: Vec<AudioTrackManifest>,
    pub pause_intervals: Vec<PauseInterval>,
}

#[derive(Clone, Debug)]
pub struct LocalRecordingPaths {
    pub temp_dir: PathBuf,
    pub video_intermediate_path: PathBuf,
    pub video_index_path: PathBuf,
    pub mouse_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct RecordingArtifact {
    pub session_id: String,
    pub output_dir: PathBuf,
    pub local_paths: LocalRecordingPaths,
    pub bundle_path: PathBuf,
    pub audio_tracks: Vec<AudioTrackManifest>,
}

impl RecordingArtifact {
    pub fn load_manifest(&self) -> Result<SessionManifest> {
        self.read_embedded_manifest()
    }

    pub fn read_embedded_manifest(&self) -> Result<SessionManifest> {
        Ok(read_recording_bundle_footer(&self.bundle_path)?.manifest)
    }
}
