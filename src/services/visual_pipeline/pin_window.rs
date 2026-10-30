use std::ffi::c_void;
use std::time::Instant;
use windows::core::{Result, w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, LineTo, MoveToEx,
    ScreenToClient, SelectObject, SetBkMode, SetTextColor, StretchDIBits, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_SINGLELINE, DT_VCENTER, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, VK_ESCAPE, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, GetWindowRect, KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW,
    SendMessageW, SetCursor, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos,
    TranslateMessage, GWLP_USERDATA, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IDC_ARROW, IDC_HAND, IDC_SIZEALL, IDC_SIZENESW,
    IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, LWA_ALPHA, MSG, SWP_NOMOVE, SWP_NOZORDER, WM_ACTIVATE,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCCREATE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_PAINT, WM_RBUTTONUP,
    WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WS_VISIBLE,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;
const TME_LEAVE: u32 = 0x00000002;
const TME_NONCLIENT: u32 = 0x00000010;

#[repr(C)]
#[allow(non_snake_case)]
struct TRACKMOUSEEVENT {
    cbSize: u32,
    dwFlags: u32,
    hwndTrack: HWND,
    dwHoverTime: u32,
}

unsafe extern "system" {
    fn TrackMouseEvent(lpEventTrack: *mut TRACKMOUSEEVENT) -> i32;
}

const TIMER_AUTOHIDE_ID: usize = 2001;
const TOPBAR_HEIGHT: i32 = 30;
const TOPBAR_TRIGGER_HEIGHT: i32 = 44; // 顶部聚焦感应区

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PinButtonAction {
    Close,
    ToggleAutoHide,
    ResetScale,
}

#[derive(Clone, Debug)]
struct PinButton {
    action: PinButtonAction,
    rect: RECT,
    text: &'static str,
    tooltip: &'static str,
    is_active: bool,
}

struct PinState {
    width: u32,
    height: u32,
    bgra_pixels: Vec<u8>,
    scale: f32,
    opacity: u8,

    // 视频播放器风格顶栏与聚焦状态
    is_auto_hide: bool,
    is_topbar_visible: bool,
    is_mouse_in_window: bool,
    last_cursor_pos: POINT,
    last_active_time: Instant,
    hovered_action: Option<PinButtonAction>,
    pressed_action: Option<PinButtonAction>,
    status_hint: Option<(String, Instant)>,
    buttons: Vec<PinButton>,
}

impl PinState {
    fn update_buttons(&mut self, win_w: i32) {
        self.buttons.clear();
        let btn_w = 26;
        let btn_h = 22;
        let top = (TOPBAR_HEIGHT - btn_h) / 2;
        let mut right = win_w - 6;

        // 1. 关闭按钮 (最右)
        self.buttons.push(PinButton {
            action: PinButtonAction::Close,
            rect: RECT {
                left: right - btn_w,
                top,
                right,
                bottom: top + btn_h,
            },
            text: "✕",
            tooltip: "关闭 (Esc 或 右键)",
            is_active: false,
        });
        right -= btn_w + 4;

        // 2. 自动收起 / 固定顶栏切换按钮 (选择项)
        self.buttons.push(PinButton {
            action: PinButtonAction::ToggleAutoHide,
            rect: RECT {
                left: right - btn_w,
                top,
                right,
                bottom: top + btn_h,
            },
            text: if self.is_auto_hide { "浮" } else { "固" },
            tooltip: if self.is_auto_hide {
                "当前: 自动悬浮收起 (点击固定顶栏)"
            } else {
                "当前: 固定顶栏 (点击开启自动收起)"
            },
            is_active: !self.is_auto_hide,
        });
        right -= btn_w + 4;

        // 3. 1:1 原尺寸恢复按钮
        self.buttons.push(PinButton {
            action: PinButtonAction::ResetScale,
            rect: RECT {
                left: right - 32,
                top,
                right,
                bottom: top + btn_h,
            },
            text: "1:1",
            tooltip: "恢复 100% 原始尺寸 (双击画面亦可)",
            is_active: (self.scale - 1.0).abs() < 0.01,
        });
    }

    fn set_status(&mut self, hint: String) {
        self.status_hint = Some((hint, Instant::now()));
    }
}

pub fn spawn_pin_window(start_x: i32, start_y: i32, width: u32, height: u32, rgba_pixels: Vec<u8>) {
    std::thread::spawn(move || {
        let mut bgra = vec![0u8; rgba_pixels.len()];
        for i in (0..rgba_pixels.len()).step_by(4) {
            bgra[i] = rgba_pixels[i + 2];     // B
            bgra[i + 1] = rgba_pixels[i + 1]; // G
            bgra[i + 2] = rgba_pixels[i];     // R
            bgra[i + 3] = rgba_pixels[i + 3]; // A
        }

        let state = Box::new(PinState {
            width,
            height,
            bgra_pixels: bgra,
            scale: 1.0,
            opacity: 255,

            is_auto_hide: true,          // 默认开启视频播放器式自动悬浮收起
            is_topbar_visible: true,     // 打开初始展示顶栏
            is_mouse_in_window: false,
            last_cursor_pos: POINT { x: -1, y: -1 },
            last_active_time: Instant::now(),
            hovered_action: None,
            pressed_action: None,
            status_hint: None,
            buttons: Vec::new(),
        });

        if let Err(e) = run_pin_loop(state, start_x, start_y) {
            tracing::error!("Pin window loop error: {:?}", e);
        }
    });
}

fn run_pin_loop(state: Box<PinState>, start_x: i32, start_y: i32) -> Result<()> {
    let instance = unsafe { GetModuleHandleW(None)? };
    let class_name = w!("DeepCloudNativePin");

    let class = WNDCLASSW {
        hInstance: instance.into(),
        lpszClassName: class_name,
        lpfnWndProc: Some(pin_wnd_proc),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        ..Default::default()
    };

    unsafe {
        let _ = RegisterClassW(&class);
    }

    let width = state.width;
    let height = state.height;
    let state_ptr = Box::into_raw(state);

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class_name,
            w!("DeepCloud Pin"),
            WS_POPUP | WS_VISIBLE,
            start_x,
            start_y,
            width as i32,
            height as i32,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            Some(state_ptr as *const c_void),
        )?
    };

    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), (*state_ptr).opacity, LWA_ALPHA);
        let _ = SetTimer(Some(hwnd), TIMER_AUTOHIDE_ID, 80, None);
    }

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.into() {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    Ok(())
}

unsafe fn trigger_track_mouse(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | TME_NONCLIENT,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    let _ = TrackMouseEvent(&mut tme);
}

extern "system" fn pin_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => unsafe {
            let create = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            LRESULT(1)
        },
        WM_ACTIVATE => unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;
                let is_active = (wparam.0 & 0xFFFF) != 0;
                if !is_active {
                    // 窗口失焦（用户点击了其他应用）：像专业视频播放器一样立即收起顶栏，保持纯净！
                    if state.is_auto_hide && state.is_topbar_visible {
                        state.is_topbar_visible = false;
                        state.hovered_action = None;
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        WM_NCHITTEST => unsafe {
            // 边缘拉伸与控制区命中检测
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }

            let screen_pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let mut client_pt = screen_pt;
            let _ = ScreenToClient(hwnd, &mut client_pt);

            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let win_w = rect.right - rect.left;
            let win_h = rect.bottom - rect.top;

            let resize_border = 5;
            let on_left = client_pt.x >= 0 && client_pt.x < resize_border;
            let on_right = client_pt.x >= win_w - resize_border && client_pt.x < win_w;
            let on_bottom = client_pt.y >= win_h - resize_border && client_pt.y < win_h;

            // 角落拉伸具有高优先级
            if on_bottom && on_left { return LRESULT(HTBOTTOMLEFT as isize); }
            if on_bottom && on_right { return LRESULT(HTBOTTOMRIGHT as isize); }

            let state = &*ptr;
            // 当顶栏处于显示状态时，顶部区域属于视频播放器顶栏交互核心区：
            // 顶部中间绝不返回 HTTOP，防止误判为非客户区触发 WM_MOUSELEAVE 导致顶栏闪退！
            // 仅在最左上与最右上 5px 角落保留对角拉伸手柄
            if state.is_topbar_visible {
                let on_top = client_pt.y >= 0 && client_pt.y < resize_border;
                if on_top && on_left { return LRESULT(HTTOPLEFT as isize); }
                if on_top && on_right { return LRESULT(HTTOPRIGHT as isize); }
            } else {
                let on_top = client_pt.y >= 0 && client_pt.y < resize_border;
                if on_top && on_left { return LRESULT(HTTOPLEFT as isize); }
                if on_top && on_right { return LRESULT(HTTOPRIGHT as isize); }
                if on_top { return LRESULT(HTTOP as isize); }
            }

            if on_left { return LRESULT(HTLEFT as isize); }
            if on_right { return LRESULT(HTRIGHT as isize); }
            if on_bottom { return LRESULT(HTBOTTOM as isize); }

            // 内部全域返回 HTCLIENT，使客户区 WM_MOUSEMOVE / TrackMouseEvent 不被打断！
            LRESULT(HTCLIENT as isize)
        },
        WM_SETCURSOR => unsafe {
            let hit = (lparam.0 & 0xFFFF) as i16;
            // 如果鼠标处于边缘拉伸区(非客户区)，由系统默认渲染双向拉伸箭头 (↔ / ↕ / ⤡ / ⤢)
            if hit != HTCLIENT as i16 {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &*ptr;
                // 1. 如果光标在顶栏按钮上 -> 手型光标
                if state.is_topbar_visible && state.hovered_action.is_some() {
                    let _ = SetCursor(LoadCursorW(None, IDC_HAND).ok());
                    return LRESULT(1);
                }

                // 2. 顶栏拖动空白区与画面区 -> 纯净箭头光标 (整洁自然，避免画面充斥十字移动手柄)
                let _ = SetCursor(LoadCursorW(None, IDC_ARROW).ok());
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        WM_MOUSEMOVE => unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;
                let client_pt = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };

                // 抖动与假性移动过滤：只有鼠标坐标真实改变时才视为用户主动操作
                let dx = (client_pt.x - state.last_cursor_pos.x).abs();
                let dy = (client_pt.y - state.last_cursor_pos.y).abs();
                let is_real_move = dx > 0 || dy > 0;
                state.last_cursor_pos = client_pt;

                let was_in_window = state.is_mouse_in_window;
                state.is_mouse_in_window = true;

                // 首次进入窗口：启动 TrackMouseEvent
                if !was_in_window {
                    trigger_track_mouse(hwnd);
                }

                let mut need_repaint = false;

                // 视频播放器核心聚焦逻辑：
                // 1. 只有当光标滑入顶部感应区 (y < TOPBAR_TRIGGER_HEIGHT) 时，才唤醒顶栏！
                //    在画面主体区移动，不突兀弹出顶栏打扰观看；
                // 2. 一旦顶栏处于唤醒状态：
                //    - 在顶部感应区内：持续锁定显示，重置活跃时间；
                //    - 在画面主体区移动：若顶栏已在显示，真实移动时重置活跃计时（给用户 1.2s 视觉缓冲）。
                if client_pt.y < TOPBAR_TRIGGER_HEIGHT {
                    state.last_active_time = Instant::now();
                    if !state.is_topbar_visible {
                        state.is_topbar_visible = true;
                        need_repaint = true;
                    }
                } else if state.is_topbar_visible && is_real_move {
                    state.last_active_time = Instant::now();
                }

                // 按钮 Hover 状态检测
                let old_hover = state.hovered_action;
                state.hovered_action = if state.is_topbar_visible {
                    state.buttons.iter().find(|b| {
                        client_pt.x >= b.rect.left && client_pt.x < b.rect.right
                            && client_pt.y >= b.rect.top && client_pt.y < b.rect.bottom
                    }).map(|b| b.action)
                } else {
                    None
                };

                if old_hover != state.hovered_action || need_repaint {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        },
        WM_MOUSELEAVE | WM_NCMOUSELEAVE => unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;
                state.is_mouse_in_window = false;
                state.hovered_action = None;

                // 移出窗口：若处于自动收起模式，顶栏立即收起隐藏！
                if state.is_auto_hide && state.is_topbar_visible {
                    state.is_topbar_visible = false;
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        },
        WM_LBUTTONDOWN => unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;
                let pt = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };

                // 1. 如果点击在顶栏按钮上：响应按钮按下
                if state.is_topbar_visible {
                    if let Some(btn) = state.buttons.iter().find(|b| {
                        pt.x >= b.rect.left && pt.x < b.rect.right
                            && pt.y >= b.rect.top && pt.y < b.rect.bottom
                    }) {
                        state.pressed_action = Some(btn.action);
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                        return LRESULT(0);
                    }
                }

                // 2. 视频播放器原生拖动机制 (顶栏空白区 或 画面任意区域拖拽)
                ReleaseCapture().ok();
                let mut screen_pt = POINT::default();
                let _ = GetCursorPos(&mut screen_pt);
                let lparam_val = (((screen_pt.y as i16 as u16 as u32) << 16) | (screen_pt.x as i16 as u16 as u32)) as isize;
                let _ = SendMessageW(
                    hwnd,
                    WM_NCLBUTTONDOWN,
                    Some(WPARAM(HTCAPTION as usize)),
                    Some(LPARAM(lparam_val)),
                );

                // 3. 拖拽完成后的状态恢复 (SendMessageW 在松开鼠标后返回)：
                state.last_active_time = Instant::now();
                let mut after_cur = POINT::default();
                let _ = GetCursorPos(&mut after_cur);
                let mut win_rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut win_rect);
                let is_inside = after_cur.x >= win_rect.left && after_cur.x < win_rect.right
                    && after_cur.y >= win_rect.top && after_cur.y < win_rect.bottom;
                state.is_mouse_in_window = is_inside;
                if is_inside {
                    trigger_track_mouse(hwnd);
                } else if state.is_auto_hide && state.is_topbar_visible {
                    state.is_topbar_visible = false;
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        },
        WM_LBUTTONUP => unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;
                let pt = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };

                let pressed = state.pressed_action.take();
                if state.is_topbar_visible {
                    if let Some(btn) = state.buttons.iter().find(|b| {
                        pt.x >= b.rect.left && pt.x < b.rect.right
                            && pt.y >= b.rect.top && pt.y < b.rect.bottom
                    }) {
                        if pressed == Some(btn.action) {
                            match btn.action {
                                PinButtonAction::Close => {
                                    let _ = DestroyWindow(hwnd);
                                    return LRESULT(0);
                                }
                                PinButtonAction::ToggleAutoHide => {
                                    state.is_auto_hide = !state.is_auto_hide;
                                    if state.is_auto_hide {
                                        state.set_status("已开启自动悬浮收起".to_string());
                                    } else {
                                        state.is_topbar_visible = true;
                                        state.set_status("顶栏已固定常显".to_string());
                                    }
                                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                                }
                                PinButtonAction::ResetScale => {
                                    state.scale = 1.0;
                                    let new_w = state.width as i32;
                                    let new_h = state.height as i32;
                                    state.set_status("100% 原始尺寸".to_string());
                                    let _ = SetWindowPos(hwnd, None, 0, 0, new_w, new_h, SWP_NOMOVE | SWP_NOZORDER);
                                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                                }
                            }
                        }
                    }
                }
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        },
        WM_LBUTTONDBLCLK | WM_NCLBUTTONDBLCLK => unsafe {
            // 视频播放器双击机制：在 100% 原始尺寸 与 150% 放大之间智能切换 (支持客户区与拖拽区双击)
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;
                let mut cur = POINT::default();
                let _ = GetCursorPos(&mut cur);
                let mut client_pt = cur;
                let _ = ScreenToClient(hwnd, &mut client_pt);

                // 排除点击在顶栏按钮上的情况
                let clicked_btn = state.is_topbar_visible && state.buttons.iter().any(|b| {
                    client_pt.x >= b.rect.left && client_pt.x < b.rect.right
                        && client_pt.y >= b.rect.top && client_pt.y < b.rect.bottom
                });
                if !clicked_btn {
                    if (state.scale - 1.0).abs() < 0.05 {
                        state.scale = 1.5;
                        state.set_status("双击放大: 150%".to_string());
                    } else {
                        state.scale = 1.0;
                        state.set_status("双击还原: 100%".to_string());
                    }
                    let new_w = (state.width as f32 * state.scale).round() as i32;
                    let new_h = (state.height as f32 * state.scale).round() as i32;
                    let _ = SetWindowPos(hwnd, None, 0, 0, new_w, new_h, SWP_NOMOVE | SWP_NOZORDER);
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        },
        WM_RBUTTONUP => unsafe {
            // 右键单击快速关闭退出
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        },
        WM_TIMER => unsafe {
            if wparam.0 == TIMER_AUTOHIDE_ID {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
                if !ptr.is_null() {
                    let state = &mut *ptr;
                    let mut need_repaint = false;

                    // 1. 状态提示过期检测 (1.5 秒后清除)
                    if let Some((_, time)) = &state.status_hint {
                        if time.elapsed().as_millis() > 1500 {
                            state.status_hint = None;
                            need_repaint = true;
                        }
                    }

                    // 2. 视频播放器自动悬浮收起控制：
                    // 若处于自动收起模式，且顶栏当前显示：
                    if state.is_auto_hide && state.is_topbar_visible {
                        // 检查鼠标左键是否处于按下状态（拖动或交互中绝对不收起）
                        let is_lbutton_down = (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0;
                        if !is_lbutton_down {
                            let mut cur = POINT::default();
                            let _ = GetCursorPos(&mut cur);
                            let _ = ScreenToClient(hwnd, &mut cur);

                            let in_top_zone = cur.y >= 0 && cur.y < TOPBAR_TRIGGER_HEIGHT;
                            if in_top_zone {
                                // 光标停在顶部感应与控件区域：保持活跃锁定，绝不收起！
                                state.last_active_time = Instant::now();
                            } else {
                                // 光标脱离顶部且在画面区：超过 1.2 秒平滑自动收起！
                                if state.last_active_time.elapsed().as_millis() > 1200 {
                                    state.is_topbar_visible = false;
                                    state.hovered_action = None;
                                    need_repaint = true;
                                }
                            }
                        }
                    }

                    if need_repaint {
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        },
        WM_PAINT => unsafe {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;

                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let cw = rect.right - rect.left;
                let ch = rect.bottom - rect.top;

                // 重新布局顶栏按钮
                state.update_buttons(cw);

                // 双缓冲渲染，彻底消除闪烁
                let dc_mem = CreateCompatibleDC(Some(dc));
                let hbm = CreateCompatibleBitmap(dc, cw, ch);
                let old_bmp = SelectObject(dc_mem, hbm.into());

                // 1. 绘制底层图片画面
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: state.width as i32,
                        biHeight: -(state.height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };

                StretchDIBits(
                    dc_mem,
                    0, 0, cw, ch,
                    0, 0, state.width as i32, state.height as i32,
                    Some(state.bgra_pixels.as_ptr() as *const _),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );

                // 2. 绘制极细外边框 (深色微发丝线)
                let border_pen = CreatePen(PS_SOLID, 1, COLORREF(0x00404040));
                let old_pen = SelectObject(dc_mem, border_pen.into());
                let old_brush = SelectObject(dc_mem, windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::HOLLOW_BRUSH));
                let _ = windows::Win32::Graphics::Gdi::Rectangle(dc_mem, 0, 0, cw, ch);
                SelectObject(dc_mem, old_brush);
                SelectObject(dc_mem, old_pen);
                let _ = DeleteObject(border_pen.into());

                // 3. 顶栏绘制 (纯悬浮浮岛设计，无实心黑条截断画面，通顶沉浸质感)
                if state.is_topbar_visible {
                    // 居中信息与拖拽胶囊 (参考播放器浮岛设计，独立半透明圆角胶囊手柄)
                    let font_name: Vec<u16> = "Segoe UI".encode_utf16().chain(std::iter::once(0)).collect();
                    let hfont = CreateFontW(
                        13, 0, 0, 0, 500, 0, 0, 0, DEFAULT_CHARSET,
                        OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0,
                        PCWSTR(font_name.as_ptr()),
                    );
                    let old_font = SelectObject(dc_mem, hfont.into());
                    let old_bk = SetBkMode(dc_mem, TRANSPARENT);

                    let (info_text, is_hint) = if let Some((hint, _)) = &state.status_hint {
                        (hint.clone(), true)
                    } else {
                        (format!("{}% · {}×{}", (state.scale * 100.0).round() as i32, cw, ch), false)
                    };
                    let mut winfo: Vec<u16> = info_text.encode_utf16().collect();

                    let pill_w = ((winfo.len() as i32 * 8) + 24).max(110).min((cw - 110).max(10));
                    let pill_x = (cw - pill_w) / 2;
                    let pill_rect = RECT {
                        left: pill_x,
                        top: 4,
                        right: pill_x + pill_w,
                        bottom: 4 + 22,
                    };

                    // 半透明深色磨砂感胶囊背景
                    let pill_bg = CreateSolidBrush(COLORREF(0x001E1E1E));
                    FillRect(dc_mem, &pill_rect, pill_bg);
                    let _ = DeleteObject(pill_bg.into());

                    // 胶囊描边
                    let pill_pen = CreatePen(PS_SOLID, 1, if is_hint { COLORREF(0x0070DF50) } else { COLORREF(0x004A4A4A) });
                    let old_pp = SelectObject(dc_mem, pill_pen.into());
                    let old_pb = SelectObject(dc_mem, windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::HOLLOW_BRUSH));
                    let _ = windows::Win32::Graphics::Gdi::Rectangle(dc_mem, pill_rect.left, pill_rect.top, pill_rect.right, pill_rect.bottom);
                    SelectObject(dc_mem, old_pb);
                    SelectObject(dc_mem, old_pp);
                    let _ = DeleteObject(pill_pen.into());

                    let old_color = SetTextColor(
                        dc_mem,
                        if is_hint { COLORREF(0x0070DF50) } else { COLORREF(0x00D0D0D0) },
                    );
                    let mut tr = pill_rect;
                    let _ = DrawTextW(dc_mem, &mut winfo, &mut tr, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

                    // 绘制右侧按钮组
                    let btn_font_name: Vec<u16> = "Segoe UI".encode_utf16().chain(std::iter::once(0)).collect();
                    let btn_font = CreateFontW(
                        13, 0, 0, 0, 600, 0, 0, 0, DEFAULT_CHARSET,
                        OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0,
                        PCWSTR(btn_font_name.as_ptr()),
                    );
                    SelectObject(dc_mem, btn_font.into());

                    for btn in &state.buttons {
                        let is_hover = state.hovered_action == Some(btn.action);
                        let is_pressed = state.pressed_action == Some(btn.action);
                        let bg_color = if btn.action == PinButtonAction::Close && (is_hover || is_pressed) {
                            COLORREF(0x002B2BD0) // 关闭按钮高亮红底
                        } else if is_pressed {
                            COLORREF(0x00454545) // 按下深灰
                        } else if is_hover {
                            COLORREF(0x00363636) // 悬停亮灰
                        } else if btn.is_active {
                            COLORREF(0x002A2A2A) // 激活态
                        } else {
                            COLORREF(0x00202020)
                        };

                        let btn_brush = CreateSolidBrush(bg_color);
                        FillRect(dc_mem, &btn.rect, btn_brush);
                        let _ = DeleteObject(btn_brush.into());

                        // 按钮边框
                        let btn_pen = CreatePen(PS_SOLID, 1, if is_hover { COLORREF(0x00555555) } else { COLORREF(0x00303030) });
                        let old_bp = SelectObject(dc_mem, btn_pen.into());
                        let old_bb = SelectObject(dc_mem, windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::HOLLOW_BRUSH));
                        let _ = windows::Win32::Graphics::Gdi::Rectangle(dc_mem, btn.rect.left, btn.rect.top, btn.rect.right, btn.rect.bottom);
                        SelectObject(dc_mem, old_bb);
                        SelectObject(dc_mem, old_bp);
                        let _ = DeleteObject(btn_pen.into());

                        // 按钮文字颜色
                        let text_color = if btn.action == PinButtonAction::Close {
                            if is_hover { COLORREF(0x00FFFFFF) } else { COLORREF(0x00A0A0A0) }
                        } else if btn.action == PinButtonAction::ToggleAutoHide && btn.is_active {
                            COLORREF(0x001ED953) // 固定激活呈现绿色
                        } else if is_hover {
                            COLORREF(0x00FFFFFF)
                        } else {
                            COLORREF(0x00C0C0C0)
                        };

                        SetTextColor(dc_mem, text_color);
                        let mut wtext: Vec<u16> = btn.text.encode_utf16().collect();
                        let mut tr = btn.rect;
                        let _ = DrawTextW(dc_mem, &mut wtext, &mut tr, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    }

                    SetTextColor(dc_mem, old_color);
                    SetBkMode(dc_mem, windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
                    SelectObject(dc_mem, old_font);
                    let _ = DeleteObject(hfont.into());
                    let _ = DeleteObject(btn_font.into());
                } else if state.is_auto_hide {
                    // 收起状态下的视频播放器式顶部极微指示手柄
                    let pill_w = 32;
                    let pill_h = 3;
                    let pill_x = (cw - pill_w) / 2;
                    let pill_rect = RECT {
                        left: pill_x,
                        top: 2,
                        right: pill_x + pill_w,
                        bottom: 2 + pill_h,
                    };
                    let pill_brush = CreateSolidBrush(COLORREF(0x00555555));
                    FillRect(dc_mem, &pill_rect, pill_brush);
                    let _ = DeleteObject(pill_brush.into());
                }

                // 一次性双缓冲拷贝
                let _ = BitBlt(dc, 0, 0, cw, ch, Some(dc_mem), 0, 0, SRCCOPY);
                SelectObject(dc_mem, old_bmp);
                let _ = DeleteObject(hbm.into());
                let _ = DeleteDC(dc_mem);
            }

            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        },
        WM_MOUSEWHEEL => unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &mut *ptr;
                let delta = ((wparam.0 >> 16) as i16) as f32 / 120.0;
                let ctrl_pressed = (wparam.0 & 0x0008) != 0;

                if ctrl_pressed {
                    let mut op = state.opacity as f32 + delta * 25.0;
                    if op < 25.0 { op = 25.0; }
                    if op > 255.0 { op = 255.0; }
                    state.opacity = op as u8;
                    state.set_status(format!("透明度: {}%", (state.opacity as f32 / 255.0 * 100.0).round() as i32));
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), state.opacity, LWA_ALPHA);
                } else {
                    let mut scale = state.scale + delta * 0.1;
                    if scale < 0.15 { scale = 0.15; }
                    if scale > 5.0 { scale = 5.0; }
                    state.scale = scale;
                    state.set_status(format!("缩放: {}%", (state.scale * 100.0).round() as i32));
                    let new_w = (state.width as f32 * state.scale).round() as i32;
                    let new_h = (state.height as f32 * state.scale).round() as i32;
                    let _ = SetWindowPos(hwnd, None, 0, 0, new_w, new_h, SWP_NOMOVE | SWP_NOZORDER);
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        },
        WM_KEYDOWN => unsafe {
            if wparam.0 == VK_ESCAPE.0 as usize {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        },
        WM_DESTROY => unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_AUTOHIDE_ID);
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if ptr != 0 {
                drop(Box::from_raw(ptr as *mut PinState));
            }
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
