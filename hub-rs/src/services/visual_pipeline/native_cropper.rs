use std::ffi::c_void;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection,
    DeleteDC, DeleteObject, EndPaint, InvalidateRect, ScreenToClient, SelectObject, StretchDIBits,
    UpdateWindow, GdiFlush, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, KillTimer, LoadCursorW, PostQuitMessage,
    RegisterClassW, SetCursor, SetForegroundWindow, SetTimer, ShowWindow,
    SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, IDC_ARROW, IDC_CROSS,
    MSG, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WM_TIMER,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use snow_capture::CaptureTarget;
use snow_ui_selector::{ElementRegionService, HitTestMode};

use crate::services::visual_pipeline::cropper::draw::{
    draw_border, draw_elements, draw_in_progress, draw_magnifier, draw_ocr_blocks,
    draw_recording_border, draw_translated_lines,
};
use crate::services::visual_pipeline::cropper::recording::{
    discard_crop_recording, start_crop_recording, stop_and_save_recording,
    toggle_pause_recording,
};
use crate::services::visual_pipeline::cropper::toolbar::{draw_toolbar, setup_toolbar};
use crate::services::visual_pipeline::cropper::types::{
    pt_in_rect, Action, AppMode, AppState, ArrowData, CroppedResult, DrawElement,
    EllipseData, MosaicData, OcrBox, RectData, StrokeData, TranslatedBox, PREDEFINED_COLORS,
};
use crate::services::visual_pipeline::VisualPipelineRuntime;

pub use crate::services::visual_pipeline::cropper::types::CroppedResult as NativeCropperResult;

use crate::services::visual_pipeline::cropper::pixels::{clip_rect, crop_rgba, paint_snapshot};

const TIMER_RECORDING_ID: usize = 1001;

static IS_CROPPER_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ACTIVE_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

struct CropperGuard;
impl Drop for CropperGuard {
    fn drop(&mut self) {
        ACTIVE_HWND.store(0, std::sync::atomic::Ordering::SeqCst);
        IS_CROPPER_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 鍚姩鍏ㄥ姛鑳藉師鐢熶氦浜掑紡鎴睆/褰曞睆绐楀彛
pub fn run_interactive_capture(
    vp: Arc<VisualPipelineRuntime>,
) -> std::result::Result<Option<(Vec<u8>, String, i32, i32, i32, i32)>, String> {
    if IS_CROPPER_ACTIVE.compare_exchange(false, true, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst).is_err() {
        tracing::warn!("Native cropper is already active, bringing existing window to front");
        let hwnd_raw = ACTIVE_HWND.load(std::sync::atomic::Ordering::SeqCst);
        if hwnd_raw != 0 {
            unsafe {
                let hwnd = HWND(hwnd_raw as *mut _);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        return Ok(None);
    }
    let _guard = CropperGuard;
    tracing::info!("[NativeCropper] Step 1: Starting interactive capture");

    // 鍚敤 Per-Monitor V2 DPI 鎰熺煡锛岀‘淇濈墿鐞嗗儚绱?1:1 绮剧‘鍖归厤
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    tracing::info!("[NativeCropper] Step 2: Calling snow_capture::capture_once");
    let target = CaptureTarget::PrimaryMonitor;
    let frame = snow_capture::capture_once(&target)
        .map_err(|e| {
            tracing::error!("[NativeCropper] snow_capture::capture_once failed: {e}");
            format!("Capture failed: {}", e)
        })?;

    let width = frame.width();
    let height = frame.height();
    tracing::info!("[NativeCropper] Step 3: Captured primary monitor frame: {}x{}", width, height);

    let rgba = frame.as_rgba_bytes();
    let mut bgra = vec![0u8; rgba.len()];
    let mut dimmed_bgra = vec![0u8; rgba.len()];
    for i in (0..rgba.len()).step_by(4) {
        bgra[i] = rgba[i + 2];     // B
        bgra[i + 1] = rgba[i + 1]; // G
        bgra[i + 2] = rgba[i];     // R
        bgra[i + 3] = rgba[i + 3]; // A

        dimmed_bgra[i] = bgra[i] / 2;
        dimmed_bgra[i + 1] = bgra[i + 1] / 2;
        dimmed_bgra[i + 2] = bgra[i + 2] / 2;
        dimmed_bgra[i + 3] = bgra[i + 3];
    }

    let (tx, rx) = channel();

    tracing::info!("[NativeCropper] Step 4: Initializing UI selector service");
    let ui_service = ElementRegionService::new().ok();

    tracing::info!("[NativeCropper] Step 5: Preparing AppState and toolbar");
    let mut state = Box::new(AppState {
        frame_width: width,
        frame_height: height,
        bgra_pixels: bgra,
        dimmed_bgra_pixels: dimmed_bgra,
        mode: AppMode::Selecting,
        is_dragging: false,
        start_pt: POINT { x: 0, y: 0 },
        current_pt: POINT { x: 0, y: 0 },
        crop_rect: None,
        hovered_rect: None,
        elements: Vec::new(),
        redo_stack: Vec::new(),
        current_stroke: None,
        current_color: PREDEFINED_COLORS[6],
        current_thickness: 2,
        is_eraser: false,
        buttons: Vec::new(),
        color_buttons: Vec::new(),
        sub_tool_buttons: Vec::new(),
        record_buttons: Vec::new(),
        hovered_action: None,
        result_tx: Some(tx),
        ui_service,
        vp: Some(vp.clone()),
        is_selecting_text: false,
        text_selection_start: POINT { x: 0, y: 0 },
        text_selection_current: POINT { x: 0, y: 0 },
        ocr_blocks: None,
        translated_lines: None,
        loading_text: String::new(),
        recording_session: None,
        active_text_input: None,
    });

    setup_toolbar(&mut state);
    let state_ptr = Box::into_raw(state);

    tracing::info!("[NativeCropper] Step 6: Creating Win32 overlay window");
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class_name = w!("SnowShotCropper");

        let wc = WNDCLASSW {
            style: windows::Win32::UI::WindowsAndMessaging::CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap(),
            hbrBackground: windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x001A1A1A)),
            ..Default::default()
        };

        let reg_res = RegisterClassW(&wc);
        tracing::info!("[NativeCropper] RegisterClassW atom: {}", reg_res);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("SnowShot"),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            width as i32,
            height as i32,
            None,
            None,
            Some(instance.into()),
            Some(state_ptr as *const _),
        ).map_err(|e| {
            let _ = Box::from_raw(state_ptr);
            tracing::error!("[NativeCropper] CreateWindowExW failed: {e}");
            format!("CreateWindowExW failed: {e}")
        })?;

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        ACTIVE_HWND.store(hwnd.0 as isize, std::sync::atomic::Ordering::SeqCst);

        // Force it to the top
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
            hwnd, 
            Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST), 
            0, 0, width as i32, height as i32, 
            windows::Win32::UI::WindowsAndMessaging::SWP_SHOWWINDOW | windows::Win32::UI::WindowsAndMessaging::SWP_FRAMECHANGED
        );

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = SetCursor(Some(LoadCursorW(None, IDC_CROSS).unwrap()));
        let _ = InvalidateRect(Some(hwnd), None, false);

        tracing::info!("[NativeCropper] Window successfully shown: HWND 0x{:X}", hwnd.0 as usize);
        hwnd
    };

    tracing::info!("[NativeCropper] Step 7: Starting Win32 message loop");
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    tracing::info!("[NativeCropper] Step 8: Exited Win32 message loop");

    if let Ok(Some(result)) = rx.recv() {
        let crop_w = result.width as usize;
        let crop_h = result.height as usize;
        let final_rgba = result.final_pixels;

        if result.action == Action::Accept || result.action == Action::Copy {
            unsafe {
                if OpenClipboard(None).is_ok() {
                    let _ = EmptyClipboard();
                    let info_size = std::mem::size_of::<BITMAPINFOHEADER>();
                    let data_size = crop_w * crop_h * 4;

                    if let Ok(hmem) = GlobalAlloc(GHND, info_size + data_size) {
                        let ptr = GlobalLock(hmem) as *mut u8;
                        let header = ptr as *mut BITMAPINFOHEADER;
                        (*header).biSize = info_size as u32;
                        (*header).biWidth = crop_w as i32;
                        (*header).biHeight = crop_h as i32;
                        (*header).biPlanes = 1;
                        (*header).biBitCount = 32;
                        (*header).biCompression = BI_RGB.0;

                        let dst_data = ptr.add(info_size);
                        let stride = crop_w * 4;
                        for y in 0..crop_h {
                            let src_offset = (crop_h - 1 - y) * stride;
                            let dst_offset = y * stride;
                            for x in 0..crop_w {
                                let r = final_rgba[src_offset + x * 4];
                                let g = final_rgba[src_offset + x * 4 + 1];
                                let b = final_rgba[src_offset + x * 4 + 2];
                                let a = final_rgba[src_offset + x * 4 + 3];

                                *dst_data.add(dst_offset + x * 4) = b;
                                *dst_data.add(dst_offset + x * 4 + 1) = g;
                                *dst_data.add(dst_offset + x * 4 + 2) = r;
                                *dst_data.add(dst_offset + x * 4 + 3) = a;
                            }
                        }
                        let _ = GlobalUnlock(hmem);
                        let _ = SetClipboardData(8 /* CF_DIB */, Some(windows::Win32::Foundation::HANDLE(hmem.0 as _)));
                    }
                    let _ = CloseClipboard();
                }
            }
        }

        if result.action == Action::Pin {
            crate::services::visual_pipeline::pin_window::spawn_pin_window(
                result.x,
                result.y,
                crop_w as u32,
                crop_h as u32,
                final_rgba.clone(),
            );
        }

        let img = image::RgbaImage::from_raw(crop_w as u32, crop_h as u32, final_rgba.clone())
            .ok_or_else(|| "Failed to construct RgbaImage".to_string())?;
        let mut bytes: Vec<u8> = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut bytes);
        img.write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| format!("Failed to encode PNG: {}", e))?;

        let action_str = match result.action {
            Action::Ocr => "ocr",
            Action::Translate => "translate",
            Action::Pin => "pin",
            Action::Copy => "copy",
            Action::Save => "save",
            Action::Record => "record",
            _ => "accept",
        }.to_string();

        Ok(Some((bytes, action_str, result.x, result.y, crop_w as i32, crop_h as i32)))
    } else {
        Ok(None)
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => unsafe {
            let create = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
            if !create.lpCreateParams.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            LRESULT(1)
        },
        WM_SETCURSOR => unsafe {
            if let Some(state) = state_mut(hwnd) {
                if state.mode == AppMode::ToolbarActive || state.mode == AppMode::Recording {
                    let _ = SetCursor(Some(LoadCursorW(None, IDC_ARROW).unwrap()));
                    return LRESULT(1);
                }
            }
            LRESULT(1)
        },
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => unsafe {
            if wparam.0 == TIMER_RECORDING_ID {
                if let Some(state) = state_mut(hwnd) {
                    if let Some(session) = &mut state.recording_session {
                        if !session.is_paused {
                            session.elapsed_secs = session.start_time.elapsed().as_secs();
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                }
            }
            LRESULT(0)
        },
        WM_KEYDOWN => unsafe {
            let key = wparam.0;
            if key == VK_ESCAPE.0 as usize {
                if let Some(state) = state_mut(hwnd) {
                    if state.mode == AppMode::Recording {
                        let _ = discard_crop_recording(state);
                    }
                    if let Some(tx) = state.result_tx.take() {
                        let _ = tx.send(None);
                    }
                }
                let _ = DestroyWindow(hwnd);
            } else if key == VK_RETURN.0 as usize {
                // 鍥炶溅蹇嵎纭鎴浘
                handle_action(hwnd, Action::Accept);
            } else if key == 'Z' as usize && (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0 {
                if let Some(state) = state_mut(hwnd) {
                    if let Some(el) = state.elements.pop() {
                        state.redo_stack.push(el);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            } else if key == 'Y' as usize && (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0 {
                if let Some(state) = state_mut(hwnd) {
                    if let Some(el) = state.redo_stack.pop() {
                        state.elements.push(el);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        },
        WM_LBUTTONDOWN => unsafe {
            if let Some(state) = state_mut(hwnd) {
                let mut pt_screen = POINT::default();
                let _ = GetCursorPos(&mut pt_screen);
                let mut pt = pt_screen;
                let _ = ScreenToClient(hwnd, &mut pt);

                // 1. 褰曞睆鎺у埗鏉＄偣鍑诲垽瀹
                if state.mode == AppMode::Recording {
                    for btn in &state.record_buttons {
                        if pt_in_rect(&btn.rect, pt) {
                            handle_action(hwnd, btn.action);
                            return LRESULT(0);
                        }
                    }
                    return LRESULT(0);
                }

                // 2. 涓诲伐鍏锋爮涓庝簩绾ц彍鍗曠偣鍑诲垽瀹
                if state.mode == AppMode::ToolbarActive || is_annotation_mode(state.mode) {
                    let mut clicked = false;

                    // 璋冭壊鐩
                    for btn in &state.color_buttons {
                        if pt_in_rect(&btn.rect, pt) {
                            clicked = true;
                            if let Action::ColorPick(c) = btn.action {
                                state.current_color = c;
                                state.is_eraser = false;
                                setup_toolbar(state);
                                let _ = InvalidateRect(Some(hwnd), None, false);
                            }
                            break;
                        }
                    }

                    // 绮楃粏
                    if !clicked {
                        for btn in &state.sub_tool_buttons {
                            if pt_in_rect(&btn.rect, pt) {
                                clicked = true;
                                if let Action::Thickness(t) = btn.action {
                                    state.current_thickness = t;
                                    setup_toolbar(state);
                                    let _ = InvalidateRect(Some(hwnd), None, false);
                                }
                                break;
                            }
                        }
                    }

                    // 涓诲姛鑳芥寜鎵
                    if !clicked {
                        for btn in &state.buttons {
                            if pt_in_rect(&btn.rect, pt) {
                                clicked = true;
                                handle_action(hwnd, btn.action);
                                break;
                            }
                        }
                    }

                    if clicked {
                        return LRESULT(0);
                    }
                }

                // 3. 鐢诲竷浜や簰锛堟爣娉ㄣ€佹秱楦︽垨鍒掗€夛級
                if is_annotation_mode(state.mode) {
                    state.is_dragging = true;
                    state.start_pt = pt;
                    state.current_pt = pt;
                    if state.mode == AppMode::DrawingPen {
                        state.current_stroke = Some(vec![pt]);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if state.mode == AppMode::SelectingText {
                    state.is_selecting_text = true;
                    state.text_selection_start = pt;
                    state.text_selection_current = pt;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else {
                    // 寮€濮嬫嫋閫夋埅灞忛€夊尯
                    state.is_dragging = true;
                    state.start_pt = pt;
                    state.current_pt = pt;
                    state.mode = AppMode::Selecting;
                    if let Some(rect_screen) = state.hovered_rect {
                        let mut top_left = POINT { x: rect_screen.left, y: rect_screen.top };
                        let mut bottom_right = POINT { x: rect_screen.right, y: rect_screen.bottom };
                        let _ = ScreenToClient(hwnd, &mut top_left);
                        let _ = ScreenToClient(hwnd, &mut bottom_right);
                        state.crop_rect = Some(RECT { left: top_left.x, top: top_left.y, right: bottom_right.x, bottom: bottom_right.y });
                    } else {
                        state.crop_rect = None;
                    }
                }
            }
            LRESULT(0)
        },
        WM_MOUSEMOVE => unsafe {
            if let Some(state) = state_mut(hwnd) {
                let mut pt_screen = POINT::default();
                let _ = GetCursorPos(&mut pt_screen);
                let mut pt = pt_screen;
                let _ = ScreenToClient(hwnd, &mut pt);

                if state.mode == AppMode::DrawingPen && state.is_dragging {
                    if let Some(stroke) = &mut state.current_stroke {
                        if let Some(last_pt) = stroke.last() {
                            let dx = pt.x - last_pt.x;
                            let dy = pt.y - last_pt.y;
                            if dx * dx + dy * dy > 9 {
                                stroke.push(pt);
                                let _ = InvalidateRect(Some(hwnd), None, false);
                            }
                        }
                    }
                } else if state.is_dragging {
                    state.current_pt = pt;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if state.is_selecting_text {
                    state.text_selection_current = pt;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if state.mode == AppMode::ToolbarActive || is_annotation_mode(state.mode) {
                    // 宸ュ叿鏍忔偓鍋滄彁绀
                    let mut hovered = None;
                    for btn in &state.buttons {
                        if pt_in_rect(&btn.rect, pt) {
                            hovered = Some(btn.action);
                            break;
                        }
                    }
                    if state.hovered_action != hovered {
                        state.hovered_action = hovered;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                } else if state.mode == AppMode::Selecting {
                    state.current_pt = pt;
                    // 鏅鸿兘 UI 鍏冪礌鍚搁檮
                    if let Some(svc) = &mut state.ui_service {
                        if let Ok(Some(rects)) = svc.hit_test_point(pt_screen, HitTestMode::Window) {
                            if let Some(r) = rects.first() {
                                let new_rect = RECT { left: r.left(), top: r.top(), right: r.right(), bottom: r.bottom() };
                                if state.hovered_rect.as_ref() != Some(&new_rect) {
                                    state.hovered_rect = Some(new_rect);
                                }
                            }
                        }
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        },
        WM_LBUTTONUP => unsafe {
            if let Some(state) = state_mut(hwnd) {
                if state.mode == AppMode::DrawingPen {
                    if let Some(stroke) = state.current_stroke.take() {
                        state.elements.push(DrawElement::Stroke(StrokeData {
                            points: stroke,
                            color: state.current_color,
                            thickness: state.current_thickness,
                            is_eraser: state.is_eraser,
                        }));
                        state.redo_stack.clear();
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    state.is_dragging = false;
                } else if is_annotation_mode(state.mode) && state.is_dragging {
                    state.is_dragging = false;
                    let left = std::cmp::min(state.start_pt.x, state.current_pt.x);
                    let top = std::cmp::min(state.start_pt.y, state.current_pt.y);
                    let right = std::cmp::max(state.start_pt.x, state.current_pt.x);
                    let bottom = std::cmp::max(state.start_pt.y, state.current_pt.y);

                    match state.mode {
                        AppMode::DrawingRect => {
                            state.elements.push(DrawElement::Rect(RectData {
                                rect: RECT { left, top, right, bottom },
                                color: state.current_color,
                                thickness: state.current_thickness,
                            }));
                        }
                        AppMode::DrawingArrow => {
                            state.elements.push(DrawElement::Arrow(ArrowData {
                                start: state.start_pt,
                                end: state.current_pt,
                                color: state.current_color,
                                thickness: state.current_thickness,
                            }));
                        }
                        AppMode::DrawingEllipse => {
                            state.elements.push(DrawElement::Ellipse(EllipseData {
                                rect: RECT { left, top, right, bottom },
                                color: state.current_color,
                                thickness: state.current_thickness,
                            }));
                        }
                        AppMode::DrawingMosaic => {
                            state.elements.push(DrawElement::Mosaic(MosaicData {
                                rect: RECT { left, top, right, bottom },
                                block_size: 12,
                            }));
                        }
                        _ => {}
                    }
                    state.redo_stack.clear();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if state.is_selecting_text {
                    state.is_selecting_text = false;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if state.is_dragging {
                    state.is_dragging = false;
                    let left = std::cmp::min(state.start_pt.x, state.current_pt.x);
                    let top = std::cmp::min(state.start_pt.y, state.current_pt.y);
                    let right = std::cmp::max(state.start_pt.x, state.current_pt.x);
                    let bottom = std::cmp::max(state.start_pt.y, state.current_pt.y);

                    let width = right - left;
                    let height = bottom - top;

                    if width > 5 || height > 5 {
                        state.crop_rect = Some(RECT { left, top, right, bottom });
                        state.mode = AppMode::ToolbarActive;
                        setup_toolbar(state);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    } else if let Some(rect) = state.hovered_rect {
                        let mut top_left = POINT { x: rect.left, y: rect.top };
                        let mut bottom_right = POINT { x: rect.right, y: rect.bottom };
                        let _ = ScreenToClient(hwnd, &mut top_left);
                        let _ = ScreenToClient(hwnd, &mut bottom_right);
                        state.crop_rect = Some(RECT { left: top_left.x, top: top_left.y,
                            right: bottom_right.x, bottom: bottom_right.y });
                        state.mode = AppMode::ToolbarActive;
                        setup_toolbar(state);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    } else {
                        state.crop_rect = None;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        },
        WM_LBUTTONDBLCLK => unsafe {
            if let Some(state) = state_mut(hwnd) {
                if let Some(r) = state.crop_rect {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(hwnd, &mut pt);
                    if pt_in_rect(&r, pt) {
                        handle_action(hwnd, Action::Accept);
                        return LRESULT(0);
                    }
                }
            }
            LRESULT(0)
        },
        WM_RBUTTONDOWN => unsafe {
            if let Some(state) = state_mut(hwnd) {
                if is_annotation_mode(state.mode) {
                    state.mode = AppMode::ToolbarActive;
                    setup_toolbar(state);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                } else if state.crop_rect.is_some() {
                    state.crop_rect = None;
                    state.mode = AppMode::Selecting;
                    setup_toolbar(state);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                } else {
                    if let Some(tx) = state.result_tx.take() {
                        let _ = tx.send(None);
                    }
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
            }
            LRESULT(0)
        },
        WM_PAINT => unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            if let Some(state) = state_mut(hwnd) {
                let hdc_mem = CreateCompatibleDC(Some(hdc));
                
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: state.frame_width as i32,
                        biHeight: -(state.frame_height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let mut pbits: *mut std::ffi::c_void = std::ptr::null_mut();
                let hbm_mem = CreateDIBSection(
                    Some(hdc),
                    &bmi as *const BITMAPINFO,
                    DIB_RGB_COLORS,
                    &mut pbits,
                    None,
                    0,
                ).unwrap_or_default();
                
                let hbm_old = SelectObject(hdc_mem, hbm_mem.into());

                // Freeze the same pixel positions throughout the selection.
                let active_rect = if state.mode == AppMode::Selecting && state.is_dragging {
                    let left = std::cmp::min(state.start_pt.x, state.current_pt.x);
                    let top = std::cmp::min(state.start_pt.y, state.current_pt.y);
                    let right = std::cmp::max(state.start_pt.x, state.current_pt.x);
                    let bottom = std::cmp::max(state.start_pt.y, state.current_pt.y);
                    Some(RECT { left, top, right, bottom })
                } else if state.mode == AppMode::Selecting && !state.is_dragging {
                    if let Some(r) = state.hovered_rect {
                        let mut top_left = POINT { x: r.left, y: r.top };
                        let mut bottom_right = POINT { x: r.right, y: r.bottom };
                        let _ = ScreenToClient(hwnd, &mut top_left);
                        let _ = ScreenToClient(hwnd, &mut bottom_right);
                        Some(RECT { left: top_left.x, top: top_left.y, right: bottom_right.x, bottom: bottom_right.y })
                    } else {
                        None
                    }
                } else {
                    state.crop_rect
                };

                if !pbits.is_null() {
                    let target = std::slice::from_raw_parts_mut(
                        pbits as *mut u8, state.bgra_pixels.len(),
                    );
                    let selection = active_rect.map(|r| [r.left, r.top, r.right, r.bottom]);
                    let _ = paint_snapshot(target, &state.bgra_pixels, &state.dimmed_bgra_pixels,
                        state.frame_width, state.frame_height, selection);
                }

                // 4. 閫夊尯杈规缁樺埗
                if state.mode == AppMode::Recording {
                    if let Some(r) = state.crop_rect {
                        let is_blink = (state.recording_session.as_ref().map(|s| s.elapsed_secs).unwrap_or(0) % 2) == 0;
                        draw_recording_border(hdc_mem, r, is_blink);
                    }
                } else if let Some(r) = state.crop_rect {
                    draw_border(hdc_mem, r, COLORREF(0x00D9531E), 2);
                } else if state.mode == AppMode::Selecting && state.is_dragging {
                    if let Some(r) = active_rect {
                        draw_border(hdc_mem, r, COLORREF(0x00FF0000), 2);
                    }
                }

                // 5. 缁樺埗鎵€鏈夊凡鐢绘爣娉
                draw_elements(hdc_mem, state, &bmi);

                // 6. 缁樺埗涓存椂鎷栨嫿鍥惧舰
                draw_in_progress(hdc_mem, state);

                // 7. 缁樺埗 OCR 鏂囧瓧鍧椾笌鍒掗€
                draw_ocr_blocks(hdc_mem, state);

                // 8. 缁樺埗澶氳瑷€灏卞湴缈昏瘧瑕嗙洊
                draw_translated_lines(hdc_mem, state);

                // 9. 鏀惧ぇ闀滐紙浠呭湪閫夋嫨闃舵涓旀湭纭畾鐭╁舰鏃跺睍绀猴級
                if state.mode == AppMode::Selecting && state.crop_rect.is_none() {
                    draw_magnifier(hdc_mem, state.current_pt, state);
                }

                // 10. 缁樺埗宸ュ叿鏍忎笌褰曞埗鎺у埗鏉
                draw_toolbar(hdc_mem, state);

                // 鍙岀紦鍐叉嫹璐濊嚦灞忓箷
                let _ = BitBlt(hdc, 0, 0, state.frame_width as i32, state.frame_height as i32, Some(hdc_mem), 0, 0, SRCCOPY);
                SelectObject(hdc_mem, hbm_old);
                let _ = DeleteObject(hbm_mem.into());
                let _ = DeleteDC(hdc_mem);
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        },
        WM_DESTROY => unsafe {
            ACTIVE_HWND.store(0, std::sync::atomic::Ordering::SeqCst);
            let _ = KillTimer(Some(hwnd), TIMER_RECORDING_ID);
            if let Some(state) = state_mut(hwnd) {
                if let Some(tx) = state.result_tx.take() {
                    let _ = tx.send(None);
                }
            }
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if ptr != 0 {
                drop(Box::from_raw(ptr as *mut AppState));
            }
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 鎵ц鎸囧畾 Action
unsafe fn handle_action(hwnd: HWND, action: Action) {
    let Some(state) = state_mut(hwnd) else { return; };

    match action {
        Action::DrawPen => {
            state.mode = AppMode::DrawingPen;
            state.is_eraser = false;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::DrawRect => {
            state.mode = AppMode::DrawingRect;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::DrawArrow => {
            state.mode = AppMode::DrawingArrow;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::DrawEllipse => {
            state.mode = AppMode::DrawingEllipse;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::DrawText => {
            state.mode = AppMode::DrawingText;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::Mosaic => {
            state.mode = AppMode::DrawingMosaic;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::Eraser => {
            state.mode = AppMode::DrawingPen;
            state.is_eraser = true;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::Undo => {
            if let Some(el) = state.elements.pop() {
                state.redo_stack.push(el);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        Action::Redo => {
            if let Some(el) = state.redo_stack.pop() {
                state.elements.push(el);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        Action::Record => {
            // 鍚姩褰曞睆
            if let Ok(()) = start_crop_recording(state) {
                let _ = SetTimer(Some(hwnd), TIMER_RECORDING_ID, 500, None);
                setup_toolbar(state);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        Action::RecordPause | Action::RecordResume => {
            let _ = toggle_pause_recording(state);
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::RecordStop => {
            let _ = KillTimer(Some(hwnd), TIMER_RECORDING_ID);
            let _ = stop_and_save_recording(state);
            let _ = DestroyWindow(hwnd);
        }
        Action::RecordDiscard => {
            let _ = KillTimer(Some(hwnd), TIMER_RECORDING_ID);
            let _ = discard_crop_recording(state);
            let _ = DestroyWindow(hwnd);
        }
        Action::Ocr => {
            // 瑙﹀彂 OCR 妫€娴
            state.loading_text = "姝ｅ湪璇嗗埆鏂囧瓧...".to_string();
            let _ = InvalidateRect(Some(hwnd), None, false);
            
            // 鏋勯€犵ず渚?OCR 缁撴灉浠ヤ究鍗虫椂浜や簰锛堜害鍙紓姝ヨ姹傚ぇ妯″瀷锛
            if let Some(r) = state.crop_rect {
                state.ocr_blocks = Some(vec![
                    OcrBox {
                        rect: RECT { left: r.left + 10, top: r.top + 10, right: r.right - 10, bottom: r.top + 40 },
                        text: "DeepCloud Visual OCR".to_string(),
                    }
                ]);
            }
            state.loading_text.clear();
            state.mode = AppMode::SelectingText;
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::Translate => {
            // 瑙﹀彂灏卞湴缈昏瘧
            state.loading_text = "姝ｅ湪澶氳瑷€缈昏瘧...".to_string();
            let _ = InvalidateRect(Some(hwnd), None, false);

            if let Some(r) = state.crop_rect {
                state.translated_lines = Some(vec![
                    TranslatedBox {
                        rect: RECT { left: r.left + 10, top: r.top + 10, right: r.right - 10, bottom: r.top + 40 },
                        original_text: "DeepCloud Visual OCR".to_string(),
                        translated_text: "DeepCloud 视觉识别与翻译".to_string(),
                    }
                ]);
            }
            state.loading_text.clear();
            setup_toolbar(state);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        Action::Cancel => {
            if state.mode == AppMode::Recording {
                let _ = discard_crop_recording(state);
            }
            let _ = DestroyWindow(hwnd);
        }
        Action::Accept | Action::Copy | Action::Pin | Action::Save => {
            finish_capture(hwnd, action);
        }
        _ => {}
    }
}

/// Bake annotations into the frozen image and export the selected region.
unsafe fn finish_capture(hwnd: HWND, action: Action) {
    let Some(state) = state_mut(hwnd) else { return; };
    let Some(rect) = state.crop_rect else { return; };
    let Some([left, top, right, bottom]) = clip_rect(
        [rect.left, rect.top, rect.right, rect.bottom], state.frame_width, state.frame_height,
    ) else { return; };
    let rect = RECT { left: left as i32, top: top as i32, right: right as i32, bottom: bottom as i32 };

    let hdc_mem = CreateCompatibleDC(None);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: state.frame_width as i32,
            biHeight: -(state.frame_height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pbits: *mut c_void = std::ptr::null_mut();
    let Ok(hbm) = CreateDIBSection(Some(hdc_mem), &bmi, DIB_RGB_COLORS, &mut pbits, None, 0) else {
        let _ = DeleteDC(hdc_mem);
        return;
    };
    let old_bitmap = SelectObject(hdc_mem, hbm.into());

    std::ptr::copy_nonoverlapping(state.bgra_pixels.as_ptr(), pbits as *mut u8, state.bgra_pixels.len());

    // 鐑樼剻鏍囨敞涓庣炕璇
    draw_elements(hdc_mem, state, &bmi);
    draw_translated_lines(hdc_mem, state);

    let _ = GdiFlush();
    let baked_pixels = std::slice::from_raw_parts(pbits as *const u8, state.bgra_pixels.len());
    let final_pixels = crop_rgba(baked_pixels, state.frame_width, state.frame_height,
        [rect.left, rect.top, rect.right, rect.bottom]);
    SelectObject(hdc_mem, old_bitmap);
    let _ = DeleteObject(hbm.into());
    let _ = DeleteDC(hdc_mem);
    let Ok(final_pixels) = final_pixels else { return; };
    let Some(tx) = state.result_tx.take() else { return; };
    let crop_w = rect.right - rect.left;
    let crop_h = rect.bottom - rect.top;

    let _ = tx.send(Some(CroppedResult {
        x: rect.left,
        y: rect.top,
        width: crop_w,
        height: crop_h,
        action,
        final_pixels,
    }));

    let _ = DestroyWindow(hwnd);
}

fn is_annotation_mode(mode: AppMode) -> bool {
    matches!(
        mode,
        AppMode::DrawingPen
            | AppMode::DrawingRect
            | AppMode::DrawingArrow
            | AppMode::DrawingEllipse
            | AppMode::DrawingText
            | AppMode::DrawingMosaic
    )
}

unsafe fn state_mut(hwnd: HWND) -> Option<&'static mut AppState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}
