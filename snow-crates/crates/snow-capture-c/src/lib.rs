#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::ptr;
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

use snow_capture::frame::Frame;
use snow_capture::{
    CaptureOptions, CaptureRegion, CaptureSession, CaptureSystem, CaptureTarget, CaptureWorkload,
    MonitorId, MonitorLayout,
};
use snow_screen_recorder::{
    EditingSession, ExportFormat, ExportRequest, RecordingAudioConfig, RecordingAudioTrackConfig,
    RecordingConfig, RecordingRegion, RecordingSession, RecordingState, RecordingTarget,
    VideoEncodeConfig, VideoEncodingSpeed,
};

pub struct SnowCaptureDesktopSessionImpl {
    system: CaptureSystem,
    options: CaptureOptions,
    workers: Vec<MonitorWorker>,
    prepared: bool,
}

pub struct SnowCaptureRegionSessionImpl {
    session: CaptureSession,
    frame: Frame,
}

pub struct SnowCaptureSnapshotImpl {
    frames: Vec<SnapshotFrame>,
}

pub struct SnowCaptureFrameLeaseImpl {
    _frame: Arc<Frame>,
}

pub struct SnowCaptureRecordingSessionImpl {
    recording: Option<RecordingSession>,
    state: RecordingState,
}

#[repr(C)]
pub struct SnowCaptureDesktopSessionConfig {
    capture_retry_count: usize,
    reserved: [u8; 32],
}

#[repr(C)]
pub struct SnowCaptureDesktopSessionState {
    worker_count: usize,
    prepared: u8,
    reserved0: [u8; 7],
    retained_resource_bytes: u64,
    backend_kind: *const c_char,
}

#[repr(C)]
pub struct SnowCaptureFrameInfo {
    pub stable_id: *const c_char,
    pub name: *const c_char,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: u8,
    pub reserved0: [u8; 3],
    pub stride_bytes: u32,
    pub rgba_bytes: *const u8,
    pub rgba_len: usize,
}

#[repr(C)]
pub struct SnowCaptureRegionSessionConfig {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub capture_retry_count: usize,
    pub reserved: [u8; 32],
}

#[repr(C)]
pub struct SnowCaptureRegionFrameInfo {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub is_duplicate: u8,
    pub reserved0: [u8; 3],
    pub rgba_bytes: *const u8,
    pub rgba_len: usize,
}

#[repr(C)]
pub struct SnowCaptureRecordingConfig {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    fps: u32,
    enable_microphone: u8,
    enable_system_audio: u8,
    reserved0: [u8; 2],
    working_directory_utf8: *const c_char,
    reserved: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnowCaptureRecordingState {
    Created = 0,
    Running = 1,
    Paused = 2,
    Stopped = 3,
}

#[derive(Clone)]
struct MonitorEntry {
    id: MonitorId,
    stable_id: CString,
    name: CString,
    x: i32,
    y: i32,
    is_primary: bool,
}

struct MonitorWorker {
    entry: MonitorEntry,
    tx: mpsc::Sender<WorkerCommand>,
    join: Option<JoinHandle<()>>,
}

struct SnapshotFrame {
    entry: MonitorEntry,
    frame: Arc<Frame>,
}

enum WorkerCommand {
    Prepare(mpsc::Sender<Result<(), String>>),
    Capture(mpsc::Sender<Result<Frame, String>>),
    ReleaseIdleResources(mpsc::Sender<Result<(), String>>),
    Stop,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").expect("empty string is valid C string"));
}

fn sanitize_cstring(value: impl AsRef<str>) -> CString {
    let bytes = value
        .as_ref()
        .as_bytes()
        .iter()
        .copied()
        .filter(|byte| *byte != 0)
        .collect::<Vec<_>>();
    CString::new(bytes).expect("interior NUL bytes were filtered")
}

fn set_last_error(error: impl ToString) {
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = sanitize_cstring(error.to_string());
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = CString::new("").expect("empty string is valid C string");
    });
}

fn default_options(config: *const SnowCaptureDesktopSessionConfig) -> CaptureOptions {
    let capture_retry_count = if config.is_null() {
        1
    } else {
        let requested = unsafe { (*config).capture_retry_count };
        if requested == 0 { 1 } else { requested }
    };

    CaptureOptions {
        capture_retry_count,
        workload: CaptureWorkload::Snapshot,
        gpu_hdr_conversion: true,
        hdr_tonemap_lut: true,
    }
}

fn snapshot_options(capture_retry_count: usize) -> CaptureOptions {
    CaptureOptions {
        capture_retry_count: capture_retry_count.max(1),
        workload: CaptureWorkload::Snapshot,
        gpu_hdr_conversion: true,
        hdr_tonemap_lut: true,
    }
}

fn build_monitor_entries(system: &CaptureSystem) -> Result<Vec<MonitorEntry>, String> {
    let MonitorLayout { monitors, .. } = system.monitor_layout().map_err(|err| err.to_string())?;
    Ok(monitors
        .into_iter()
        .map(|geometry| {
            let stable_id = geometry.monitor.stable_id();
            let name = geometry.monitor.name().to_owned();
            MonitorEntry {
                id: geometry.monitor.clone(),
                stable_id: sanitize_cstring(stable_id),
                name: sanitize_cstring(name),
                x: geometry.x,
                y: geometry.y,
                is_primary: geometry.monitor.is_primary(),
            }
        })
        .collect())
}

impl MonitorWorker {
    fn start(
        system: CaptureSystem,
        options: CaptureOptions,
        entry: MonitorEntry,
    ) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<WorkerCommand>();
        let worker_entry = entry.clone();
        let join = thread::Builder::new()
            .name("snow-capture-monitor".to_owned())
            .spawn(move || {
                let mut session = system
                    .open_session(CaptureTarget::Monitor(worker_entry.id), options)
                    .map_err(|err| err.to_string());
                let mut reusable_frame = Frame::empty();

                while let Ok(command) = rx.recv() {
                    match command {
                        WorkerCommand::Prepare(reply) => {
                            let result = match session.as_mut() {
                                Ok(capture_session) => capture_session
                                    .prepare_target()
                                    .map(|_| ())
                                    .map_err(|err| err.to_string()),
                                Err(error) => Err(error.clone()),
                            };
                            let _ = reply.send(result);
                        }
                        WorkerCommand::Capture(reply) => {
                            let result = match session.as_mut() {
                                Ok(session) => session
                                    .capture_into(&mut reusable_frame)
                                    .map(|()| reusable_frame.clone())
                                    .map_err(|err| err.to_string()),
                                Err(error) => Err(error.clone()),
                            };
                            let _ = reply.send(result);
                        }
                        WorkerCommand::ReleaseIdleResources(reply) => {
                            let result = match session.as_mut() {
                                Ok(capture_session) => {
                                    capture_session.release_idle_resources();
                                    reusable_frame = Frame::empty();
                                    Ok(())
                                }
                                Err(error) => Err(error.clone()),
                            };
                            let _ = reply.send(result);
                        }
                        WorkerCommand::Stop => break,
                    }
                }
            })
            .map_err(|err| format!("failed to spawn capture monitor worker: {err}"))?;

        Ok(Self {
            entry,
            tx,
            join: Some(join),
        })
    }

    fn stop(mut self) {
        let _ = self.tx.send(WorkerCommand::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }

    fn request_prepare(&self) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(WorkerCommand::Prepare(tx))
            .map_err(|_| "capture worker is not running".to_owned())?;
        Ok(rx)
    }

    fn request_capture(&self) -> Result<mpsc::Receiver<Result<Frame, String>>, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(WorkerCommand::Capture(tx))
            .map_err(|_| "capture worker is not running".to_owned())?;
        Ok(rx)
    }

    fn request_release_idle_resources(&self) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(WorkerCommand::ReleaseIdleResources(tx))
            .map_err(|_| "capture worker is not running".to_owned())?;
        Ok(rx)
    }
}

impl Drop for SnowCaptureDesktopSessionImpl {
    fn drop(&mut self) {
        for worker in std::mem::take(&mut self.workers) {
            worker.stop();
        }
    }
}

fn session_mut<'a>(
    session: *mut SnowCaptureDesktopSessionImpl,
) -> Option<&'a mut SnowCaptureDesktopSessionImpl> {
    if session.is_null() {
        set_last_error("desktop session is null");
        None
    } else {
        Some(unsafe { &mut *session })
    }
}

fn snapshot_ref<'a>(
    snapshot: *const SnowCaptureSnapshotImpl,
) -> Option<&'a SnowCaptureSnapshotImpl> {
    if snapshot.is_null() {
        set_last_error("snapshot is null");
        None
    } else {
        Some(unsafe { &*snapshot })
    }
}

fn rebuild_workers(session: &mut SnowCaptureDesktopSessionImpl) -> Result<(), String> {
    session
        .system
        .refresh_display_configuration()
        .map_err(|err| err.to_string())?;

    let entries = build_monitor_entries(&session.system)?;
    let old_workers = std::mem::take(&mut session.workers);
    let mut next_workers = Vec::with_capacity(entries.len());

    for entry in entries {
        match MonitorWorker::start(session.system.clone(), session.options, entry) {
            Ok(worker) => next_workers.push(worker),
            Err(error) => {
                for worker in next_workers {
                    worker.stop();
                }
                session.workers = old_workers;
                session.prepared = false;
                return Err(error);
            }
        }
    }

    for worker in old_workers {
        worker.stop();
    }

    session.workers = next_workers;
    session.prepared = false;
    Ok(())
}

fn capture_all_frames(
    session: &mut SnowCaptureDesktopSessionImpl,
) -> Result<Vec<SnapshotFrame>, String> {
    let mut receivers = Vec::with_capacity(session.workers.len());
    for worker in &session.workers {
        match worker.request_capture() {
            Ok(receiver) => receivers.push((worker.entry.clone(), receiver)),
            Err(error) => return Err(error),
        }
    }

    let mut frames = Vec::with_capacity(receivers.len());
    for (entry, receiver) in receivers {
        match receiver.recv() {
            Ok(Ok(frame)) => {
                frames.push(SnapshotFrame {
                    entry,
                    frame: Arc::new(frame),
                });
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err("capture worker stopped before capture completed".to_owned()),
        }
    }

    Ok(frames)
}

fn backend_kind_ptr(session: &SnowCaptureDesktopSessionImpl) -> *const c_char {
    match session.system.backend_kind().as_str() {
        "auto" => c"auto".as_ptr(),
        "dxgi" => c"dxgi".as_ptr(),
        "wgc" => c"wgc".as_ptr(),
        "gdi" => c"gdi".as_ptr(),
        _ => c"unknown".as_ptr(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_desktop_session_create(
    config: *const SnowCaptureDesktopSessionConfig,
) -> *mut SnowCaptureDesktopSessionImpl {
    let options = default_options(config);
    match CaptureSystem::builder().build() {
        Ok(system) => {
            let mut session = SnowCaptureDesktopSessionImpl {
                system,
                options,
                workers: Vec::new(),
                prepared: false,
            };
            if let Err(error) = rebuild_workers(&mut session) {
                set_last_error(error);
            } else {
                clear_last_error();
            }
            Box::into_raw(Box::new(session))
        }
        Err(error) => {
            set_last_error(error);
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_desktop_session_destroy(
    session: *mut SnowCaptureDesktopSessionImpl,
) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_desktop_session_prepare(
    session: *mut SnowCaptureDesktopSessionImpl,
) -> u8 {
    let Some(session) = session_mut(session) else {
        return 0;
    };

    let receivers = match session
        .workers
        .iter()
        .map(MonitorWorker::request_prepare)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(receivers) => receivers,
        Err(error) => {
            set_last_error(error);
            return 0;
        }
    };

    for receiver in receivers {
        match receiver.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                set_last_error(error);
                return 0;
            }
            Err(_) => {
                set_last_error("capture worker stopped before prepare completed");
                return 0;
            }
        }
    }

    clear_last_error();
    session.prepared = true;
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_desktop_session_state(
    session: *mut SnowCaptureDesktopSessionImpl,
    out_state: *mut SnowCaptureDesktopSessionState,
) -> u8 {
    if out_state.is_null() {
        set_last_error("out_state is null");
        return 0;
    }
    let Some(session) = session_mut(session) else {
        return 0;
    };

    unsafe {
        *out_state = SnowCaptureDesktopSessionState {
            worker_count: session.workers.len(),
            prepared: u8::from(session.prepared),
            reserved0: [0; 7],
            retained_resource_bytes: 0,
            backend_kind: backend_kind_ptr(session),
        };
    }
    clear_last_error();
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_desktop_session_refresh_layout(
    session: *mut SnowCaptureDesktopSessionImpl,
) -> u8 {
    let Some(session) = session_mut(session) else {
        return 0;
    };

    match rebuild_workers(session) {
        Ok(()) => {
            let ok = snow_capture_desktop_session_prepare(session);
            if ok != 0 {
                clear_last_error();
            }
            ok
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_desktop_session_release_idle_resources(
    session: *mut SnowCaptureDesktopSessionImpl,
) -> u8 {
    let Some(session) = session_mut(session) else {
        return 0;
    };

    let receivers = match session
        .workers
        .iter()
        .map(MonitorWorker::request_release_idle_resources)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(receivers) => receivers,
        Err(error) => {
            set_last_error(error);
            return 0;
        }
    };

    for receiver in receivers {
        match receiver.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                set_last_error(error);
                return 0;
            }
            Err(_) => {
                set_last_error("capture worker stopped before idle release completed");
                return 0;
            }
        }
    }

    clear_last_error();
    session.prepared = false;
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_desktop_session_capture_all(
    session: *mut SnowCaptureDesktopSessionImpl,
) -> *mut SnowCaptureSnapshotImpl {
    let Some(session) = session_mut(session) else {
        return ptr::null_mut();
    };

    let frames = match capture_all_frames(session) {
        Ok(frames) => frames,
        Err(first_error) => {
            if let Err(refresh_error) = rebuild_workers(session) {
                set_last_error(format!(
                    "{first_error}; layout refresh failed: {refresh_error}"
                ));
                return ptr::null_mut();
            }
            match capture_all_frames(session) {
                Ok(frames) => frames,
                Err(retry_error) => {
                    set_last_error(format!(
                        "{first_error}; retry after layout refresh failed: {retry_error}"
                    ));
                    return ptr::null_mut();
                }
            }
        }
    };

    session.prepared = true;
    clear_last_error();
    Box::into_raw(Box::new(SnowCaptureSnapshotImpl { frames }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_region_session_create(
    config: *const SnowCaptureRegionSessionConfig,
) -> *mut SnowCaptureRegionSessionImpl {
    if config.is_null() {
        set_last_error("region session config is null");
        return ptr::null_mut();
    }
    let config = unsafe { &*config };
    let region = match CaptureRegion::new(config.x, config.y, config.width, config.height) {
        Ok(region) => region,
        Err(error) => {
            set_last_error(error);
            return ptr::null_mut();
        }
    };
    let system = match CaptureSystem::builder().build() {
        Ok(system) => system,
        Err(error) => {
            set_last_error(error);
            return ptr::null_mut();
        }
    };
    let session = match system.open_session(
        CaptureTarget::Region(region),
        snapshot_options(config.capture_retry_count),
    ) {
        Ok(session) => session,
        Err(error) => {
            set_last_error(error);
            return ptr::null_mut();
        }
    };
    clear_last_error();
    Box::into_raw(Box::new(SnowCaptureRegionSessionImpl {
        session,
        frame: Frame::empty(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_region_session_destroy(
    session: *mut SnowCaptureRegionSessionImpl,
) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_region_session_prepare(
    session: *mut SnowCaptureRegionSessionImpl,
) -> u8 {
    if session.is_null() {
        set_last_error("region session is null");
        return 0;
    }
    let session = unsafe { &mut *session };
    match session.session.prepare_target() {
        Ok(_) => {
            clear_last_error();
            1
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_region_session_capture(
    session: *mut SnowCaptureRegionSessionImpl,
    out_info: *mut SnowCaptureRegionFrameInfo,
) -> u8 {
    if out_info.is_null() {
        set_last_error("region frame out_info is null");
        return 0;
    }
    if session.is_null() {
        set_last_error("region session is null");
        return 0;
    }
    let session = unsafe { &mut *session };
    if let Err(error) = session.session.capture_into(&mut session.frame) {
        set_last_error(error);
        return 0;
    }
    let stride_bytes = match session.frame.width().checked_mul(4) {
        Some(stride) => stride,
        None => {
            set_last_error("region frame stride overflow");
            return 0;
        }
    };
    let rgba = session.frame.as_rgba_bytes();
    unsafe {
        *out_info = SnowCaptureRegionFrameInfo {
            width: session.frame.width(),
            height: session.frame.height(),
            stride_bytes,
            is_duplicate: u8::from(session.frame.metadata().is_duplicate()),
            reserved0: [0; 3],
            rgba_bytes: rgba.as_ptr(),
            rgba_len: rgba.len(),
        };
    }
    clear_last_error();
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_snapshot_count(snapshot: *const SnowCaptureSnapshotImpl) -> usize {
    snapshot_ref(snapshot).map_or(0, |snapshot| snapshot.frames.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_snapshot_frame_info(
    snapshot: *const SnowCaptureSnapshotImpl,
    index: usize,
    out_info: *mut SnowCaptureFrameInfo,
) -> u8 {
    if out_info.is_null() {
        set_last_error("out_info is null");
        return 0;
    }
    let Some(snapshot) = snapshot_ref(snapshot) else {
        return 0;
    };
    let Some(frame) = snapshot.frames.get(index) else {
        set_last_error("snapshot frame index is out of range");
        return 0;
    };

    let rgba = frame.frame.as_rgba_bytes();
    let stride_bytes = match frame.frame.width().checked_mul(4) {
        Some(stride) => stride,
        None => {
            set_last_error("frame stride overflow");
            return 0;
        }
    };
    let required_len = match usize::try_from(stride_bytes)
        .ok()
        .and_then(|stride| stride.checked_mul(frame.frame.height() as usize))
    {
        Some(len) => len,
        None => {
            set_last_error("frame length overflow");
            return 0;
        }
    };
    if rgba.len() < required_len {
        set_last_error("frame buffer is smaller than the reported dimensions");
        return 0;
    }

    unsafe {
        *out_info = SnowCaptureFrameInfo {
            stable_id: frame.entry.stable_id.as_ptr(),
            name: frame.entry.name.as_ptr(),
            x: frame.entry.x,
            y: frame.entry.y,
            width: frame.frame.width(),
            height: frame.frame.height(),
            is_primary: u8::from(frame.entry.is_primary),
            reserved0: [0; 3],
            stride_bytes,
            rgba_bytes: rgba.as_ptr(),
            rgba_len: rgba.len(),
        };
    }
    clear_last_error();
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_snapshot_frame_retain(
    snapshot: *const SnowCaptureSnapshotImpl,
    index: usize,
) -> *mut SnowCaptureFrameLeaseImpl {
    let Some(snapshot) = snapshot_ref(snapshot) else {
        return ptr::null_mut();
    };
    let Some(frame) = snapshot.frames.get(index) else {
        set_last_error("snapshot frame index is out of range");
        return ptr::null_mut();
    };

    clear_last_error();
    Box::into_raw(Box::new(SnowCaptureFrameLeaseImpl {
        _frame: Arc::clone(&frame.frame),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_frame_lease_release(lease: *mut SnowCaptureFrameLeaseImpl) {
    if !lease.is_null() {
        drop(unsafe { Box::from_raw(lease) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_snapshot_destroy(snapshot: *mut SnowCaptureSnapshotImpl) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
}

fn recording_session_mut<'a>(
    session: *mut SnowCaptureRecordingSessionImpl,
) -> Option<&'a mut SnowCaptureRecordingSessionImpl> {
    if session.is_null() {
        set_last_error("recording session is null");
        None
    } else {
        Some(unsafe { &mut *session })
    }
}

fn recording_session_ref<'a>(
    session: *const SnowCaptureRecordingSessionImpl,
) -> Option<&'a SnowCaptureRecordingSessionImpl> {
    if session.is_null() {
        set_last_error("recording session is null");
        None
    } else {
        Some(unsafe { &*session })
    }
}

fn path_from_utf8(value: *const c_char, label: &str) -> Result<PathBuf, String> {
    if value.is_null() {
        return Err(format!("{label} is null"));
    }
    let value = unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|_| format!("{label} is not valid UTF-8"))?;
    if value.is_empty() {
        return Err(format!("{label} is empty"));
    }
    Ok(PathBuf::from(value))
}

fn ffi_recording_state(state: RecordingState) -> SnowCaptureRecordingState {
    match state {
        RecordingState::Created => SnowCaptureRecordingState::Created,
        RecordingState::Running => SnowCaptureRecordingState::Running,
        RecordingState::Paused => SnowCaptureRecordingState::Paused,
        RecordingState::Stopped => SnowCaptureRecordingState::Stopped,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_recording_session_create(
    config: *const SnowCaptureRecordingConfig,
) -> *mut SnowCaptureRecordingSessionImpl {
    if config.is_null() {
        set_last_error("recording config is null");
        return ptr::null_mut();
    }
    let config = unsafe { &*config };
    if config.width == 0 || config.height == 0 {
        set_last_error("recording region must have a non-zero width and height");
        return ptr::null_mut();
    }
    if config.width % 2 != 0 || config.height % 2 != 0 {
        set_last_error("recording region width and height must be even");
        return ptr::null_mut();
    }
    if config.fps == 0 {
        set_last_error("recording fps must be greater than zero");
        return ptr::null_mut();
    }
    let output_dir = match path_from_utf8(config.working_directory_utf8, "working directory") {
        Ok(path) => path,
        Err(error) => {
            set_last_error(error);
            return ptr::null_mut();
        }
    };

    let audio = RecordingAudioConfig {
        tracks: vec![
            RecordingAudioTrackConfig {
                enabled: config.enable_system_audio != 0,
                ..RecordingAudioTrackConfig::system_default("system")
            },
            RecordingAudioTrackConfig {
                enabled: config.enable_microphone != 0,
                ..RecordingAudioTrackConfig::microphone_default("microphone")
            },
        ],
        ..RecordingAudioConfig::default()
    };
    let recording_config = RecordingConfig {
        target: RecordingTarget::Region(RecordingRegion::new(
            config.x,
            config.y,
            config.width,
            config.height,
        )),
        output_dir,
        fps: config.fps,
        video: VideoEncodeConfig {
            quality: 80,
            speed: VideoEncodingSpeed::UltraFast,
        },
        audio,
        ..RecordingConfig::default()
    };

    match RecordingSession::create(recording_config) {
        Ok(recording) => {
            clear_last_error();
            Box::into_raw(Box::new(SnowCaptureRecordingSessionImpl {
                recording: Some(recording),
                state: RecordingState::Created,
            }))
        }
        Err(error) => {
            set_last_error(error);
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_recording_session_destroy(
    session: *mut SnowCaptureRecordingSessionImpl,
) {
    if session.is_null() {
        return;
    }
    let mut session = unsafe { Box::from_raw(session) };
    if matches!(
        session.state,
        RecordingState::Running | RecordingState::Paused
    ) && let Some(recording) = session.recording.take()
    {
        let _ = recording.stop();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_recording_session_start(
    session: *mut SnowCaptureRecordingSessionImpl,
) -> u8 {
    let Some(session) = recording_session_mut(session) else {
        return 0;
    };
    let Some(recording) = session.recording.as_mut() else {
        set_last_error("recording session has already stopped");
        return 0;
    };
    match recording.start() {
        Ok(()) => {
            session.state = RecordingState::Running;
            clear_last_error();
            1
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_recording_session_pause(
    session: *mut SnowCaptureRecordingSessionImpl,
) -> u8 {
    let Some(session) = recording_session_mut(session) else {
        return 0;
    };
    let Some(recording) = session.recording.as_ref() else {
        set_last_error("recording session has already stopped");
        return 0;
    };
    match recording.pause() {
        Ok(()) => {
            session.state = RecordingState::Paused;
            clear_last_error();
            1
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_recording_session_resume(
    session: *mut SnowCaptureRecordingSessionImpl,
) -> u8 {
    let Some(session) = recording_session_mut(session) else {
        return 0;
    };
    let Some(recording) = session.recording.as_ref() else {
        set_last_error("recording session has already stopped");
        return 0;
    };
    match recording.resume() {
        Ok(()) => {
            session.state = RecordingState::Running;
            clear_last_error();
            1
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snow_capture_recording_session_state(
    session: *const SnowCaptureRecordingSessionImpl,
    out_state: *mut SnowCaptureRecordingState,
) -> u8 {
    if out_state.is_null() {
        set_last_error("recording out_state is null");
        return 0;
    }
    let Some(session) = recording_session_ref(session) else {
        return 0;
    };
    unsafe { *out_state = ffi_recording_state(session.state) };
    clear_last_error();
    1
}

fn configure_recording_export_request(
    mut request: ExportRequest,
    output_path: PathBuf,
    export_gif: bool,
) -> ExportRequest {
    request.output_path = output_path;
    request.format = if export_gif {
        ExportFormat::Gif
    } else {
        ExportFormat::Mp4
    };
    request.mouse.visible = true;
    for track in &mut request.audio_tracks {
        track.enabled = true;
    }
    request
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_recording_session_stop_and_export(
    session: *mut SnowCaptureRecordingSessionImpl,
    output_file_utf8: *const c_char,
    export_gif: u8,
) -> u8 {
    let Some(session) = recording_session_mut(session) else {
        return 0;
    };
    let output_path = match path_from_utf8(output_file_utf8, "output file") {
        Ok(path) => path,
        Err(error) => {
            set_last_error(error);
            return 0;
        }
    };
    let Some(recording) = session.recording.take() else {
        set_last_error("recording session has already stopped");
        return 0;
    };

    let artifact = match recording.stop() {
        Ok(artifact) => artifact,
        Err(error) => {
            session.state = RecordingState::Stopped;
            set_last_error(error);
            return 0;
        }
    };
    session.state = RecordingState::Stopped;

    let bundle_path = artifact.bundle_path.clone();
    let editing = match EditingSession::open(artifact) {
        Ok(editing) => editing,
        Err(error) => {
            let _ = std::fs::remove_file(bundle_path);
            set_last_error(error);
            return 0;
        }
    };
    let request =
        configure_recording_export_request(editing.export_request(), output_path, export_gif != 0);

    let result = editing.export(request);
    let _ = std::fs::remove_file(bundle_path);
    match result {
        Ok(_) => {
            clear_last_error();
            1
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snow_capture_last_error_message() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry() -> MonitorEntry {
        MonitorEntry {
            id: MonitorId::from_parts(1, 2, 3, "unit-monitor", true),
            stable_id: sanitize_cstring("stable-unit-monitor"),
            name: sanitize_cstring("unit-monitor"),
            x: -10,
            y: 20,
            is_primary: true,
        }
    }

    fn test_snapshot() -> *mut SnowCaptureSnapshotImpl {
        let frame = Frame::from_rgba8(2, 2, vec![7; 16]).expect("valid test frame");
        Box::into_raw(Box::new(SnowCaptureSnapshotImpl {
            frames: vec![SnapshotFrame {
                entry: test_entry(),
                frame: Arc::new(frame),
            }],
        }))
    }

    #[test]
    fn immediate_gif_export_includes_recorded_cursor_motion() {
        let output_path = PathBuf::from("recording.gif");
        let request =
            configure_recording_export_request(ExportRequest::default(), output_path.clone(), true);

        assert_eq!(request.output_path, output_path);
        assert_eq!(request.format, ExportFormat::Gif);
        assert!(request.mouse.visible);
    }

    #[test]
    fn null_snapshot_info_fails() {
        let mut info = SnowCaptureFrameInfo {
            stable_id: ptr::null(),
            name: ptr::null(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            is_primary: 0,
            reserved0: [0; 3],
            stride_bytes: 0,
            rgba_bytes: ptr::null(),
            rgba_len: 0,
        };

        let ok = unsafe { snow_capture_snapshot_frame_info(ptr::null(), 0, &mut info) };
        assert_eq!(ok, 0);
        assert!(!snow_capture_last_error_message().is_null());
    }

    #[test]
    fn snapshot_frame_info_reports_monitor_and_tight_stride() {
        let snapshot = test_snapshot();
        let mut info = SnowCaptureFrameInfo {
            stable_id: ptr::null(),
            name: ptr::null(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            is_primary: 0,
            reserved0: [0; 3],
            stride_bytes: 0,
            rgba_bytes: ptr::null(),
            rgba_len: 0,
        };

        let ok = unsafe { snow_capture_snapshot_frame_info(snapshot, 0, &mut info) };
        assert_eq!(ok, 1);
        assert_eq!(info.x, -10);
        assert_eq!(info.y, 20);
        assert_eq!(info.width, 2);
        assert_eq!(info.height, 2);
        assert_eq!(info.stride_bytes, 8);
        assert_eq!(info.rgba_len, 16);
        assert!(!info.rgba_bytes.is_null());

        unsafe { snow_capture_snapshot_destroy(snapshot) };
    }

    #[test]
    fn frame_lease_survives_snapshot_destroy() {
        let snapshot = test_snapshot();
        let lease = snow_capture_snapshot_frame_retain(snapshot, 0);
        assert!(!lease.is_null());
        unsafe { snow_capture_snapshot_destroy(snapshot) };

        let lease_ref = unsafe { &*lease };
        assert_eq!(lease_ref._frame.width(), 2);
        assert_eq!(lease_ref._frame.height(), 2);
        assert!(
            lease_ref
                ._frame
                .as_rgba_bytes()
                .iter()
                .all(|byte| *byte == 7)
        );

        unsafe { snow_capture_frame_lease_release(lease) };
    }

    #[test]
    fn release_idle_resources_null_session_fails() {
        let ok = snow_capture_desktop_session_release_idle_resources(ptr::null_mut());
        assert_eq!(ok, 0);
        assert!(!snow_capture_last_error_message().is_null());
    }

    #[test]
    fn region_session_rejects_null_and_empty_config() {
        assert!(unsafe { snow_capture_region_session_create(ptr::null()) }.is_null());

        let config = SnowCaptureRegionSessionConfig {
            x: 0,
            y: 0,
            width: 0,
            height: 100,
            capture_retry_count: 1,
            reserved: [0; 32],
        };
        assert!(unsafe { snow_capture_region_session_create(&config) }.is_null());
    }

    #[test]
    fn region_capture_rejects_null_handles() {
        let mut info = SnowCaptureRegionFrameInfo {
            width: 0,
            height: 0,
            stride_bytes: 0,
            is_duplicate: 0,
            reserved0: [0; 3],
            rgba_bytes: ptr::null(),
            rgba_len: 0,
        };
        assert_eq!(
            unsafe { snow_capture_region_session_capture(ptr::null_mut(), &mut info) },
            0
        );
        assert_eq!(
            unsafe { snow_capture_region_session_prepare(ptr::null_mut()) },
            0
        );
    }

    #[test]
    fn odd_recording_region_is_rejected_before_session_creation() {
        let config = SnowCaptureRecordingConfig {
            x: 0,
            y: 0,
            width: 801,
            height: 451,
            fps: 60,
            enable_microphone: 0,
            enable_system_audio: 0,
            reserved0: [0; 2],
            working_directory_utf8: ptr::null(),
            reserved: [0; 32],
        };

        let session = unsafe { snow_capture_recording_session_create(&config) };
        assert!(session.is_null());
        let error = unsafe { CStr::from_ptr(snow_capture_last_error_message()) };
        assert_eq!(
            error.to_str().expect("recording error should be UTF-8"),
            "recording region width and height must be even"
        );
    }

    #[test]
    fn release_idle_resources_empty_session_succeeds() {
        let system = CaptureSystem::builder()
            .build()
            .expect("capture system should initialize");
        let mut session = SnowCaptureDesktopSessionImpl {
            system,
            options: CaptureOptions::default(),
            workers: Vec::new(),
            prepared: false,
        };

        let ok = snow_capture_desktop_session_release_idle_resources(&mut session);
        assert_eq!(ok, 1);
    }

    #[test]
    fn desktop_session_state_reports_worker_count_and_prepared_flag() {
        let system = CaptureSystem::builder()
            .build()
            .expect("capture system should initialize");
        let mut session = SnowCaptureDesktopSessionImpl {
            system,
            options: CaptureOptions::default(),
            workers: Vec::new(),
            prepared: true,
        };
        let mut state = SnowCaptureDesktopSessionState {
            worker_count: usize::MAX,
            prepared: 0,
            reserved0: [1; 7],
            retained_resource_bytes: 99,
            backend_kind: ptr::null(),
        };

        let ok = unsafe { snow_capture_desktop_session_state(&mut session, &mut state) };
        assert_eq!(ok, 1);
        assert_eq!(state.worker_count, 0);
        assert_eq!(state.prepared, 1);
        assert_eq!(state.retained_resource_bytes, 0);
        assert!(!state.backend_kind.is_null());
    }
}
