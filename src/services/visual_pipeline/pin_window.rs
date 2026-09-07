use std::ffi::c_void;
use windows::core::{Result, w};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostQuitMessage, RegisterClassW, SetWindowLongPtrW, TranslateMessage, GetWindowLongPtrW,
    GWLP_USERDATA, MSG, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NCHITTEST, WM_PAINT,
    WNDCLASSW, WS_EX_TOPMOST, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE, HTCAPTION,
    LoadCursorW, IDC_ARROW, WM_NCCREATE, WM_RBUTTONUP,
    WS_EX_LAYERED, SetLayeredWindowAttributes, LWA_ALPHA, WM_MOUSEWHEEL, WM_KEYDOWN,
    SetWindowPos, SWP_NOMOVE, SWP_NOZORDER, GetClientRect
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;

struct PinState {
    width: u32,
    height: u32,
    bgra_pixels: Vec<u8>,
    scale: f32,
    opacity: u8,
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
    
    // RegisterClass might fail if already registered by another thread, ignore the error
    unsafe {
        let _ = RegisterClassW(&class);
    }

    let width = state.width;
    let height = state.height;
    let state_ptr = Box::into_raw(state);

    // Spawn at the specific coordinates
    // (start_x and start_y are already passed into this function)

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
        let _ = SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), (*state_ptr).opacity, LWA_ALPHA);
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

extern "system" fn pin_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => unsafe {
            let create = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            LRESULT(1)
        },
        WM_NCHITTEST => {
            // Allows the user to drag the window by clicking anywhere on it
            LRESULT(HTCAPTION as isize)
        }
        WM_LBUTTONDBLCLK | WM_RBUTTONUP => unsafe {
            // Double click or right click to close the pinned image
            DestroyWindow(hwnd).ok();
            LRESULT(0)
        }
        WM_PAINT => unsafe {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                let state = &*ptr;
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

                let mut rect = windows::Win32::Foundation::RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let cw = rect.right - rect.left;
                let ch = rect.bottom - rect.top;

                StretchDIBits(
                    dc,
                    0, 0, cw, ch,
                    0, 0, state.width as i32, state.height as i32,
                    Some(state.bgra_pixels.as_ptr() as *const _),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
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
                    let _ = SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), state.opacity, LWA_ALPHA);
                } else {
                    let mut scale = state.scale + delta * 0.1;
                    if scale < 0.1 { scale = 0.1; }
                    if scale > 5.0 { scale = 5.0; }
                    state.scale = scale;
                    let new_w = (state.width as f32 * state.scale) as i32;
                    let new_h = (state.height as f32 * state.scale) as i32;
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
