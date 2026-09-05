use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, SelectObject,
    SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CYVIRTUALSCREEN};

use super::draw::draw_border;
use super::types::{
    pt_in_rect, to_wstring, Action, AppMode, AppState, ToolbarButton, PREDEFINED_COLORS,
};

/// 重新计算并布置所有工具栏按钮（主工具栏、二级子栏、录屏控制条）
pub fn setup_toolbar(state: &mut AppState) {
    let Some(rect) = state.crop_rect else {
        state.buttons.clear();
        state.color_buttons.clear();
        state.sub_tool_buttons.clear();
        state.record_buttons.clear();
        return;
    };

    let screen_height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let btn_w = 36;
    let btn_h = 30;
    let pad = 4;
    let bar_h = 38;

    // 1. 如果正在录屏模式，只展示录屏控制条
    if state.mode == AppMode::Recording {
        state.buttons.clear();
        state.color_buttons.clear();
        state.sub_tool_buttons.clear();
        state.record_buttons.clear();

        let is_paused = state.recording_session.as_ref().map(|s| s.is_paused).unwrap_or(false);
        let rec_actions = [
            if is_paused {
                (Action::RecordResume, "\u{E768}", "继续录制") // Play
            } else {
                (Action::RecordPause, "\u{E769}", "暂停录制")  // Pause
            },
            (Action::RecordStop, "\u{E71A}", "完成并导出"),     // Stop/Save
            (Action::RecordDiscard, "\u{E74D}", "放弃录制"),  // Delete/Trash
        ];

        let total_w = 120 + (btn_w * rec_actions.len() as i32) + pad * (rec_actions.len() as i32 + 2);
        let mut bar_y = rect.bottom + 8;
        if bar_y + bar_h > screen_height {
            bar_y = rect.top - bar_h - 8;
        }
        let start_x = rect.right - total_w;

        let mut curr_x = start_x + 120 + pad; // 前面留给动态时间显示
        for (act, icon, tip) in rec_actions.iter() {
            state.record_buttons.push(ToolbarButton {
                rect: RECT {
                    left: curr_x,
                    top: bar_y + (bar_h - btn_h) / 2,
                    right: curr_x + btn_w,
                    bottom: bar_y + (bar_h + btn_h) / 2,
                },
                text: to_wstring(icon),
                action: *act,
                tooltip: tip.to_string(),
                is_active: false,
            });
            curr_x += btn_w + pad;
        }
        return;
    }

    // 2. 正常截图工具栏
    let main_actions = [
        (Action::DrawPen, "\u{E76D}", "自由画笔"),
        (Action::DrawRect, "\u{E75B}", "矩形标注"),
        (Action::DrawArrow, "\u{E74A}", "箭头标注"),
        (Action::DrawEllipse, "\u{EA3B}", "圆形标注"),
        (Action::DrawText, "\u{E8D2}", "文字标注"),
        (Action::Mosaic, "\u{E790}", "马赛克"),
        (Action::Eraser, "\u{E75C}", "橡皮擦"),
        (Action::Undo, "\u{E7A7}", "撤销 (Ctrl+Z)"),
        (Action::Redo, "\u{E7A6}", "重做 (Ctrl+Y)"),
        (Action::Ocr, "T", "提取文字 (OCR)"),
        (Action::Translate, "\u{E775}", "就地翻译"),
        (Action::Record, "\u{E714}", "区域录屏"),
        (Action::Pin, "\u{E718}", "贴在桌面"),
        (Action::Copy, "\u{E8C8}", "复制到剪贴板"),
        (Action::Accept, "\u{E73E}", "确认完成"),
        (Action::Cancel, "\u{E711}", "关闭退出 (Esc)"),
    ];

    let total_w = (btn_w * main_actions.len() as i32) + (pad * (main_actions.len() as i32 + 1));
    let mut bar_y = rect.bottom + 8;
    if bar_y + bar_h > screen_height {
        bar_y = rect.top - bar_h - 8;
    }

    let mut start_x = rect.right - total_w;
    if start_x < rect.left {
        start_x = rect.left;
    }

    state.buttons.clear();
    let mut curr_x = start_x + pad;
    for (act, icon, tip) in main_actions.iter() {
        let is_act = match (act, state.mode) {
            (Action::DrawPen, AppMode::DrawingPen) => true,
            (Action::DrawRect, AppMode::DrawingRect) => true,
            (Action::DrawArrow, AppMode::DrawingArrow) => true,
            (Action::DrawEllipse, AppMode::DrawingEllipse) => true,
            (Action::DrawText, AppMode::DrawingText) => true,
            (Action::Mosaic, AppMode::DrawingMosaic) => true,
            (Action::Eraser, AppMode::DrawingPen) if state.is_eraser => true,
            _ => false,
        };

        state.buttons.push(ToolbarButton {
            rect: RECT {
                left: curr_x,
                top: bar_y + (bar_h - btn_h) / 2,
                right: curr_x + btn_w,
                bottom: bar_y + (bar_h + btn_h) / 2,
            },
            text: to_wstring(icon),
            action: *act,
            tooltip: tip.to_string(),
            is_active: is_act,
        });
        curr_x += btn_w + pad;
    }

    // 3. 如果处在绘画/标注模式，展开调色板与粗细二级菜单
    let is_drawing_mode = matches!(
        state.mode,
        AppMode::DrawingPen
            | AppMode::DrawingRect
            | AppMode::DrawingArrow
            | AppMode::DrawingEllipse
            | AppMode::DrawingText
    );

    state.color_buttons.clear();
    state.sub_tool_buttons.clear();

    if is_drawing_mode && !state.is_eraser {
        let color_y = bar_y + bar_h + 4;
        let csize = 20;
        let cpad = 4;
        let mut cx = start_x + pad;

        for c in PREDEFINED_COLORS.iter() {
            state.color_buttons.push(ToolbarButton {
                rect: RECT {
                    left: cx,
                    top: color_y,
                    right: cx + csize,
                    bottom: color_y + csize,
                },
                text: vec![],
                action: Action::ColorPick(*c),
                tooltip: "颜色".to_string(),
                is_active: state.current_color == *c,
            });
            cx += csize + cpad;
        }

        // 粗细调节
        cx += 12;
        let thicknesses = [(2, "细"), (4, "中"), (8, "粗")];
        for (th, name) in thicknesses.iter() {
            state.sub_tool_buttons.push(ToolbarButton {
                rect: RECT {
                    left: cx,
                    top: color_y,
                    right: cx + 28,
                    bottom: color_y + csize,
                },
                text: to_wstring(name),
                action: Action::Thickness(*th),
                tooltip: format!("粗细: {}px", th),
                is_active: state.current_thickness == *th,
            });
            cx += 32;
        }
    }
}

/// 绘制工具栏到 GDI 内存 DC
pub unsafe fn draw_toolbar(dc: HDC, state: &AppState) {
    // 1. 绘制录屏状态悬浮条
    if state.mode == AppMode::Recording {
        if let Some(first) = state.record_buttons.first() {
            let last = state.record_buttons.last().unwrap();
            let bar_rect = RECT {
                left: first.rect.left - 128,
                top: first.rect.top - 4,
                right: last.rect.right + 8,
                bottom: last.rect.bottom + 4,
            };

            let bg = CreateSolidBrush(COLORREF(0x00222222));
            FillRect(dc, &bar_rect, bg);
            let _ = DeleteObject(bg.into());
            draw_border(dc, bar_rect, COLORREF(0x00444444), 1);

            // 绘制录制动态时长文字 (REC 00:15)
            let secs = state.recording_session.as_ref().map(|s| s.elapsed_secs).unwrap_or(0);
            let time_str = format!("● REC  {:02}:{:02}", secs / 60, secs % 60);
            let mut wtime = to_wstring(&time_str);
            let mut time_rect = RECT {
                left: bar_rect.left + 12,
                top: bar_rect.top,
                right: first.rect.left - 8,
                bottom: bar_rect.bottom,
            };

            let mut font_name = to_wstring("Microsoft YaHei");
            font_name.push(0);
            let hfont = CreateFontW(
                16, 0, 0, 0, 700, 0, 0, 0, DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0,
                PCWSTR(font_name.as_ptr()),
            );
            let old_font = SelectObject(dc, hfont.into());
            let old_bk = SetBkMode(dc, TRANSPARENT);
            let old_color = SetTextColor(dc, COLORREF(0x000000FF)); // 亮红录制灯文字

            let _ = DrawTextW(dc, &mut wtime, &mut time_rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

            // 绘制录制按钮
            for btn in &state.record_buttons {
                draw_single_button(dc, btn, state);
            }

            SetTextColor(dc, old_color);
            SetBkMode(dc, windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
            SelectObject(dc, old_font);
            let _ = DeleteObject(hfont.into());
        }
        return;
    }

    // 2. 绘制普通截图主工具条
    if !state.buttons.is_empty() {
        let first = state.buttons.first().unwrap();
        let last = state.buttons.last().unwrap();
        let bar_rect = RECT {
            left: first.rect.left - 8,
            top: first.rect.top - 4,
            right: last.rect.right + 8,
            bottom: last.rect.bottom + 4,
        };

        let bg = CreateSolidBrush(COLORREF(0x00FFFFFF));
        FillRect(dc, &bar_rect, bg);
        let _ = DeleteObject(bg.into());
        draw_border(dc, bar_rect, COLORREF(0x00E0E0E0), 1);

        for btn in &state.buttons {
            draw_single_button(dc, btn, state);
        }

        // 3. 绘制调色盘
        if !state.color_buttons.is_empty() {
            let c_first = state.color_buttons.first().unwrap();
            let c_last = state.sub_tool_buttons.last().unwrap_or(state.color_buttons.last().unwrap());
            let sub_rect = RECT {
                left: c_first.rect.left - 8,
                top: c_first.rect.top - 4,
                right: c_last.rect.right + 8,
                bottom: c_first.rect.bottom + 4,
            };
            let sub_bg = CreateSolidBrush(COLORREF(0x00FFFFFF));
            FillRect(dc, &sub_rect, sub_bg);
            let _ = DeleteObject(sub_bg.into());
            draw_border(dc, sub_rect, COLORREF(0x00E0E0E0), 1);

            for btn in &state.color_buttons {
                let color = btn.action.match_color().unwrap_or(0xFFFFFF);
                let brush = CreateSolidBrush(COLORREF(color));
                FillRect(dc, &btn.rect, brush);
                let _ = DeleteObject(brush.into());

                if btn.is_active || state.hovered_action == Some(btn.action) {
                    draw_border(dc, btn.rect, COLORREF(0x00FF8800), 2);
                } else {
                    draw_border(dc, btn.rect, COLORREF(0x00D0D0D0), 1);
                }
            }

            for btn in &state.sub_tool_buttons {
                draw_single_button(dc, btn, state);
            }
        }
    }

    // 4. 如果处于 Loading 状态 (例如 OCR 或翻译正在请求大模型)，在截取选区中心绘制 Loading 遮罩
    if !state.loading_text.is_empty() {
        if let Some(r) = state.crop_rect {
            let mut wtext = to_wstring(&state.loading_text);
            let cx = (r.left + r.right) / 2;
            let cy = (r.top + r.bottom) / 2;
            let tip_rect = RECT {
                left: cx - 100,
                top: cy - 20,
                right: cx + 100,
                bottom: cy + 20,
            };
            let bg = CreateSolidBrush(COLORREF(0x00111111));
            FillRect(dc, &tip_rect, bg);
            let _ = DeleteObject(bg.into());
            draw_border(dc, tip_rect, COLORREF(0x00555555), 1);

            let old_bk = SetBkMode(dc, TRANSPARENT);
            let old_color = SetTextColor(dc, COLORREF(0x00FFFFFF));
            let _ = DrawTextW(dc, &mut wtext, &mut tip_rect.clone(), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            SetTextColor(dc, old_color);
            SetBkMode(dc, windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
        }
    }
}

/// 绘制单个按扭
unsafe fn draw_single_button(dc: HDC, btn: &ToolbarButton, state: &AppState) {
    let mut bg_color = COLORREF(0x00FFFFFF);
    if btn.is_active {
        bg_color = COLORREF(0x00E5E5E5); // 激活深灰
    } else if state.hovered_action == Some(btn.action) {
        bg_color = COLORREF(0x00F4F4F4); // 悬停浅灰
    }

    let btn_brush = CreateSolidBrush(bg_color);
    FillRect(dc, &btn.rect, btn_brush);
    let _ = DeleteObject(btn_brush.into());

    let mut font_name = to_wstring("Segoe MDL2 Assets");
    font_name.push(0);
    let hfont = CreateFontW(
        18, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0,
        PCWSTR(font_name.as_ptr()),
    );
    let old_font = SelectObject(dc, hfont.into());
    let old_bk = SetBkMode(dc, TRANSPARENT);
    let text_color = if btn.is_active {
        COLORREF(0x00D9531E) // 激活高亮主题色
    } else {
        COLORREF(0x00333333)
    };
    let old_color = SetTextColor(dc, text_color);

    let mut wtext = btn.text.clone();
    let mut btn_rect = btn.rect;
    let _ = DrawTextW(dc, &mut wtext, &mut btn_rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    SetTextColor(dc, old_color);
    SetBkMode(dc, windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
    SelectObject(dc, old_font);
    let _ = DeleteObject(hfont.into());
}
