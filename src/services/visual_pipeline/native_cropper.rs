use std::ffi::c_void;
use std::sync::mpsc::channel;
use std::sync::Arc;
use windows::core::w;
use windows::Win32::Foundation::{
    COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DeleteDC, DeleteObject, EndPaint, InvalidateRect, ScreenToClient, SelectObject, SaveDC, RestoreDC, IntersectClipRect,
    UpdateWindow, GdiFlush, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SetFocus, SetCapture, ReleaseCapture, VK_CONTROL, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONINFORMATION, WM_USER, 
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, PostQuitMessage,
    RegisterClassW, SetCursor, SetForegroundWindow, SetTimer, ShowWindow,
    SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, IDC_ARROW, IDC_CROSS, IDC_SIZENWSE, IDC_SIZENESW, IDC_SIZENS, IDC_SIZEWE, IDC_SIZEALL,
    MSG, SW_SHOW, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WM_TIMER,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use snow_capture::CaptureTarget;
use snow_ui_selector::{ElementRegionService, HitTestMode};

use crate::services::visual_pipeline::cropper::draw::{
    draw_border, draw_elements, draw_in_progress, draw_magnifier, draw_ocr_blocks,
    draw_recording_border, draw_translated_lines,
};
use crate::services::visual_pipeline::cropper::toolbar::{draw_toolbar, setup_toolbar};
use crate::services::visual_pipeline::cropper::types::{
    pt_in_rect, hit_test_resize, DragMode, Action, AppMode, AppState, ArrowData, CroppedResult, DrawElement,
    EllipseData, MosaicData, RectData, StrokeData, PREDEFINED_COLORS,
};
use crate::services::visual_pipeline::VisualPipelineRuntime;

pub use crate::services::visual_pipeline::cropper::types::CroppedResult as NativeCropperResult;

use crate::services::visual_pipeline::cropper::pixels::{clip_rect, crop_rgba, paint_snapshot};

use crate::services::visual_pipeline::cropper::{actions, text_selection};

const TIMER_RECORDING_ID: usize = 1001;
const WM_OCR_DONE: u32 = WM_USER + 100;
const WM_TRANSLATE_DONE: u32 = WM_USER + 101;
const WM_USER_102: u32 = WM_USER + 102;

static IS_CROPPER_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ACTIVE_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

struct CropperGuard;
impl Drop for CropperGuard {
    fn drop(&mut self) {
        ACTIVE_HWND.store(0, std::sync::atomic::Ordering::SeqCst);
        IS_CROPPER_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

// Native capture interaction.
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

    // Native capture interaction.
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
    let default_audio = vp.settings().default_record_audio.clone();
    let audio_mode = match default_audio.as_str() {
        "mic" => crate::services::visual_pipeline::cropper::types::RecordAudioMode::Mic,
        "both" => crate::services::visual_pipeline::cropper::types::RecordAudioMode::Both,
        "none" => crate::services::visual_pipeline::cropper::types::RecordAudioMode::Mute,
        _ => crate::services::visual_pipeline::cropper::types::RecordAudioMode::System,
    };

    let mut state = Box::new(AppState {
        record_audio_mode: audio_mode,

        analysis_job: None, record_job: None, text_lines: Vec::new(), selected_spans: Vec::new(),
        status_text: String::new(), last_saved_path: None, input_window: None,
        frame_width: width,
        frame_height: height,
        bgra_pixels: bgra,
        dimmed_bgra_pixels: dimmed_bgra,
        mode: AppMode::Selecting,
        is_dragging: false,
        drag_mode: None,
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
        let _ = SetTimer(Some(hwnd), TIMER_RECORDING_ID, 100, None);
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
            let mut cursor = IDC_ARROW;
            if let Some(state) = state_mut(hwnd) {
                if state.mode == AppMode::Selecting {
                    cursor = IDC_CROSS;
                } else if matches!(state.mode, AppMode::ToolbarActive | AppMode::Saved) {
                    if let Some(drag) = state.drag_mode {
                        cursor = match drag {
                            DragMode::Move => IDC_SIZEALL,
                            DragMode::ResizeLeft | DragMode::ResizeRight => IDC_SIZEWE,
                            DragMode::ResizeTop | DragMode::ResizeBottom => IDC_SIZENS,
                            DragMode::ResizeTopLeft | DragMode::ResizeBottomRight => IDC_SIZENWSE,
                            DragMode::ResizeTopRight | DragMode::ResizeBottomLeft => IDC_SIZENESW,
                        };
                    }
                }
            }
            let _ = SetCursor(LoadCursorW(None, cursor).ok());
            LRESULT(1)
        },
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => unsafe { handle_action(hwnd, Action::Cancel); LRESULT(0) },
        WM_TIMER => unsafe {
            if let Some(state) = state_mut(hwnd) {
                actions::poll(hwnd, state);
                if let Some(session) = &mut state.recording_session {
                    if !session.is_paused { session.elapsed_secs = session.start_time.elapsed().as_secs(); }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        },
        WM_KEYDOWN => unsafe {
            let key = wparam.0;
            let control = (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
            if let Some(state) = state_mut(hwnd) {
                if control && key == 'A' as usize && state.mode == AppMode::SelectingText {
                    state.selected_spans = text_selection::selection(&state.text_lines, [0,0], [0,0], true);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if control && key == 'C' as usize && state.mode == AppMode::SelectingText {
                    actions::copy_selection(hwnd, state);
                } else if control && key == 'S' as usize {
                    handle_action(hwnd, if state.mode == AppMode::Recording { Action::RecordStop } else { Action::Save });
                } else if control && key == 'Z' as usize { handle_action(hwnd, Action::Undo); }
                else if control && key == 'Y' as usize { handle_action(hwnd, Action::Redo); }
                else if key == VK_ESCAPE.0 as usize { handle_action(hwnd, Action::Cancel); }
                else if key == VK_RETURN.0 as usize && !actions::busy(state) { handle_action(hwnd, Action::Accept); }
            }
            LRESULT(0)
        },
        WM_LBUTTONDOWN => unsafe {
            if let Some(state) = state_mut(hwnd) {
                let point = client_cursor(hwnd);
                let action = state.record_buttons.iter().chain(state.buttons.iter()).chain(state.color_buttons.iter())
                    .chain(state.sub_tool_buttons.iter()).find(|b| pt_in_rect(&b.rect, point)).map(|b| b.action);
                if let Some(action) = action { handle_action(hwnd, action); return LRESULT(0); }
                if actions::busy(state) || matches!(state.mode, AppMode::Recording | AppMode::Saved) { return LRESULT(0); }
                actions::commit_text(state);
                let _ = SetFocus(Some(hwnd));
                if state.mode == AppMode::Selecting {
                    state.start_pt = point; state.current_pt = point; state.is_dragging = true;
                    SetCapture(hwnd);
                } else if state.mode == AppMode::ToolbarActive && state.drag_mode.is_some() {
                    state.start_pt = point; state.current_pt = point; state.is_dragging = true;
                    SetCapture(hwnd);
                } else if state.crop_rect.is_some_and(|r| pt_in_rect(&r, point)) {
                    if state.mode == AppMode::SelectingText {
                        state.is_selecting_text = true;
                        state.selected_spans.clear();
                        state.text_selection_start = point;
                        state.text_selection_current = point;
                        SetCapture(hwnd);
                    } else if state.mode == AppMode::DrawingText {
                        actions::start_text(hwnd, state, point);
                    } else if is_annotation_mode(state.mode) {
                        state.start_pt=point;state.current_pt=point;state.is_dragging=true;
                        if state.mode == AppMode::DrawingPen { state.current_stroke = Some(vec![point]); }
                        SetCapture(hwnd);
                    }
                }
                let _=InvalidateRect(Some(hwnd),None,false);
            }
            LRESULT(0)
        },
        WM_MOUSEMOVE => unsafe {
            if let Some(state)=state_mut(hwnd) {
                let point=client_cursor(hwnd);
                state.hovered_action=state.record_buttons.iter().chain(state.buttons.iter()).chain(state.color_buttons.iter())
                    .chain(state.sub_tool_buttons.iter()).find(|b| pt_in_rect(&b.rect,point)).map(|b| b.action);
                if state.is_selecting_text {
                    state.text_selection_current=point;
                    state.selected_spans=text_selection::selection(&state.text_lines,
                        [state.text_selection_start.x,state.text_selection_start.y],[point.x,point.y],false);
                } else if state.is_dragging {
                    if state.mode == AppMode::ToolbarActive {
                        if let Some(drag) = state.drag_mode {
                            if let Some(rect) = &mut state.crop_rect {
                                let dx = point.x - state.current_pt.x;
                                let dy = point.y - state.current_pt.y;
                                let mut l = rect.left; let mut r = rect.right;
                                let mut t = rect.top; let mut b = rect.bottom;
                                match drag {
                                    DragMode::Move => { l+=dx; r+=dx; t+=dy; b+=dy; }
                                    DragMode::ResizeLeft => { l+=dx; }
                                    DragMode::ResizeRight => { r+=dx; }
                                    DragMode::ResizeTop => { t+=dy; }
                                    DragMode::ResizeBottom => { b+=dy; }
                                    DragMode::ResizeTopLeft => { l+=dx; t+=dy; }
                                    DragMode::ResizeTopRight => { r+=dx; t+=dy; }
                                    DragMode::ResizeBottomLeft => { l+=dx; b+=dy; }
                                    DragMode::ResizeBottomRight => { r+=dx; b+=dy; }
                                }
                                if r - l < 10 { if dx > 0 { r = l + 10; } else { l = r - 10; } }
                                if b - t < 10 { if dy > 0 { b = t + 10; } else { t = b - 10; } }
                                // clamp to screen
                                let w = state.frame_width as i32; let h = state.frame_height as i32;
                                if l < 0 { let d = -l; l+=d; if drag == DragMode::Move { r+=d; } }
                                if t < 0 { let d = -t; t+=d; if drag == DragMode::Move { b+=d; } }
                                if r > w { let d = r-w; r-=d; if drag == DragMode::Move { l-=d; } }
                                if b > h { let d = b-h; b-=d; if drag == DragMode::Move { t-=d; } }
                                
                                rect.left = l; rect.right = r; rect.top = t; rect.bottom = b;
                                state.current_pt = point;
                                setup_toolbar(state);
                            }
                        }
                    } else {
                        state.current_pt=point;
                        if let Some(stroke)=&mut state.current_stroke {
                            if stroke.last().is_none_or(|p| p.x!=point.x || p.y!=point.y) { stroke.push(point); }
                        }
                    }
                } else if state.mode == AppMode::Selecting {
                    state.current_pt=point;
                    let mut global=POINT::default();let _=GetCursorPos(&mut global);
                    state.hovered_rect=state.ui_service.as_mut().and_then(|svc| svc.hit_test_point(global,HitTestMode::Window).ok().flatten())
                        .and_then(|rects| rects.first().map(|r| RECT{left:r.left(),top:r.top(),right:r.right(),bottom:r.bottom()}));
                } else if state.mode == AppMode::ToolbarActive {
                    if let Some(rect) = &state.crop_rect {
                        state.drag_mode = hit_test_resize(rect, point, 8);
                    }
                }
                let _=InvalidateRect(Some(hwnd),None,false);
            }
            LRESULT(0)
        },
        WM_LBUTTONUP => unsafe {
            if let Some(state)=state_mut(hwnd) {
                let point=client_cursor(hwnd);
                if state.is_selecting_text {
                    state.text_selection_current=point;
                    state.selected_spans=text_selection::selection(&state.text_lines,
                        [state.text_selection_start.x,state.text_selection_start.y],[point.x,point.y],false);
                    state.is_selecting_text=false;
                } else if state.is_dragging {
                    state.current_pt=point;
                    let rect=RECT {left:state.start_pt.x.min(point.x),top:state.start_pt.y.min(point.y),
                        right:state.start_pt.x.max(point.x),bottom:state.start_pt.y.max(point.y)};
                    match state.mode {
                        AppMode::Selecting=>{
                            let mut selected=rect;
                            if rect.right-rect.left<=5 && rect.bottom-rect.top<=5 {
                                if let Some(r)=state.hovered_rect {
                                    let mut a=POINT{x:r.left,y:r.top};let mut b=POINT{x:r.right,y:r.bottom};
                                    let _=ScreenToClient(hwnd,&mut a);let _=ScreenToClient(hwnd,&mut b);
                                    selected=RECT{left:a.x,top:a.y,right:b.x,bottom:b.y};
                                }
                            }
                            if let Some([l,t,r,b])=clip_rect([selected.left,selected.top,selected.right,selected.bottom],state.frame_width,state.frame_height) {
                                state.crop_rect=Some(RECT{left:l as i32,top:t as i32,right:r as i32,bottom:b as i32});
                                state.mode=AppMode::ToolbarActive;
                                actions::status(hwnd,state,"选区已锁定，请选择工具；右键重新框选");
                            }
                        }
                        AppMode::ToolbarActive=>{
                            if let Some(rect) = &state.crop_rect {
                                state.drag_mode = hit_test_resize(rect, point, 8);
                            }
                        }
                        AppMode::DrawingPen | AppMode::DrawingLine=>{
                            let points=if state.mode==AppMode::DrawingLine {vec![state.start_pt,point]} else {
                                let mut points=state.current_stroke.take().unwrap_or_default();points.push(point);points
                            };
                            state.elements.push(DrawElement::Stroke(StrokeData{points,color:state.current_color,thickness:state.current_thickness,is_eraser:state.is_eraser}));state.redo_stack.clear();
                        }
                        AppMode::DrawingRect=>{state.elements.push(DrawElement::Rect(RectData{rect,color:state.current_color,thickness:state.current_thickness}));state.redo_stack.clear();}
                        AppMode::DrawingArrow=>{state.elements.push(DrawElement::Arrow(ArrowData{start:state.start_pt,end:point,color:state.current_color,thickness:state.current_thickness}));state.redo_stack.clear();}
                        AppMode::DrawingEllipse=>{state.elements.push(DrawElement::Ellipse(EllipseData{rect,color:state.current_color,thickness:state.current_thickness}));state.redo_stack.clear();}
                        AppMode::DrawingMosaic=>{state.elements.push(DrawElement::Mosaic(MosaicData{rect,block_size:12}));state.redo_stack.clear();}
                        _=>{}
                    }
                    state.is_dragging=false;
                }
                let _=ReleaseCapture();
                setup_toolbar(state);
                let _=InvalidateRect(Some(hwnd),None,false);
            }
            LRESULT(0)
        },
        WM_LBUTTONDBLCLK => unsafe {
            if let Some(state)=state_mut(hwnd) {
                if state.mode == AppMode::ToolbarActive && state.crop_rect.is_some_and(|r|pt_in_rect(&r,client_cursor(hwnd))) {
                    handle_action(hwnd,Action::Accept);
                }
            }
            LRESULT(0)
        },
        WM_RBUTTONDOWN => unsafe {
            if let Some(state)=state_mut(hwnd) {
                if actions::busy(state)||matches!(state.mode,AppMode::Recording|AppMode::Saved){return LRESULT(0);}
                actions::commit_text(state);
                if is_annotation_mode(state.mode)||state.mode==AppMode::SelectingText {
                    state.mode=AppMode::ToolbarActive;state.selected_spans.clear();
                } else if state.crop_rect.is_some() {
                    state.crop_rect=None;state.mode=AppMode::Selecting;state.elements.clear();state.redo_stack.clear();
                    state.ocr_blocks=None;state.translated_lines=None;state.text_lines.clear();state.status_text.clear();
                } else {handle_action(hwnd,Action::Cancel);return LRESULT(0);}
                setup_toolbar(state);let _=InvalidateRect(Some(hwnd),None,false);
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

                // Native capture interaction.
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

                // Native capture interaction.
                let saved_dc=SaveDC(hdc_mem);
                if let Some(r)=state.crop_rect { IntersectClipRect(hdc_mem,r.left,r.top,r.right,r.bottom); }
                draw_elements(hdc_mem, state, &bmi);

                // Native capture interaction.
                draw_in_progress(hdc_mem, state);

                // Native capture interaction.


                // Native capture interaction.
                draw_translated_lines(hdc_mem, state);
                draw_ocr_blocks(hdc_mem,state);
                let _ = RestoreDC(hdc_mem,saved_dc);
                let _ = DeleteDC(hdc_mem);
                if state.mode == AppMode::Selecting && state.crop_rect.is_none() {
                    draw_magnifier(hdc_mem, state.current_pt, state);
                }

                // Native capture interaction.
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

// Native capture interaction.
unsafe fn handle_action(hwnd: HWND, action: Action) {
    if let Some(state)=state_mut(hwnd) { actions::handle(hwnd,state,action); }
}

unsafe fn client_cursor(hwnd: HWND) -> POINT {
    let mut point=POINT::default();let _=GetCursorPos(&mut point);let _=ScreenToClient(hwnd,&mut point);point
}

/// Bake annotations into the frozen image and export the selected region.
pub(crate) unsafe fn finish_capture(hwnd: HWND, action: Action) {
    let Some(state)=state_mut(hwnd) else {return;};
    actions::commit_text(state);
    match capture_pixels(state) {
        Ok((rect,final_pixels))=>{
            if let Some(tx)=state.result_tx.take() {
                let _=tx.send(Some(CroppedResult{x:rect.left,y:rect.top,width:rect.right-rect.left,height:rect.bottom-rect.top,action,final_pixels}));
                let _=DestroyWindow(hwnd);
            }
        }
        Err(e)=>actions::status(hwnd,state,format!("截图失败：{e}")),
    }
}

pub(crate) fn snapshot_pixels(state: &AppState) -> std::result::Result<(RECT,Vec<u8>),String> {
    let r=state.crop_rect.ok_or("请先选择范围")?;
    let [l,t,r,b]=clip_rect([r.left,r.top,r.right,r.bottom],state.frame_width,state.frame_height).ok_or("空选区")?;
    let rect=RECT{left:l as i32,top:t as i32,right:r as i32,bottom:b as i32};
    let rgba=crop_rgba(&state.bgra_pixels,state.frame_width,state.frame_height,[rect.left,rect.top,rect.right,rect.bottom])?;
    Ok((rect,rgba))
}

pub(crate) unsafe fn capture_pixels(state: &AppState) -> std::result::Result<(RECT,Vec<u8>),String> {
    let (rect,original)=snapshot_pixels(state)?;
    if state.elements.is_empty() && state.translated_lines.is_none() {return Ok((rect,original));}
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
        return Err("无法创建图片缓冲区".into());
    };
    let old_bitmap = SelectObject(hdc_mem, hbm.into());

    std::ptr::copy_nonoverlapping(state.bgra_pixels.as_ptr(), pbits as *mut u8, state.bgra_pixels.len());

    // Native capture interaction.
    draw_elements(hdc_mem, state, &bmi);
    draw_translated_lines(hdc_mem, state);

    let _ = GdiFlush();
    let baked_pixels = std::slice::from_raw_parts(pbits as *const u8, state.bgra_pixels.len());
    let final_pixels = crop_rgba(baked_pixels, state.frame_width, state.frame_height,
        [rect.left, rect.top, rect.right, rect.bottom]);
    SelectObject(hdc_mem, old_bitmap);
    let _ = DeleteObject(hbm.into());
    let _ = DeleteDC(hdc_mem);
    Ok((rect,final_pixels?))
}

fn is_annotation_mode(mode: AppMode) -> bool {
    matches!(
        mode,
        AppMode::DrawingPen
            | AppMode::DrawingLine
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
