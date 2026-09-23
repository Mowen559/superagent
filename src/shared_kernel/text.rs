//! 通用 UTF-8 安全文本截断与格式化工具

/// 按最大字节数安全截断 UTF-8 字符串切片（不会在多字节字符中间截断导致 panic）
pub fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut idx = max_bytes;
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    &text[..idx]
}

/// 按最大字符数安全截断，超出时追加 `...`
pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

/// 按最大字符数截断（不trim首尾空白），超出时追加自定义后缀
pub fn truncate_with_suffix(value: &str, max_chars: usize, suffix: &str) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars).collect();
    out.push_str(suffix);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_utf8_chinese() {
        let s = "你好世界Hello";
        // "你好" 占 6 字节，传入 7 字节应安全回退到 6 字节边界
        assert_eq!(truncate_utf8(s, 7), "你好");
    }

    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("  abcdef  ", 4), "abcd...");
        assert_eq!(truncate_chars("abc", 4), "abc");
    }
}
