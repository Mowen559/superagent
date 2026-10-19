//! Git Hunk 坐标对齐与代码行号自愈重定位 (Diff Relocation & Anchor Repair)
//!
//! 借鉴 Alibaba open-code-review 生产级架构设计：
//! 1. 解析 Unified Diff 中的 Hunk 块，计算增删改造成的坐标位移（Relocation）；
//! 2. 实现弹性锚点对齐器 (AnchorAligner)，在代码替换时智能容忍换行符 (CRLF/LF)、
//!    行首/行尾空白微差及代码行号微幅漂移，消除大模型调用 replace 时的文本硬匹配失败。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub header: String,
}

impl DiffHunk {
    /// 计算该 Hunk 对后续行号的位移增量 (new_lines - old_lines)
    pub fn line_delta(&self) -> isize {
        self.new_lines as isize - self.old_lines as isize
    }
}

/// 解析 Unified Diff 文本中的所有 @@ ... @@ hunk 块
pub fn parse_unified_diff_hunks(diff: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    for line in diff.lines() {
        if line.starts_with("@@ ") {
            if let Some(hunk) = parse_hunk_header(line) {
                hunks.push(hunk);
            }
        }
    }
    hunks
}

fn parse_hunk_header(header_line: &str) -> Option<DiffHunk> {
    // 格式如: @@ -10,5 +12,7 @@ optional header
    let parts: Vec<&str> = header_line.split("@@").collect();
    if parts.len() < 3 {
        return None;
    }
    let range_str = parts[1].trim();
    let mut range_parts = range_str.split_whitespace();
    let old_part = range_parts.next()?;
    let new_part = range_parts.next()?;

    let (old_start, old_lines) = parse_range_spec(old_part.strip_prefix('-')?)?;
    let (new_start, new_lines) = parse_range_spec(new_part.strip_prefix('+')?)?;

    Some(DiffHunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        header: header_line.to_string(),
    })
}

fn parse_range_spec(spec: &str) -> Option<(usize, usize)> {
    if let Some((start, lines)) = spec.split_once(',') {
        let s = start.parse::<usize>().ok()?;
        let l = lines.parse::<usize>().ok()?;
        Some((s, l))
    } else {
        let s = spec.parse::<usize>().ok()?;
        Some((s, 1))
    }
}

/// 根据历史 Hunk 变更位移，将原始文件的旧行号重定位到新文件行号
pub fn relocate_line_number(hunks: &[DiffHunk], old_line: usize) -> usize {
    let mut current_offset: isize = 0;
    for hunk in hunks {
        if old_line < hunk.old_start {
            // 在当前 hunk 之前，不受后续偏移影响
            break;
        } else if old_line >= hunk.old_start && old_line < hunk.old_start + hunk.old_lines {
            // 正好处于当前被修改/替换的 Hunk 内部
            // 映射到 new_start 对应的相对比例位置
            let relative = old_line - hunk.old_start;
            let target = (hunk.new_start as isize + relative as isize + current_offset).max(1);
            return target as usize;
        } else {
            // 在当前 hunk 之后，累计增量位移
            current_offset += hunk.line_delta();
        }
    }
    let relocated = (old_line as isize + current_offset).max(1);
    relocated as usize
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignmentResult {
    pub start_byte: usize,
    pub end_byte: usize,
    pub matched_text: String,
    pub repaired: bool,
    pub repair_strategy: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosestCandidate {
    pub start_line: usize,
    pub end_line: usize,
    pub snippet: String,
    pub similarity: f32,
}

/// 弹性锚点对齐器：解决大模型输出代码块时的缩进微差、换行符差异和轻度漂移
pub struct AnchorAligner;

impl AnchorAligner {
    /// 寻找文件中与 needle 最相似的代码块候选（供 SWE-agent 风格的诊断引导使用）
    pub fn find_closest_candidate(haystack: &str, needle: &str) -> Option<ClosestCandidate> {
        let needle_lines: Vec<&str> = needle.lines().map(str::trim).filter(|s| !s.is_empty()).collect();
        if needle_lines.is_empty() {
            return None;
        }

        let haystack_lines: Vec<&str> = haystack.lines().collect();
        if haystack_lines.is_empty() {
            return None;
        }

        let n_len = needle_lines.len().min(haystack_lines.len());
        let mut best: Option<(usize, usize, f32)> = None;

        for i in 0..=(haystack_lines.len().saturating_sub(n_len)) {
            let mut match_score = 0.0f32;
            for j in 0..n_len {
                let h_trim = haystack_lines[i + j].trim();
                let n_trim = needle_lines[j];
                if h_trim == n_trim {
                    match_score += 1.0;
                } else if !h_trim.is_empty() && !n_trim.is_empty() {
                    if h_trim.contains(n_trim) || n_trim.contains(h_trim) {
                        match_score += 0.8;
                    } else {
                        let sim = simple_token_similarity(h_trim, n_trim);
                        match_score += sim;
                    }
                }
            }

            let avg_sim = match_score / (n_len as f32);
            if avg_sim > 0.15 {
                match best {
                    None => best = Some((i, i + n_len, avg_sim)),
                    Some((_, _, prev_sim)) => {
                        if avg_sim > prev_sim {
                            best = Some((i, i + n_len, avg_sim));
                        }
                    }
                }
            }
        }

        if let Some((start_line_idx, end_line_idx, sim)) = best {
            let snippet = haystack_lines[start_line_idx..end_line_idx].join("\n");
            return Some(ClosestCandidate {
                start_line: start_line_idx + 1,
                end_line: end_line_idx,
                snippet,
                similarity: sim,
            });
        }

        None
    }

    /// 在 haystack 中搜索 needle，如果精确匹配失败，则使用降级容错算法定位唯一的对应代码块
    pub fn align(haystack: &str, needle: &str) -> Option<AlignmentResult> {
        if needle.is_empty() {
            return None;
        }

        // 策略 1: 精确匹配 (Exact Match)
        if let Some(start_byte) = haystack.find(needle) {
            let end_byte = start_byte + needle.len();
            return Some(AlignmentResult {
                start_byte,
                end_byte,
                matched_text: needle.to_string(),
                repaired: false,
                repair_strategy: "exact".to_string(),
                confidence: 1.0,
            });
        }

        // 策略 2: 换行符归一化 (Normalize CRLF <-> LF)
        if let Some(res) = Self::align_normalized_newlines(haystack, needle) {
            return Some(res);
        }

        // 策略 3: 行级首尾空白微差容忍 (Line-trimmed Matching)
        if let Some(res) = Self::align_line_trimmed(haystack, needle) {
            return Some(res);
        }

        // 策略 4: 滑动窗口子序列相似度匹配 (Sliding Window Fuzzy)
        Self::align_sliding_window_fuzzy(haystack, needle)
    }

    /// 策略 2 实现：统一归一化换行符
    fn align_normalized_newlines(haystack: &str, needle: &str) -> Option<AlignmentResult> {
        let norm_haystack = haystack.replace("\r\n", "\n");
        let norm_needle = needle.replace("\r\n", "\n");

        if let Some(norm_idx) = norm_haystack.find(&norm_needle) {
            // 映射回原始 haystack 的字节跨度
            let orig_start = map_normalized_index_to_original(haystack, norm_idx)?;
            let norm_end = norm_idx + norm_needle.len();
            let orig_end = map_normalized_index_to_original(haystack, norm_end)?;

            let matched_slice = &haystack[orig_start..orig_end];
            return Some(AlignmentResult {
                start_byte: orig_start,
                end_byte: orig_end,
                matched_text: matched_slice.to_string(),
                repaired: true,
                repair_strategy: "newline_normalized".to_string(),
                confidence: 0.99,
            });
        }
        None
    }

    /// 策略 3 实现：行级别去除两端空白后进行逐行匹配
    fn align_line_trimmed(haystack: &str, needle: &str) -> Option<AlignmentResult> {
        let needle_lines: Vec<&str> = needle.lines().map(str::trim).filter(|s| !s.is_empty()).collect();
        if needle_lines.is_empty() {
            return None;
        }

        let haystack_lines_raw: Vec<&str> = haystack.lines().collect();
        let haystack_lines_trimmed: Vec<&str> = haystack_lines_raw.iter().map(|s| s.trim()).collect();

        let n_len = needle_lines.len();
        if n_len > haystack_lines_trimmed.len() {
            return None;
        }

        let mut matches = Vec::new();
        for i in 0..=(haystack_lines_trimmed.len() - n_len) {
            let window = &haystack_lines_trimmed[i..i + n_len];
            if window == needle_lines.as_slice() {
                matches.push((i, i + n_len));
            }
        }

        // 仅当唯一匹配时，安全应用自愈，防止多重歧义
        if matches.len() == 1 {
            let (start_line_idx, end_line_idx) = matches[0];
            let (start_byte, end_byte) = lines_to_byte_range(haystack, start_line_idx, end_line_idx)?;
            let matched_text = haystack[start_byte..end_byte].to_string();

            return Some(AlignmentResult {
                start_byte,
                end_byte,
                matched_text,
                repaired: true,
                repair_strategy: "line_trimmed_unique".to_string(),
                confidence: 0.95,
            });
        }

        None
    }

    /// 策略 4 实现：滑动窗口相似度匹配（针对少量字符变动或行内注释轻微差异）
    fn align_sliding_window_fuzzy(haystack: &str, needle: &str) -> Option<AlignmentResult> {
        let needle_lines: Vec<&str> = needle.lines().map(str::trim).filter(|s| !s.is_empty()).collect();
        if needle_lines.is_empty() {
            return None;
        }

        let haystack_lines: Vec<&str> = haystack.lines().collect();
        let n_len = needle_lines.len();
        if n_len > haystack_lines.len() {
            return None;
        }

        let mut best_match: Option<(usize, usize, f32)> = None;

        for i in 0..=(haystack_lines.len() - n_len) {
            let mut line_matches = 0;
            for j in 0..n_len {
                let h_trim = haystack_lines[i + j].trim();
                let n_trim = needle_lines[j];
                if h_trim == n_trim {
                    line_matches += 1;
                } else if !h_trim.is_empty() && !n_trim.is_empty() {
                    let sim = simple_jaccard_similarity(h_trim, n_trim);
                    if sim > 0.80 {
                        line_matches += 1;
                    }
                }
            }

            let score = line_matches as f32 / n_len as f32;
            if score >= 0.85 {
                match best_match {
                    None => best_match = Some((i, i + n_len, score)),
                    Some((_, _, prev_score)) => {
                        if score > prev_score {
                            best_match = Some((i, i + n_len, score));
                        } else if (score - prev_score).abs() < 0.01 {
                            // 存在多个同等分数的歧义候选，放弃模糊自愈
                            return None;
                        }
                    }
                }
            }
        }

        if let Some((start_line, end_line, score)) = best_match {
            let (start_byte, end_byte) = lines_to_byte_range(haystack, start_line, end_line)?;
            let matched_text = haystack[start_byte..end_byte].to_string();

            return Some(AlignmentResult {
                start_byte,
                end_byte,
                matched_text,
                repaired: true,
                repair_strategy: "fuzzy_sliding_window".to_string(),
                confidence: score,
            });
        }

        None
    }
}

fn simple_jaccard_similarity(a: &str, b: &str) -> f32 {
    let set_a: std::collections::HashSet<char> = a.chars().collect();
    let set_b: std::collections::HashSet<char> = b.chars().collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        1.0
    } else {
        intersection as f32 / union as f32
    }
}

/// Token 级别的代码相似度（按词切分，精准比对变量名和符号）
fn simple_token_similarity(a: &str, b: &str) -> f32 {
    let tokens_a: std::collections::HashSet<&str> = a
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .collect();
    let tokens_b: std::collections::HashSet<&str> = b
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .collect();
    let intersection = tokens_a.intersection(&tokens_b).count();
    let union = tokens_a.union(&tokens_b).count();
    if union == 0 {
        1.0
    } else {
        intersection as f32 / union as f32
    }
}

fn map_normalized_index_to_original(haystack: &str, norm_target_idx: usize) -> Option<usize> {
    let mut norm_count = 0;
    let mut orig_idx = 0;
    let chars: Vec<char> = haystack.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if norm_count == norm_target_idx {
            return Some(orig_idx);
        }
        if chars[i] == '\r' && i + 1 < chars.len() && chars[i + 1] == '\n' {
            norm_count += 1;
            orig_idx += 2;
            i += 2;
        } else {
            norm_count += chars[i].len_utf8();
            orig_idx += chars[i].len_utf8();
            i += 1;
        }
    }
    if norm_count == norm_target_idx {
        Some(orig_idx)
    } else {
        None
    }
}

fn lines_to_byte_range(haystack: &str, start_line: usize, end_line: usize) -> Option<(usize, usize)> {
    let mut current_line = 0;
    let mut start_byte = None;
    let mut end_byte = None;
    let mut offset = 0;

    for line in haystack.split_inclusive('\n') {
        if current_line == start_line && start_byte.is_none() {
            start_byte = Some(offset);
        }
        offset += line.len();
        if current_line + 1 == end_line {
            end_byte = Some(offset);
            break;
        }
        current_line += 1;
    }

    if start_line == end_line {
        end_byte = start_byte;
    }

    match (start_byte, end_byte) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unified_diff_hunk_and_relocates() {
        let diff = r#"
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,4 +10,6 @@ pub fn test() {
+    // added line 1
+    // added line 2
     let x = 1;
@@ -30,5 +32,5 @@
"#;
        let hunks = parse_unified_diff_hunks(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].old_start, 10);
        assert_eq!(hunks[0].line_delta(), 2);

        // Before hunk 1: line 5 stays line 5
        assert_eq!(relocate_line_number(&hunks, 5), 5);
        // After hunk 1 but before hunk 2: line 20 shifts by +2 to 22
        assert_eq!(relocate_line_number(&hunks, 20), 22);
    }

    #[test]
    fn anchor_aligner_exact_match() {
        let content = "fn hello() {\n    println!(\"world\");\n}\n";
        let search = "println!(\"world\");";
        let res = AnchorAligner::align(content, search).expect("should align");
        assert!(!res.repaired);
        assert_eq!(res.matched_text, search);
        assert_eq!(res.confidence, 1.0);
    }

    #[test]
    fn anchor_aligner_crlf_repair() {
        let content = "fn hello() {\r\n    println!(\"world\");\r\n}\r\n";
        let search = "    println!(\"world\");\n}";
        let res = AnchorAligner::align(content, search).expect("should align crlf");
        assert!(res.repaired);
        assert_eq!(res.repair_strategy, "newline_normalized");
    }

    #[test]
    fn anchor_aligner_trimmed_indentation_repair() {
        let content = "class Service {\n        def execute():\n            return True\n}\n";
        // LLM provided less indentation spaces
        let search = "    def execute():\n        return True";
        let res = AnchorAligner::align(content, search).expect("should align trimmed");
        assert!(res.repaired);
        assert_eq!(res.repair_strategy, "line_trimmed_unique");
        assert!(res.matched_text.contains("def execute():"));
    }

    #[test]
    fn anchor_aligner_closest_candidate_hint() {
        let content = "line 1\nline 2: fn process_order(id: u64) -> Result<()> {\nline 3:     let valid = true;\nline 4: }\nline 5\n";
        // LLM hallucinated slightly different signature
        let search = "fn process_order(order_id: u64) {\n    let valid = true;\n}";
        let candidate = AnchorAligner::find_closest_candidate(content, search).expect("should find closest candidate");
        assert!(candidate.snippet.contains("process_order"));
        assert!(candidate.start_line >= 2);
    }
}
