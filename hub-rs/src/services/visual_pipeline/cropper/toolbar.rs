use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, SelectObject,
    SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, DT_LEFT, DT_TOP, DT_WORDBREAK, OUT_DEFAULT_PRECIS, TRANSPARENT,
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

    let screen_height = state.frame_height as i32;
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
    state.record_buttons.clear();
    let mut main_actions = vec![
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
        (Action::DrawLine, "\u{E738}", "直线"),
        (Action::CopyText, "文", "复制选中文字 (Ctrl+C)"),
        (Action::Save, "\u{E74E}", "保存图片 (Ctrl+S)"),
    ];

    if state.last_saved_path.is_some() {
        main_actions.push((Action::OpenFolder,"\u{E8B7}","打开保存位置"));
        main_actions.push((Action::CopyPath,"路径","复制保存路径"));
    }
    if state.mode == AppMode::Saved {
        main_actions.retain(|(action,_,_)| matches!(action,Action::OpenFolder|Action::CopyPath|Action::Cancel));
    }
    let columns=((state.frame_width as i32-16)/(btn_w+pad)).max(1).min(main_actions.len() as i32);
    let rows=(main_actions.len() as i32+columns-1)/columns;
    let total_w = (btn_w * columns) + (pad * (columns + 1));
    let mut bar_y = rect.bottom + 8;
    if bar_y + bar_h*rows > screen_height { bar_y = rect.top - bar_h*rows - 8; }
    bar_y=bar_y.clamp(4,(screen_height-bar_h*rows-4).max(4));
    let start_x=(rect.right-total_w).clamp(8,(state.frame_width as i32-total_w-8).max(8));

    state.buttons.clear();
    let mut curr_x = start_x + pad;
    for (index,(act, icon, tip)) in main_actions.iter().enumerate() {
        let row=index as i32/columns;
        curr_x=start_x+pad+(index as i32%columns)*(btn_w+pad);
        let is_act = match (act, state.mode) {
            (Action::DrawPen, AppMode::DrawingPen) => !state.is_eraser,
            (Action::DrawLine, AppMode::DrawingLine) => true,
            (Action::Ocr, AppMode::SelectingText) => state.translated_lines.is_none(),
            (Action::Translate, AppMode::SelectingText) => state.translated_lines.is_some(),
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
                top: bar_y + row*bar_h + (bar_h - btn_h) / 2,
                right: curr_x + btn_w,
                bottom: bar_y + row*bar_h + (bar_h + btn_h) / 2,
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
            | AppMode::DrawingLine
            | AppMode::DrawingRect
            | AppMode::DrawingArrow
            | AppMode::DrawingEllipse
            | AppMode::DrawingText
    );

    state.color_buttons.clear();
    state.sub_tool_buttons.clear();

    if is_drawing_mode && !state.is_eraser {
        let color_y = if bar_y+bar_h*rows+32<screen_height {bar_y+bar_h*rows+4} else {(bar_y-32).max(0)};
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

    if state.mode != AppMode::Recording {
        let mut hovered_btn_rect = None;
        let tooltip = state.hovered_action.and_then(|action| state.buttons.iter().chain(state.color_buttons.iter()).chain(state.sub_tool_buttons.iter())
            .find(|b| b.action==action)).map(|b| { hovered_btn_rect = Some(b.rect); b.tooltip.as_str() });
        
        let mut message = String::new();
        if let Some(t) = tooltip { message.push_str(t); }
        if !state.status_text.is_empty() && (state.analysis_job.is_some() || state.record_job.is_some() || tooltip.is_none()) {
            if !message.is_empty() { message.push_str(" | "); }
            message.push_str(&state.status_text);
        }
        if message.is_empty() { return; }
        let message = message.as_str();

        let font = windows::Win32::Graphics::Gdi::CreateFontW(
            15,0,0,0,400,0,0,0,
            windows::Win32::Graphics::Gdi::DEFAULT_CHARSET,
            windows::Win32::Graphics::Gdi::OUT_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLEARTYPE_QUALITY,
            0,windows::core::w!("Microsoft YaHei")
        );
        let old_font = windows::Win32::Graphics::Gdi::SelectObject(dc,font.into());
        
        let mut text = to_wstring(message);
        let max_width = 400;
        let mut text_rect = RECT { left: 0, top: 0, right: max_width, bottom: 0 };
        let _ = windows::Win32::Graphics::Gdi::DrawTextW(
            dc, &mut text, &mut text_rect,
            windows::Win32::Graphics::Gdi::DT_CALCRECT | windows::Win32::Graphics::Gdi::DT_LEFT | windows::Win32::Graphics::Gdi::DT_TOP | windows::Win32::Graphics::Gdi::DT_WORDBREAK
        );
        
        let padding_x = 12;
        let padding_y = 8;
        let box_width = text_rect.right - text_rect.left + padding_x * 2;
        let box_height = text_rect.bottom - text_rect.top + padding_y * 2;
        
        let (mut left, mut top) = if let Some(r) = hovered_btn_rect {
            (r.left - padding_x / 2, r.bottom + 10)
        } else {
            let controls_bottom = state.buttons.iter().chain(state.color_buttons.iter()).chain(state.sub_tool_buttons.iter()).map(|b|b.rect.bottom).max().unwrap_or(40);
            let fallback_left = state.buttons.first().map(|b|b.rect.left).unwrap_or(8);
            (fallback_left, controls_bottom + 12)
        };
        
        left = left.clamp(8, (state.frame_width as i32 - box_width - 8).max(8));
        top = top.clamp(4, (state.frame_height as i32 - box_height - 8).max(4));
        
        let rect = RECT { left, top, right: left + box_width, bottom: top + box_height };
        
        let bg = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(0x002B2B2B));
        let pen = windows::Win32::Graphics::Gdi::CreatePen(windows::Win32::Graphics::Gdi::PS_SOLID, 1, COLORREF(0x00444444));
        let old_bg = windows::Win32::Graphics::Gdi::SelectObject(dc, bg.into());
        let old_pen = windows::Win32::Graphics::Gdi::SelectObject(dc, pen.into());
        let _ = windows::Win32::Graphics::Gdi::RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, 8, 8);
        windows::Win32::Graphics::Gdi::SelectObject(dc, old_bg);
        windows::Win32::Graphics::Gdi::SelectObject(dc, old_pen);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bg.into());
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(pen.into());
        
        let old_bk = windows::Win32::Graphics::Gdi::SetBkMode(dc, windows::Win32::Graphics::Gdi::TRANSPARENT);
        let old_color = windows::Win32::Graphics::Gdi::SetTextColor(dc, COLORREF(0x00FFFFFF));
        
        let mut draw_rect = RECT { left: left + padding_x, top: top + padding_y, right: rect.right - padding_x, bottom: rect.bottom - padding_y };
        let _ = windows::Win32::Graphics::Gdi::DrawTextW(dc, &mut text, &mut draw_rect, windows::Win32::Graphics::Gdi::DT_LEFT | windows::Win32::Graphics::Gdi::DT_TOP | windows::Win32::Graphics::Gdi::DT_WORDBREAK);
        
        windows::Win32::Graphics::Gdi::SetTextColor(dc, old_color);
        windows::Win32::Graphics::Gdi::SetBkMode(dc, windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
        windows::Win32::Graphics::Gdi::SelectObject(dc, old_font);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(font.into());
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

    let mut font_name = to_wstring(if matches!(btn.action,Action::Ocr|Action::CopyText|Action::CopyPath) {"Microsoft YaHei"} else {"Segoe MDL2 Assets"});
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
