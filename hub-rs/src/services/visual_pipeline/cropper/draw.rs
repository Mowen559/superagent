use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, POINT, RECT};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::Gdi::{
    BeginPath, CreateFontW, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject,
    DrawTextW, EndPath, ExtCreatePen, FillRect, GetStockObject, LineTo, MoveToEx,
    PathToRegion, Rectangle, SelectClipRgn, SelectObject, SetBkMode, SetPolyFillMode,
    SetTextColor, StretchDIBits, BITMAPINFO, BS_SOLID, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_SINGLELINE, DT_TOP, DT_VCENTER,
    DT_WORDBREAK, DT_CALCRECT, HOLLOW_BRUSH, LOGBRUSH, OUT_DEFAULT_PRECIS, PEN_STYLE, PS_ENDCAP_ROUND,
    PS_GEOMETRIC, PS_JOIN_ROUND, PS_SOLID, SRCCOPY, TRANSPARENT, WidenPath, WINDING,
};

use super::types::{
    AppState, ArrowData, DrawElement, EllipseData, MosaicData, OcrBox, RectData,
    StrokeData, TextData, TranslatedBox,
};

/// 绘制所有已确认的涂鸦/标注元素
pub unsafe fn draw_elements(dc: HDC, state: &AppState, bmi: &BITMAPINFO) {
    let mut elements_to_draw = state.elements.clone();
    
    // 如果当前正在画笔涂鸦
    if let Some(pts) = &state.current_stroke {
        elements_to_draw.push(DrawElement::Stroke(StrokeData {
            points: pts.clone(),
            color: state.current_color,
            thickness: state.current_thickness,
            is_eraser: state.is_eraser,
        }));
    }

    SetPolyFillMode(dc, WINDING);

    for el in &elements_to_draw {
        match el {
            DrawElement::Stroke(s) => draw_stroke(dc, s, state, bmi),
            DrawElement::Rect(r) => draw_rect_element(dc, r),
            DrawElement::Arrow(a) => draw_arrow_element(dc, a),
            DrawElement::Ellipse(e) => draw_ellipse_element(dc, e),
            DrawElement::Text(t) => draw_text_element(dc, t),
            DrawElement::Mosaic(m) => draw_mosaic_element(dc, m, state, bmi),
        }
    }
}

/// 绘制正在拖拽中的临时图形
pub unsafe fn draw_in_progress(dc: HDC, state: &AppState) {
    if !state.is_dragging {
        return;
    }
    let left = std::cmp::min(state.start_pt.x, state.current_pt.x);
    let top = std::cmp::min(state.start_pt.y, state.current_pt.y);
    let right = std::cmp::max(state.start_pt.x, state.current_pt.x);
    let bottom = std::cmp::max(state.start_pt.y, state.current_pt.y);

    match state.mode {
        super::types::AppMode::DrawingLine => {
            draw_stroke(dc, &StrokeData { points:vec![state.start_pt,state.current_pt],
                color:state.current_color,thickness:state.current_thickness,is_eraser:false },state,&BITMAPINFO::default());
        }
        super::types::AppMode::DrawingRect => {
            draw_rect_element(dc, &RectData {
                rect: RECT { left, top, right, bottom },
                color: state.current_color,
                thickness: state.current_thickness,
            });
        }
        super::types::AppMode::DrawingArrow => {
            draw_arrow_element(dc, &ArrowData {
                start: state.start_pt,
                end: state.current_pt,
                color: state.current_color,
                thickness: state.current_thickness,
            });
        }
        super::types::AppMode::DrawingEllipse => {
            draw_ellipse_element(dc, &EllipseData {
                rect: RECT { left, top, right, bottom },
                color: state.current_color,
                thickness: state.current_thickness,
            });
        }
        super::types::AppMode::DrawingMosaic => {
            draw_border(dc, RECT { left, top, right, bottom }, COLORREF(0x00A0A0A0), 1);
        }
        _ => {}
    }
}

/// 绘制自由笔画/橡皮擦
unsafe fn draw_stroke(dc: HDC, s: &StrokeData, state: &AppState, bmi: &BITMAPINFO) {
    if s.points.is_empty() {
        return;
    }
    let log_brush = LOGBRUSH {
        lbStyle: BS_SOLID,
        lbColor: COLORREF(s.color),
        lbHatch: 0,
    };
    let stroke_pen = ExtCreatePen(
        PEN_STYLE(PS_GEOMETRIC.0 | PS_SOLID.0 | PS_ENDCAP_ROUND.0 | PS_JOIN_ROUND.0),
        s.thickness as u32,
        &log_brush,
        None,
    );
    let old_pen = SelectObject(dc, stroke_pen.into());

    if s.is_eraser {
        let _ = BeginPath(dc);
        let _ = MoveToEx(dc, s.points[0].x, s.points[0].y, None);
        for pt in s.points.iter().skip(1) {
            let _ = LineTo(dc, pt.x, pt.y);
        }
        let _ = EndPath(dc);
        let _ = WidenPath(dc);
        let hrgn = PathToRegion(dc);
        SelectObject(dc, old_pen);
        let _ = DeleteObject(stroke_pen.into());

        if !hrgn.is_invalid() {
            let saved=windows::Win32::Graphics::Gdi::SaveDC(dc);
            let _ = windows::Win32::Graphics::Gdi::ExtSelectClipRgn(dc,Some(hrgn),windows::Win32::Graphics::Gdi::RGN_AND);
            let _ = StretchDIBits(
                dc,
                0, 0, state.frame_width as i32, state.frame_height as i32,
                0, 0, state.frame_width as i32, state.frame_height as i32,
                Some(state.bgra_pixels.as_ptr() as *const _),
                bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            let _ = windows::Win32::Graphics::Gdi::RestoreDC(dc,saved);
            let _ = DeleteObject(hrgn.into());
        }
    } else {
        let _ = MoveToEx(dc, s.points[0].x, s.points[0].y, None);
        for pt in s.points.iter().skip(1) {
            let _ = LineTo(dc, pt.x, pt.y);
        }
        SelectObject(dc, old_pen);
        let _ = DeleteObject(stroke_pen.into());
    }
}

/// 绘制空心线框矩形
pub unsafe fn draw_rect_element(dc: HDC, r: &RectData) {
    let pen = CreatePen(PS_SOLID, r.thickness, COLORREF(r.color));
    let old_pen = SelectObject(dc, pen.into());
    let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
    let _ = Rectangle(dc, r.rect.left, r.rect.top, r.rect.right, r.rect.bottom);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    let _ = DeleteObject(pen.into());
}

/// 绘制方向箭头
pub unsafe fn draw_arrow_element(dc: HDC, a: &ArrowData) {
    let dx = (a.end.x - a.start.x) as f64;
    let dy = (a.end.y - a.start.y) as f64;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 4.0 {
        return;
    }

    let pen = CreatePen(PS_SOLID, a.thickness, COLORREF(a.color));
    let brush = CreateSolidBrush(COLORREF(a.color));
    let old_pen = SelectObject(dc, pen.into());
    let old_brush = SelectObject(dc, brush.into());

    // 主干线
    let _ = MoveToEx(dc, a.start.x, a.start.y, None);
    let _ = LineTo(dc, a.end.x, a.end.y);

    // 箭头头部多边形计算
    let arrow_len = (a.thickness as f64 * 3.5).max(12.0);
    let arrow_width = (a.thickness as f64 * 2.0).max(8.0);

    let ux = dx / len;
    let uy = dy / len;

    let base_x = a.end.x as f64 - arrow_len * ux;
    let base_y = a.end.y as f64 - arrow_len * uy;

    let pt1 = POINT {
        x: (base_x + arrow_width * -uy).round() as i32,
        y: (base_y + arrow_width * ux).round() as i32,
    };
    let pt2 = POINT {
        x: (base_x - arrow_width * -uy).round() as i32,
        y: (base_y - arrow_width * ux).round() as i32,
    };

    let pts = [a.end, pt1, pt2];
    use windows::Win32::Graphics::Gdi::Polygon;
    let _ = Polygon(dc, &pts);

    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    let _ = DeleteObject(brush.into());
    let _ = DeleteObject(pen.into());
}

/// 绘制椭圆
pub unsafe fn draw_ellipse_element(dc: HDC, e: &EllipseData) {
    let pen = CreatePen(PS_SOLID, e.thickness, COLORREF(e.color));
    let old_pen = SelectObject(dc, pen.into());
    let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
    use windows::Win32::Graphics::Gdi::Ellipse;
    let _ = Ellipse(dc, e.rect.left, e.rect.top, e.rect.right, e.rect.bottom);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    let _ = DeleteObject(pen.into());
}

/// 绘制文本标注
pub unsafe fn draw_text_element(dc: HDC, t: &TextData) {
    if t.text.is_empty() {
        return;
    }
    let mut font_name: Vec<u16> = "Microsoft YaHei".encode_utf16().collect();
    font_name.push(0);
    let hfont = CreateFontW(
        t.font_size, 0, 0, 0, 700, 0, 0, 0, DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0,
        PCWSTR(font_name.as_ptr()),
    );
    let old_font = SelectObject(dc, hfont.into());
    let old_bk = SetBkMode(dc, TRANSPARENT);
    let old_color = SetTextColor(dc, COLORREF(t.color));

    let mut wtext: Vec<u16> = t.text.encode_utf16().collect();
    let mut rect = RECT {
        left: t.pos.x,
        top: t.pos.y,
        right: t.pos.x + 800,
        bottom: t.pos.y + 400,
    };
    let _ = DrawTextW(dc, &mut wtext, &mut rect, DT_LEFT | DT_TOP | DT_WORDBREAK);

    SetTextColor(dc, old_color);
    SetBkMode(dc, windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
    SelectObject(dc, old_font);
    let _ = DeleteObject(hfont.into());
}

/// 绘制马赛克模糊区域
pub unsafe fn draw_mosaic_element(dc: HDC, m: &MosaicData, state: &AppState, _bmi: &BITMAPINFO) {
    let block = m.block_size.max(8);
    let l = m.rect.left.max(0);
    let t = m.rect.top.max(0);
    let r = m.rect.right.min(state.frame_width as i32);
    let b = m.rect.bottom.min(state.frame_height as i32);

    if r <= l || b <= t {
        return;
    }

    let stride = state.frame_width as usize * 4;
    for y in (t..b).step_by(block as usize) {
        for x in (l..r).step_by(block as usize) {
            let sample_x = (x + block / 2).min(r - 1) as usize;
            let sample_y = (y + block / 2).min(b - 1) as usize;
            let offset = sample_y * stride + sample_x * 4;
            if offset + 3 < state.bgra_pixels.len() {
                let blue = state.bgra_pixels[offset];
                let green = state.bgra_pixels[offset + 1];
                let red = state.bgra_pixels[offset + 2];
                let color = COLORREF((blue as u32) << 16 | (green as u32) << 8 | (red as u32));
                
                let block_rect = RECT {
                    left: x,
                    top: y,
                    right: (x + block).min(r),
                    bottom: (y + block).min(b),
                };
                let brush = CreateSolidBrush(color);
                FillRect(dc, &block_rect, brush);
                let _ = DeleteObject(brush.into());
            }
        }
    }
}

/// Draw the complete recognized text at its original bounding boxes.
pub unsafe fn draw_ocr_blocks(dc: HDC, state: &AppState) {
    if state.mode != super::types::AppMode::SelectingText { return; }
    if state.translated_lines.is_none() {
        for line in &state.text_lines {
            let [left,top,right,bottom]=line.rect;
            draw_fitted_text(dc,RECT{left,top,right,bottom},&line.text);
        }
    }
    for span in &state.selected_spans {
        if let Some(line)=state.text_lines.get(span.line) {
            let count=line.text.chars().count().max(1) as i32;
            let [left,top,right,bottom]=line.rect;
            let width=right-left;
            let rect=RECT{left:left+width*span.start as i32/count,top,
                right:left+width*span.end as i32/count,bottom};
            draw_border(dc,rect,COLORREF(0x00EE9900),2);
        }
    }
}

pub unsafe fn draw_translated_lines(dc: HDC, state: &AppState) {
    if let Some(lines)=&state.translated_lines {
        for line in lines { draw_fitted_text(dc,line.rect,&line.translated_text); }
    }
}

unsafe fn draw_fitted_text(dc:HDC, rect:RECT, text:&str) {
    if rect.right<=rect.left || rect.bottom<=rect.top {return;}
    let background=CreateSolidBrush(COLORREF(0x00FFFFFF));FillRect(dc,&rect,background);let _=DeleteObject(background.into());
    let old_bk=SetBkMode(dc,TRANSPARENT);let old_color=SetTextColor(dc,COLORREF(0x00202020));
    // Measure the whole string, including wrapped translations; never use SINGLELINE.
    for size in (1..=(rect.bottom-rect.top).min(28).max(1)).rev() {
        let font=CreateFontW(size,0,0,0,400,0,0,0,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,0,windows::core::w!("Microsoft YaHei"));
        let old_font=SelectObject(dc,font.into());
        let mut wide:Vec<u16>=text.encode_utf16().collect();
        let mut measured=RECT{left:0,top:0,right:rect.right-rect.left,bottom:0};
        let _=DrawTextW(dc,&mut wide,&mut measured,DT_LEFT|DT_TOP|DT_WORDBREAK|DT_CALCRECT);
        let fits=measured.bottom<=rect.bottom-rect.top && measured.right<=rect.right-rect.left;
        if fits||size==1 { let mut target=rect;let _=DrawTextW(dc,&mut wide,&mut target,DT_LEFT|DT_TOP|DT_WORDBREAK); }
        SelectObject(dc,old_font);let _=DeleteObject(font.into());
        if fits {break;}
    }
    SetTextColor(dc,old_color);SetBkMode(dc,windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
}

/// 绘制边框工具函数
pub unsafe fn draw_border(dc: HDC, rect: RECT, color: COLORREF, width: i32) {
    let outline_pen = CreatePen(PS_SOLID, width, color);
    let old_pen = SelectObject(dc, outline_pen.into());
    let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
    let _ = Rectangle(dc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    let _ = DeleteObject(outline_pen.into());
}

/// 绘制录屏呼吸边框 (红色与白色交替)
pub unsafe fn draw_recording_border(dc: HDC, rect: RECT, is_blink: bool) {
    let color = if is_blink {
        COLORREF(0x000000FF) // 亮红色
    } else {
        COLORREF(0x008888FF) // 淡红
    };
    draw_border(dc, rect, color, 3);
}

/// 绘制光标取色放大镜 (Magnifier)
pub unsafe fn draw_magnifier(dc: HDC, pt: POINT, state: &AppState) {
    let mag_size = 120;
    let grid_count = 11;
    let cell_size = mag_size / grid_count;
    let half_grid = grid_count / 2;

    let mut mag_x = pt.x + 20;
    let mut mag_y = pt.y + 20;
    if mag_x + mag_size > state.frame_width as i32 {
        mag_x = pt.x - mag_size - 20;
    }
    if mag_y + mag_size + 40 > state.frame_height as i32 {
        mag_y = pt.y - mag_size - 40;
    }

    let frame_rect = RECT {
        left: mag_x - 2,
        top: mag_y - 2,
        right: mag_x + mag_size + 2,
        bottom: mag_y + mag_size + 36,
    };

    let bg_brush = CreateSolidBrush(COLORREF(0x001A1A1A));
    FillRect(dc, &frame_rect, bg_brush);
    let _ = DeleteObject(bg_brush.into());
    draw_border(dc, frame_rect, COLORREF(0x00555555), 1);

    let stride = state.frame_width as usize * 4;
    let mut center_rgb = (0u8, 0u8, 0u8);

    for gy in 0..grid_count {
        for gx in 0..grid_count {
            let sx = (pt.x - half_grid + gx).max(0).min(state.frame_width as i32 - 1) as usize;
            let sy = (pt.y - half_grid + gy).max(0).min(state.frame_height as i32 - 1) as usize;
            let offset = sy * stride + sx * 4;

            if offset + 3 < state.bgra_pixels.len() {
                let b = state.bgra_pixels[offset];
                let g = state.bgra_pixels[offset + 1];
                let r = state.bgra_pixels[offset + 2];
                if gx == half_grid && gy == half_grid {
                    center_rgb = (r, g, b);
                }

                let cell_rect = RECT {
                    left: mag_x + gx * cell_size,
                    top: mag_y + gy * cell_size,
                    right: mag_x + (gx + 1) * cell_size,
                    bottom: mag_y + (gy + 1) * cell_size,
                };
                let cell_brush = CreateSolidBrush(COLORREF((b as u32) << 16 | (g as u32) << 8 | (r as u32)));
                FillRect(dc, &cell_rect, cell_brush);
                let _ = DeleteObject(cell_brush.into());
            }
        }
    }

    // 中心十字准星
    let center_cell = RECT {
        left: mag_x + half_grid * cell_size,
        top: mag_y + half_grid * cell_size,
        right: mag_x + (half_grid + 1) * cell_size,
        bottom: mag_y + (half_grid + 1) * cell_size,
    };
    draw_border(dc, center_cell, COLORREF(0x000000FF), 1);

    // 绘制坐标与 RGB 文本信息
    let info_text = format!("RGB: ({}, {}, {})\nPOS: ({}, {})", center_rgb.0, center_rgb.1, center_rgb.2, pt.x, pt.y);
    let mut winfo: Vec<u16> = info_text.encode_utf16().collect();
    let mut text_rect = RECT {
        left: mag_x + 2,
        top: mag_y + mag_size + 2,
        right: mag_x + mag_size + 2,
        bottom: mag_y + mag_size + 34,
    };
    let old_bk = SetBkMode(dc, TRANSPARENT);
    let old_color = SetTextColor(dc, COLORREF(0x00FFFFFF));
    let _ = DrawTextW(dc, &mut winfo, &mut text_rect, DT_LEFT | DT_TOP);
    SetTextColor(dc, old_color);
    SetBkMode(dc, windows::Win32::Graphics::Gdi::BACKGROUND_MODE(old_bk as u32));
}
