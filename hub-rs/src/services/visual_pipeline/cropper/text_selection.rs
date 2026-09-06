//! Character selection on OCR line boxes. Coordinates are physical pixels.
#[derive(Clone, Debug)]
pub struct TextLine {
    pub rect: [i32; 4],
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedSpan {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

pub fn selection(lines: &[TextLine], start: [i32; 2], end: [i32; 2], all: bool) -> Vec<SelectedSpan> {
    if all {
        return lines.iter().enumerate().map(|(line, text)| SelectedSpan {
            line, start: 0, end: text.text.chars().count(),
        }).collect();
    }
    let nearest = |p: [i32; 2]| lines.iter().enumerate().min_by_key(|(_, line)| {
        let [left, top, right, bottom] = line.rect;
        let dy = (top - p[1]).max(p[1] - bottom).max(0) as i64;
        let dx = (left - p[0]).max(p[0] - right).max(0) as i64;
        dy * 100_000 + dx
    }).map(|(i, _)| i);
    let (Some(a), Some(b)) = (nearest(start), nearest(end)) else { return vec![]; };
    let caret = |index: usize, x: i32| {
        let line = &lines[index];
        let count = line.text.chars().count();
        let width = (line.rect[2] - line.rect[0]).max(1) as f64;
        (((x - line.rect[0]) as f64 / width * count as f64).round() as i64).clamp(0, count as i64) as usize
    };
    let mut first = (a, caret(a, start[0]));
    let mut last = (b, caret(b, end[0]));
    if first > last { std::mem::swap(&mut first, &mut last); }
    (first.0..=last.0).map(|line| SelectedSpan {
        line,
        start: if line == first.0 { first.1 } else { 0 },
        end: if line == last.0 { last.1 } else { lines[line].text.chars().count() },
    }).filter(|span| span.end > span.start).collect()
}

pub fn selected_text(lines: &[TextLine], spans: &[SelectedSpan]) -> String {
    spans.iter().map(|span| lines[span.line].text.chars().skip(span.start)
        .take(span.end - span.start).collect::<String>()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lines() -> Vec<TextLine> {
        vec![TextLine { rect: [100, 20, 200, 40], text: "甲乙丙丁戊".into() },
             TextLine { rect: [100, 70, 200, 90], text: "ABCDE".into() }]
    }
    #[test] fn selects_characters_without_changing_capture_bounds() {
        let text = lines();
        let spans = selection(&text, [120, 30], [180, 30], false);
        assert_eq!(selected_text(&text, &spans), "乙丙丁");
        assert_eq!(text[0].rect, [100, 20, 200, 40]);
    }
    #[test] fn reverse_drag_across_lines_retains_reading_order() {
        let text = lines();
        let spans = selection(&text, [160, 80], [140, 30], false);
        assert_eq!(selected_text(&text, &spans), "丙丁戊\nABC");
    }
    #[test] fn select_all_includes_every_recognized_line() {
        let text = lines();
        assert_eq!(selected_text(&text, &selection(&text, [0,0], [0,0], true)), "甲乙丙丁戊\nABCDE");
    }
    #[test] fn no_text_or_zero_length_drag_is_empty() {
        assert!(selection(&[], [0,0], [0,0], false).is_empty());
        assert!(selection(&lines(), [110,30], [110,30], false).is_empty());
    }
}
