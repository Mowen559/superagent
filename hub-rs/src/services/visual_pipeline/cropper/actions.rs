//! UI actions: background providers/recording and explicit result feedback.
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;
use super::types::*;
use super::toolbar::setup_toolbar;
use super::text_selection::{self, TextLine};
use crate::services::visual_pipeline::{native_cropper, record_manager, ffmpeg_engine::AudioConfig};

pub enum RecordOutcome { Started, Paused, Resumed, Saved(PathBuf), Discarded }

pub fn busy(state: &AppState) -> bool { state.analysis_job.is_some() || state.record_job.is_some() }

pub unsafe fn status(hwnd: HWND, state: &mut AppState, message: impl Into<String>) {
    state.status_text = message.into();
    let title: Vec<u16> = format!("SnowShot · {}", state.status_text).encode_utf16().chain(Some(0)).collect();
    let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
    let _ = InvalidateRect(Some(hwnd), None, false);
}

pub unsafe fn copy_text(hwnd: HWND, text: &str) -> Result<(), String> {
    if text.is_empty() { return Err("请先选择文字".into()); }
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    OpenClipboard(Some(hwnd)).map_err(|e| e.to_string())?;
    let result = (|| {
        let memory = GlobalAlloc(GHND, text.len()*2).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(memory) as *mut u16;
        if ptr.is_null() { let _ = windows::Win32::Foundation::GlobalFree(Some(memory)); return Err("剪贴板内存分配失败".into()); }
        std::ptr::copy_nonoverlapping(text.as_ptr(), ptr, text.len());
        let _ = GlobalUnlock(memory);
        if let Err(e) = EmptyClipboard() { let _ = windows::Win32::Foundation::GlobalFree(Some(memory)); return Err(e.to_string()); }
        if let Err(e) = SetClipboardData(13, Some(HANDLE(memory.0))) {
            let _ = windows::Win32::Foundation::GlobalFree(Some(memory));
            return Err(e.to_string());
        }
        Ok(())
    })();
    let _ = CloseClipboard();
    result
}

pub unsafe fn copy_selection(hwnd: HWND, state: &mut AppState) {
    let spans = if state.selected_spans.is_empty() {
        text_selection::selection(&state.text_lines, [0,0], [0,0], true)
    } else { state.selected_spans.clone() };
    match copy_text(hwnd, &text_selection::selected_text(&state.text_lines, &spans)) {
        Ok(()) => status(hwnd, state, "文字已复制，拖动可继续选字"),
        Err(e) => status(hwnd, state, format!("复制失败：{e}")),
    }
}

pub unsafe fn commit_text(state: &mut AppState) {
    if let Some(raw) = state.input_window.take() {
        let child = HWND(raw as *mut _);
        let mut buffer = vec![0u16; GetWindowTextLengthW(child).max(0) as usize + 1];
        let length = GetWindowTextW(child, &mut buffer);
        let text = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
        if !text.trim().is_empty() {
            if let Some((pos, _)) = state.active_text_input.take() {
                state.elements.push(DrawElement::Text(TextData { pos, text, color: state.current_color, font_size: 24 }));
                state.redo_stack.clear();
            }
        }
        let _ = DestroyWindow(child);
    }
    state.active_text_input = None;
}

pub unsafe fn start_text(hwnd: HWND, state: &mut AppState, point: windows::Win32::Foundation::POINT) {
    commit_text(state);
    let Some(rect) = state.crop_rect else { return; };
    let width = (rect.right-point.x).min(360).max(1);
    let height = (rect.bottom-point.y).min(100).max(1);
    match CreateWindowExW(WINDOW_EX_STYLE(0), w!("EDIT"), w!(""),
        WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
        point.x, point.y, width, height, Some(hwnd), None, None, None) {
        Ok(child) => {
            SendMessageW(child, WM_SETFONT, Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)), Some(LPARAM(1)));
            state.input_window = Some(child.0 as isize);
            state.active_text_input = Some((point, String::new()));
            let _ = SetFocus(Some(child));
            status(hwnd, state, "输入文字，点击工具栏提交；支持输入法与换行");
        }
        Err(e) => status(hwnd, state, format!("文字输入框创建失败：{e}")),
    }
}

unsafe fn save_png(hwnd: HWND, state: &mut AppState) -> Result<(), String> {
    commit_text(state);
    let (rect, rgba) = native_cropper::capture_pixels(state)?;
    let mut name = [0u16; 32768];
    let default: Vec<u16> = format!("截图_{}.png", chrono::Local::now().format("%Y%m%d_%H%M%S")).encode_utf16().collect();
    name[..default.len()].copy_from_slice(&default);
    let filter: Vec<u16> = "PNG 图片\0*.png\0\0".encode_utf16().collect();
    let mut dialog = OPENFILENAMEW { lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd, lpstrFilter: PCWSTR(filter.as_ptr()), lpstrFile: PWSTR(name.as_mut_ptr()),
        nMaxFile: name.len() as u32, lpstrDefExt: w!("png"),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR, ..Default::default() };
    if !GetSaveFileNameW(&mut dialog).as_bool() {
        if CommDlgExtendedError() != COMMON_DLG_ERRORS(0) { return Err("无法打开保存对话框".into()); }
        return Ok(());
    }
    let path = PathBuf::from(String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]));
    image::RgbaImage::from_raw((rect.right-rect.left) as u32, (rect.bottom-rect.top) as u32, rgba)
        .ok_or("无效图片尺寸")?.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    state.last_saved_path = Some(path.clone());
    status(hwnd, state, format!("图片已保存：{}", path.display()));
    setup_toolbar(state);
    Ok(())
}

unsafe fn recording_region(hwnd: HWND, state: &AppState, enabled: bool) -> Result<(), String> {
    if !enabled { SetWindowRgn(hwnd, None, true); let _ = SetWindowDisplayAffinity(hwnd, WDA_NONE); return Ok(()); }
    SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).map_err(|e| format!("无法排除录屏控制条：{e}"))?;
    let rect = state.crop_rect.ok_or("请先选择录屏范围")?;
    let region = CreateRectRgn(0,0,0,0);
    let mut add = |r: RECT| {
        let part = CreateRectRgn(r.left,r.top,r.right,r.bottom);
        CombineRgn(Some(region),Some(region),Some(part),RGN_OR);
        let _ = DeleteObject(part.into());
    };
    for r in [RECT { left: rect.left-3, top: rect.top-3, right: rect.right+3, bottom: rect.top },
        RECT { left: rect.left-3, top: rect.bottom, right: rect.right+3, bottom: rect.bottom+3 },
        RECT { left: rect.left-3, top: rect.top, right: rect.left, bottom: rect.bottom },
        RECT { left: rect.right, top: rect.top, right: rect.right+3, bottom: rect.bottom }] { add(r); }
    if let (Some(first), Some(last)) = (state.record_buttons.first(), state.record_buttons.last()) {
        add(RECT { left: first.rect.left-128, top: first.rect.top-4, right: last.rect.right+8, bottom: last.rect.bottom+4 });
    }
    if SetWindowRgn(hwnd, Some(region), true) == 0 { let _ = DeleteObject(region.into()); return Err("无法创建录屏控制条区域".into()); }
    Ok(())
}

unsafe fn start_record_job(hwnd: HWND, state: &mut AppState, action: Action) -> Result<(), String> {
    let rect = state.crop_rect.ok_or("请先选择范围")?;
    let dir = state.vp.as_ref().ok_or("视觉引擎不可用")?.artifact_dir().to_path_buf();
    if action == Action::Record {
        state.mode = AppMode::Recording;
        setup_toolbar(state);
        if let Err(e) = recording_region(hwnd,state,true) { state.mode=AppMode::ToolbarActive; setup_toolbar(state); return Err(e); }
        let _ = UpdateWindow(hwnd);
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new().name("CropperRecording".into()).spawn(move || {
        let result = match action {
            Action::Record => record_manager::start_recording(rect.left,rect.top,rect.right-rect.left,rect.bottom-rect.top,dir,AudioConfig::None).map(|_| RecordOutcome::Started),
            Action::RecordPause => record_manager::pause_recording().map(|_| RecordOutcome::Paused),
            Action::RecordResume => record_manager::resume_recording().map(|_| RecordOutcome::Resumed),
            Action::RecordDiscard => record_manager::discard_recording().map(|_| RecordOutcome::Discarded),
            _ => {
                let path = dir.join(format!("recording_{}.mp4",uuid::Uuid::new_v4()));
                record_manager::stop_and_export(path.clone()).map(|_| RecordOutcome::Saved(path))
            }
        };
        if sender.send(result).is_err() { let _ = record_manager::discard_recording(); }
    }).map_err(|e| e.to_string())?;
    state.record_job = Some(receiver);
    status(hwnd,state,if action == Action::Record { "正在启动录屏…" } else { "正在处理录屏，请稍候…" });
    Ok(())
}

pub unsafe fn handle(hwnd: HWND, state: &mut AppState, action: Action) {
    if state.record_job.is_some() { return; }
    if action == Action::Cancel {
        if state.mode == AppMode::Recording { handle(hwnd,state,Action::RecordStop); }
        else { state.analysis_job = None; let _ = DestroyWindow(hwnd); }
        return;
    }
    if state.analysis_job.is_some() { return; }
    commit_text(state);
    let _ = SetFocus(Some(hwnd));
    if state.mode == AppMode::Recording && !matches!(action, Action::RecordPause | Action::RecordResume | Action::RecordStop | Action::RecordDiscard) { return; }
    match action {
        Action::Ocr | Action::Translate => {
            if let Some(vp) = state.vp.clone() {
                match native_cropper::snapshot_pixels(state).and_then(|(rect,pixels)| {
                    let existing = if action==Action::Translate { state.ocr_blocks.as_ref().map(|blocks| blocks.iter().map(|b| TextLine {
                        rect:[b.rect.left,b.rect.top,b.rect.right,b.rect.bottom],text:b.text.clone() }).collect()).unwrap_or_default() } else { vec![] };
                    super::analysis::start(vp,pixels,[rect.left,rect.top,rect.right,rect.bottom],action==Action::Translate,existing)
                }) {
                    Ok(job) => { state.analysis_job=Some(job); status(hwnd,state,if action==Action::Ocr { "正在识别完整选区…" } else { "正在逐块翻译…" }); },
                    Err(e) => status(hwnd,state,format!("识别/翻译失败：{e}")),
                }
            }
        }
        Action::CopyText => copy_selection(hwnd,state),
        Action::OpenFolder => {
            if let Some(path)=&state.last_saved_path {
                let mut command=std::process::Command::new("explorer.exe");
                command.arg(format!("/select,{}",path.display()));
                if let Err(e)=command.spawn() { status(hwnd,state,format!("打开位置失败：{e}")); }
            }
        }
        Action::CopyPath => {
            if let Some(path)=&state.last_saved_path {
                match copy_text(hwnd,&path.display().to_string()) {
                    Ok(())=>status(hwnd,state,format!("保存位置已复制：{}",path.display())),
                    Err(e)=>status(hwnd,state,format!("复制失败：{e}")),
                }
            }
        }
        Action::Save => { if let Err(e)=save_png(hwnd,state) { status(hwnd,state,format!("保存失败：{e}")); } }
        Action::Record | Action::RecordPause | Action::RecordResume | Action::RecordStop | Action::RecordDiscard => {
            if let Err(e)=start_record_job(hwnd,state,action) { status(hwnd,state,format!("录屏失败：{e}")); }
        }
        Action::Undo => { if let Some(e)=state.elements.pop(){state.redo_stack.push(e);} }
        Action::Redo => { if let Some(e)=state.redo_stack.pop(){state.elements.push(e);} }
        Action::ColorPick(color)=>state.current_color=color,
        Action::Thickness(thickness)=>state.current_thickness=thickness,
        Action::Copy | Action::Pin | Action::Accept => { native_cropper::finish_capture(hwnd,action); return; }
        _ => {
            let mode=match action {
                Action::DrawPen | Action::Eraser=>AppMode::DrawingPen,
                Action::DrawLine=>AppMode::DrawingLine,
                Action::DrawRect=>AppMode::DrawingRect,
                Action::DrawArrow=>AppMode::DrawingArrow,
                Action::DrawEllipse=>AppMode::DrawingEllipse,
                Action::DrawText=>AppMode::DrawingText,
                Action::Mosaic=>AppMode::DrawingMosaic,
                _=>return,
            };
            state.mode=mode;
            state.is_eraser=action==Action::Eraser;
            state.selected_spans.clear();
            status(hwnd,state,"选区已锁定，可在范围内标注；Ctrl+Z 撤销 / Ctrl+Y 重做");
        }
    }
    setup_toolbar(state);
    let _=InvalidateRect(Some(hwnd),None,false);
}

pub unsafe fn poll(hwnd: HWND, state: &mut AppState) {
    if state.mode==AppMode::Recording && state.record_job.is_none() {
        if let Some(path)=record_manager::completed_path() {
            state.recording_session=None;state.mode=AppMode::Saved;
            let _=recording_region(hwnd,state,false);state.last_saved_path=Some(path.clone());
            status(hwnd,state,format!("视频已保存：{}",path.display()));setup_toolbar(state);
        }
    }
    let result=state.analysis_job.as_ref().and_then(|job| match job.receiver.try_recv() {
        Ok(result)=>Some(result), Err(TryRecvError::Disconnected)=>Some(Err("识别任务意外结束".into())), _=>None,
    });
    if let Some(result)=result {
        state.analysis_job=None;
        match result {
            Ok(output)=>{
                state.ocr_blocks=Some(output.lines.iter().map(|line| OcrBox { rect:RECT{left:line.rect[0],top:line.rect[1],right:line.rect[2],bottom:line.rect[3]},text:line.text.clone() }).collect());
                state.translated_lines=output.translated.as_ref().map(|lines| lines.iter().zip(&output.lines).map(|(line,source)| TranslatedBox {
                    rect:RECT{left:line.rect[0],top:line.rect[1],right:line.rect[2],bottom:line.rect[3]},original_text:source.text.clone(),translated_text:line.text.clone() }).collect());
                state.text_lines=output.translated.unwrap_or(output.lines);
                state.selected_spans.clear();
                state.mode=AppMode::SelectingText;
                status(hwnd,state,format!("{}完成：{} 行 · 拖动选字，Ctrl+A 全选 / Ctrl+C 复制",if state.translated_lines.is_some(){"翻译"}else{"OCR"},state.text_lines.len()));
            }
            Err(e)=>status(hwnd,state,format!("识别/翻译失败：{e}")),
        }
        setup_toolbar(state);
    }
    let result=state.record_job.as_ref().and_then(|job| match job.try_recv() {
        Ok(result)=>Some(result), Err(TryRecvError::Disconnected)=>Some(Err("录屏任务意外结束".into())), _=>None,
    });
    if let Some(result)=result {
        state.record_job=None;
        match result {
            Ok(RecordOutcome::Started)=>{
                state.recording_session=Some(RecordingState{start_time:std::time::Instant::now(),is_paused:false,elapsed_secs:0});
                status(hwnd,state,"录屏中，停止后显示保存位置");
            }
            Ok(RecordOutcome::Paused)=>{if let Some(session)=&mut state.recording_session{session.is_paused=true;}status(hwnd,state,"录屏已暂停");}
            Ok(RecordOutcome::Resumed)=>{if let Some(session)=&mut state.recording_session{session.start_time=std::time::Instant::now()-std::time::Duration::from_secs(session.elapsed_secs);session.is_paused=false;}status(hwnd,state,"录屏继续");}
            Ok(RecordOutcome::Saved(path))=>{
                state.recording_session=None; state.mode=AppMode::Saved;
                let _=recording_region(hwnd,state,false);
                state.last_saved_path=Some(path.clone());
                status(hwnd,state,format!("视频已保存：{}",path.display()));
            }
            Ok(RecordOutcome::Discarded)=>{
                state.recording_session=None;state.mode=AppMode::ToolbarActive;
                let _=recording_region(hwnd,state,false);status(hwnd,state,"已丢弃本次录屏");
            }
            Err(e)=>{
                if !record_manager::has_session(){state.recording_session=None;state.mode=AppMode::ToolbarActive;let _=recording_region(hwnd,state,false);}
                status(hwnd,state,format!("录屏失败，可重试：{e}"));
            }
        }
        setup_toolbar(state);
        if state.mode==AppMode::Recording { let _=recording_region(hwnd,state,true); }
    }
}
