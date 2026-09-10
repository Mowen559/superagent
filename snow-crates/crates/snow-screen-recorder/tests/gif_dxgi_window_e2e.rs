#![cfg(target_os = "windows")]

use std::ffi::c_void;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ffmpeg_next as ffmpeg;
use snow_screen_recorder::{
    CaptureBackendKind, EditingSession, ExportFormat, RecordingAudioConfig,
    RecordingAudioTrackConfig, RecordingConfig, RecordingSession, RecordingTarget, WindowSelector,
};
use tempfile::tempdir;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetMessageW, KillTimer, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
    SetWindowLongPtrW, ShowWindow, TranslateMessage, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_NCCREATE,
    WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::PCWSTR;

const WINDOW_SIZE: i32 = 100;
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 80;
const COLORS: [COLORREF; 4] = [
    COLORREF(0x0000_00ff),
    COLORREF(0x0000_ff00),
    COLORREF(0x00ff_0000),
    COLORREF(0x0000_ffff),
];
const EXPECTED_RGB: [[u8; 3]; 4] = [
    [0xff, 0x00, 0x00],
    [0x00, 0xff, 0x00],
    [0x00, 0x00, 0xff],
    [0xff, 0xff, 0x00],
];
const COLOR_CHANNEL_TOLERANCE: u8 = 64;
const MIN_SOLID_COLOR_RATIO: f64 = 0.9;

struct ColorWindowState {
    color_index: usize,
}

struct ColorWindow {
    raw_handle: isize,
    thread: Option<JoinHandle<()>>,
}

impl ColorWindow {
    fn spawn() -> Self {
        let (handle_tx, handle_rx) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let instance = unsafe { GetModuleHandleW(None).expect("module handle should resolve") };
            let class_name: Vec<u16> =
                format!("SnowShotGifDxgiColorWindow{}\0", std::process::id())
                    .encode_utf16()
                    .collect();
            let class_name = PCWSTR(class_name.as_ptr());
            let class = WNDCLASSW {
                hInstance: HINSTANCE(instance.0),
                lpszClassName: class_name,
                lpfnWndProc: Some(color_window_proc),
                ..Default::default()
            };
            assert_ne!(unsafe { RegisterClassW(&class) }, 0);

            let state = Box::into_raw(Box::new(ColorWindowState { color_index: 0 }));
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    class_name,
                    class_name,
                    WS_POPUP | WS_VISIBLE,
                    64,
                    64,
                    WINDOW_SIZE,
                    WINDOW_SIZE,
                    None,
                    None,
                    Some(HINSTANCE(instance.0)),
                    Some(state.cast::<c_void>()),
                )
                .expect("color window should be created")
            };
            unsafe {
                let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
            }
            handle_tx.send(hwnd.0 as isize).unwrap();

            let mut message = MSG::default();
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.into() {
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        });

        let raw_handle = handle_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("color window should become ready");
        thread::sleep(Duration::from_millis(250));
        Self {
            raw_handle,
            thread: Some(thread),
        }
    }
}

impl Drop for ColorWindow {
    fn drop(&mut self) {
        let hwnd = HWND(self.raw_handle as *mut c_void);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("color window thread should exit");
        }
    }
}

extern "system" fn color_window_proc(
    hwnd: HWND,
    message: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => unsafe {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            LRESULT(1)
        },
        WM_CREATE => unsafe {
            SetTimer(Some(hwnd), TIMER_ID, TIMER_INTERVAL_MS, None);
            LRESULT(0)
        },
        WM_TIMER => unsafe {
            if let Some(state) = color_window_state(hwnd) {
                state.color_index = (state.color_index + 1) % COLORS.len();
            }
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        },
        WM_PAINT => unsafe {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let color = color_window_state(hwnd)
                .map(|state| COLORS[state.color_index])
                .unwrap_or(COLORS[0]);
            let brush = CreateSolidBrush(color);
            FillRect(dc, &paint.rcPaint, brush);
            let _ = DeleteObject(brush.into());
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        },
        WM_CLOSE => unsafe {
            DestroyWindow(hwnd).expect("color window should be destroyed");
            LRESULT(0)
        },
        WM_DESTROY => unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_ID);
            let state = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if state != 0 {
                drop(Box::from_raw(state as *mut ColorWindowState));
            }
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, message, WPARAM(0), lparam) },
    }
}

unsafe fn color_window_state(hwnd: HWND) -> Option<&'static mut ColorWindowState> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ColorWindowState;
    unsafe { state.as_mut() }
}

fn decode_video_rgba(path: &std::path::Path) -> Vec<Vec<u8>> {
    ffmpeg::init().expect("FFmpeg should initialize");
    let mut input = ffmpeg::format::input(path).expect("recorded video should open");
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .expect("recording should have a video stream");
    let stream_index = stream.index();
    let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .expect("video decoder context should be created");
    let mut decoder = context
        .decoder()
        .video()
        .expect("video decoder should open");
    let width = decoder.width();
    let height = decoder.height();
    assert_eq!((width, height), (WINDOW_SIZE as u32, WINDOW_SIZE as u32));
    let mut scaler = ffmpeg::software::scaling::Context::get(
        decoder.format(),
        width,
        height,
        ffmpeg::format::Pixel::RGBA,
        width,
        height,
        ffmpeg::software::scaling::flag::Flags::POINT,
    )
    .expect("RGBA scaler should be created");
    let mut decoded = ffmpeg::frame::Video::empty();
    let mut rgba = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::RGBA, width, height);
    let mut frames = Vec::new();
    let mut collect_frame = |decoded: &ffmpeg::frame::Video| {
        scaler
            .run(decoded, &mut rgba)
            .expect("video frame should convert to RGBA");
        let stride = rgba.stride(0);
        let row_bytes = width as usize * 4;
        let source = rgba.data(0);
        let mut pixels = Vec::with_capacity(row_bytes * height as usize);
        for row in 0..height as usize {
            let start = row * stride;
            pixels.extend_from_slice(&source[start..start + row_bytes]);
        }
        frames.push(pixels);
    };

    for (packet_stream, packet) in input.packets() {
        if packet_stream.index() != stream_index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .expect("video packet should decode");
        while decoder.receive_frame(&mut decoded).is_ok() {
            collect_frame(&decoded);
        }
    }
    decoder.send_eof().expect("video decoder should flush");
    while decoder.receive_frame(&mut decoded).is_ok() {
        collect_frame(&decoded);
    }
    frames
}

fn changed_pixel_ratio(left: &[u8], right: &[u8]) -> f64 {
    assert_eq!(left.len(), right.len());
    let changed = left
        .chunks_exact(4)
        .zip(right.chunks_exact(4))
        .filter(|(left, right)| {
            left[..3]
                .iter()
                .zip(&right[..3])
                .any(|(left, right)| left.abs_diff(*right) >= 32)
        })
        .count();
    changed as f64 / (left.len() / 4) as f64
}

fn strongest_expected_color_ratio(frame: &[u8]) -> f64 {
    EXPECTED_RGB
        .iter()
        .map(|expected| {
            frame
                .chunks_exact(4)
                .filter(|pixel| {
                    pixel[..3].iter().zip(expected).all(|(actual, expected)| {
                        actual.abs_diff(*expected) <= COLOR_CHANNEL_TOLERANCE
                    })
                })
                .count() as f64
                / (frame.len() / 4) as f64
        })
        .fold(0.0, f64::max)
}

fn assert_frames_are_complete_solid_presents(frames: &[Vec<u8>], label: &str) {
    let (worst_index, worst_ratio) = frames
        .iter()
        .enumerate()
        .map(|(index, frame)| (index, strongest_expected_color_ratio(frame)))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .expect("recording should contain at least one frame");
    assert!(
        worst_ratio >= MIN_SOLID_COLOR_RATIO,
        "{label} frame {worst_index} mixed pixels from different presents; only {worst_ratio:.3} of its pixels matched one expected solid color"
    );
}

#[test]
fn dxgi_window_recording_exports_visually_changing_gif_frames() {
    let temp = tempdir().expect("temporary output directory should be created");
    let window = ColorWindow::spawn();
    let mut audio_track = RecordingAudioTrackConfig::system_default("system");
    audio_track.enabled = false;
    let config = RecordingConfig {
        target: RecordingTarget::Window(WindowSelector::new(window.raw_handle)),
        capture_backend: CaptureBackendKind::DxgiDuplication,
        output_dir: temp.path().to_path_buf(),
        fps: 20,
        audio: RecordingAudioConfig {
            tracks: vec![audio_track],
            ..RecordingAudioConfig::default()
        },
        ..RecordingConfig::default()
    };

    let mut recording = RecordingSession::create(config).expect("recording should be created");
    recording
        .start()
        .expect("DXGI window recording should start");
    thread::sleep(Duration::from_secs(1));
    let artifact = recording.stop().expect("DXGI window recording should stop");

    let source_frames = decode_video_rgba(&artifact.bundle_path);
    let source_max_changed_ratio = source_frames
        .windows(2)
        .map(|pair| changed_pixel_ratio(&pair[0], &pair[1]))
        .fold(0.0, f64::max);
    assert!(
        source_frames.len() >= 2,
        "intermediate recording should decode to multiple frames"
    );
    assert_frames_are_complete_solid_presents(&source_frames, "intermediate recording");
    assert!(
        source_max_changed_ratio > 0.5,
        "intermediate recording frames should visibly change; maximum changed-pixel ratio was {source_max_changed_ratio:.3} across {} decoded frames",
        source_frames.len()
    );

    let editing = EditingSession::open(artifact).expect("recording should open for editing");
    let output_path = temp.path().join("dxgi-color-window.gif");
    let mut request = editing.export_request();
    request.format = ExportFormat::Gif;
    request.output_path = output_path.clone();
    editing.export(request).expect("GIF export should succeed");

    let frames = decode_video_rgba(&output_path);
    assert!(frames.len() >= 2, "GIF should decode to multiple frames");
    assert_frames_are_complete_solid_presents(&frames, "GIF");
    let max_changed_ratio = frames
        .windows(2)
        .map(|pair| changed_pixel_ratio(&pair[0], &pair[1]))
        .fold(0.0, f64::max);
    assert!(
        max_changed_ratio > 0.5,
        "GIF frames should visibly change; maximum changed-pixel ratio was {max_changed_ratio:.3} across {} decoded frames",
        frames.len()
    );
}
