//! Token-killer source compression.
//!
//! The actual filtering algorithm is delegated to the vendored `rtk` crate
//! (`rtk::core::filter`) so hub-rs stays byte-for-byte in sync with upstream
//! rtk instead of maintaining a divergent copy. This module keeps hub's public
//! surface used across the codebase:
//!  - `FilterLevel` with lenient parsing (`min`/`agg`/`max`/`off`/`raw` aliases),
//!  - `Language` with `from_path` (rtk only exposes `from_extension`),
//!  - the `compress()` wrapper returning `CompressionResult` stats that the
//!    understand/reverse routes serialize.

use std::str::FromStr;

/// How hard to compress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterLevel {
    /// Return content unchanged.
    None,
    /// Strip comments + collapse blank lines, keep all code.
    Minimal,
    /// Keep only imports, signatures, types and constants (bodies dropped).
    Aggressive,
}

impl FromStr for FilterLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" | "off" | "raw" => Ok(FilterLevel::None),
            "minimal" | "min" => Ok(FilterLevel::Minimal),
            "aggressive" | "agg" | "max" => Ok(FilterLevel::Aggressive),
            _ => Err(format!("Unknown filter level: {}", s)),
        }
    }
}

impl std::fmt::Display for FilterLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterLevel::None => write!(f, "none"),
            FilterLevel::Minimal => write!(f, "minimal"),
            FilterLevel::Aggressive => write!(f, "aggressive"),
        }
    }
}

/// Source language, used to pick comment syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    C,
    Cpp,
    Java,
    Ruby,
    Shell,
    /// Data formats (JSON, YAML, TOML, XML, CSV, Markdown) — never code-filtered.
    Data,
    Unknown,
}

impl Language {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Language::Rust,
            "py" | "pyw" => Language::Python,
            "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
            "ts" | "tsx" => Language::TypeScript,
            "go" => Language::Go,
            "c" | "h" => Language::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" => Language::Cpp,
            "java" => Language::Java,
            "rb" => Language::Ruby,
            "sh" | "bash" | "zsh" => Language::Shell,
            "json" | "jsonc" | "json5" | "yaml" | "yml" | "toml" | "xml" | "csv" | "tsv"
            | "graphql" | "gql" | "sql" | "md" | "markdown" | "txt" | "env" | "lock" => {
                Language::Data
            }
            _ => Language::Unknown,
        }
    }

    /// Detect language from a full path's extension.
    pub fn from_path(path: &str) -> Self {
        path.rsplit('.')
            .next()
            .filter(|_| path.contains('.'))
            .map(Language::from_extension)
            .unwrap_or(Language::Unknown)
    }
}

// to_rtk_level and to_rtk_lang bridging
fn to_rtk_level(l: FilterLevel) -> rtk::core::filter::FilterLevel {
    match l {
        FilterLevel::None => rtk::core::filter::FilterLevel::None,
        FilterLevel::Minimal => rtk::core::filter::FilterLevel::Minimal,
        FilterLevel::Aggressive => rtk::core::filter::FilterLevel::Aggressive,
    }
}

fn to_rtk_lang(l: &Language) -> rtk::core::filter::Language {
    match l {
        Language::Rust => rtk::core::filter::Language::Rust,
        Language::Python => rtk::core::filter::Language::Python,
        Language::TypeScript => rtk::core::filter::Language::TypeScript,
        Language::JavaScript => rtk::core::filter::Language::JavaScript,
        Language::Go => rtk::core::filter::Language::Go,
        Language::C => rtk::core::filter::Language::C,
        Language::Cpp => rtk::core::filter::Language::Cpp,
        Language::Java => rtk::core::filter::Language::Java,
        Language::Ruby => rtk::core::filter::Language::Ruby,
        Language::Shell => rtk::core::filter::Language::Shell,
        Language::Data => rtk::core::filter::Language::Data,
        Language::Unknown => rtk::core::filter::Language::Unknown,
    }
}

/// Apply a filter level to `content`.
pub fn filter(content: &str, lang: &Language, level: FilterLevel) -> String {
    let rtk_level = to_rtk_level(level);
    let rtk_lang = to_rtk_lang(lang);
    rtk::core::filter::get_filter(rtk_level).filter(content, &rtk_lang)
}

/// Truncate to `max_lines`
#[allow(dead_code)]
pub fn smart_truncate(content: &str, max_lines: usize, lang: &Language) -> String {
    rtk::core::filter::smart_truncate(content, max_lines, &to_rtk_lang(lang))
}

/// Result of compressing a chunk of text.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionResult {
    pub content: String,
    pub level: String,
    pub language: String,
    pub original_bytes: usize,
    pub compressed_bytes: usize,
    pub original_lines: usize,
    pub compressed_lines: usize,
    /// Percentage of bytes removed (0-100).
    pub reduction_percent: f64,
    /// Approximate token estimate (bytes / 4, same heuristic rtk uses).
    pub original_tokens_est: usize,
    pub compressed_tokens_est: usize,
}

/// "Never worse" guard: if the filtered output is empty (filter ate everything)
/// or ended up larger than the original, keep the raw content instead.
fn never_worse(raw: &str, filtered: String) -> String {
    if filtered.trim().is_empty() && !raw.trim().is_empty() {
        return raw.to_string();
    }
    if filtered.len() >= raw.len() {
        return raw.to_string();
    }
    filtered
}

/// Compress `content` for a given path/extension and level, returning the
/// compressed text plus reduction statistics. Guaranteed never to enlarge input.
pub fn compress(content: &str, path_or_ext: &str, level: FilterLevel) -> CompressionResult {
    let lang = Language::from_path(path_or_ext);
    let filtered = never_worse(content, filter(content, &lang, level));

    let original_bytes = content.len();
    let compressed_bytes = filtered.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes - compressed_bytes) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines: content.lines().count(),
        compressed_lines: filtered.lines().count(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: filtered,
        level: level.to_string(),
        language: format!("{:?}", lang).to_lowercase(),
    }
}

/// Helper: Checks if a line is pure terminal/build noise that should be discarded
pub fn is_terminal_noise(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Maven / Gradle download progress
    if trimmed.starts_with("[INFO] Downloading from ")
        || trimmed.starts_with("[INFO] Downloaded from ")
        || trimmed.starts_with("Progress (")
        || trimmed.starts_with("Downloading: ")
        || trimmed.starts_with("Downloaded: ")
        || trimmed.starts_with("Downloading from ")
    {
        return true;
    }

    // NPM / Yarn / PNPM / Pip progress bars
    if trimmed.contains("[===")
        || trimmed.contains("npm http fetch GET")
        || trimmed.contains("fetching packages")
        || trimmed.starts_with("⸨===")
        || (trimmed.starts_with('[') && trimmed.contains("#####"))
        || (trimmed.starts_with('[') && trimmed.contains("░░░"))
    {
        return true;
    }

    // Docker layer downloads
    if (trimmed.contains("Downloading [") || trimmed.contains("Extracting ["))
        && trimmed.contains(']')
    {
        return true;
    }

    // Harmless verbose compiler/build progress lines when there's no error on that line
    if trimmed.starts_with("Compiling ")
        && !trimmed.contains("error")
        && !trimmed.contains("warning")
    {
        return true;
    }

    false
}

/// Helper: Checks if a line is a critical failure anchor
pub fn is_critical_error_anchor(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("caused by:")
        || l.contains("build failure")
        || l.contains("assertion failed")
        || l.contains("exception in thread")
        || l.contains("stack backtrace:")
        || l.contains("traceback (most recent call last):")
        || l.contains("panic:")
        || l.contains("panic at")
        || l.contains("error[e") // rustc error code e.g. error[E0308]
        || l.contains("error ts") // typescript error code e.g. error TS2304
        || l.contains("failures:")
        || l.contains("fatal:")
        || l.contains("syntaxerror:")
        || l.contains("typeerror:")
        || l.contains("referenceerror:")
        || (l.starts_with("error:") || l.starts_with("error :") || l.contains(" error: "))
        || (l.starts_with("failed:") || l.starts_with("fail:"))
        || l.contains("failed (failures=")
        || (l.starts_with("[error]") && !l.contains("0 error"))
}

/// Compress raw terminal output / build logs while preserving error tracebacks, panic messages,
/// compiler warnings and key status lines, while aggressively filtering download and progress bar noise.
pub fn compress_terminal_output(content: &str, max_lines: usize) -> CompressionResult {
    let lines: Vec<&str> = content.lines().collect();
    let original_bytes = content.len();
    let original_lines = lines.len();

    let has_noise = lines.iter().any(|l| is_terminal_noise(l));

    if original_lines <= max_lines && !has_noise {
        return CompressionResult {
            original_lines,
            compressed_lines: original_lines,
            original_bytes,
            compressed_bytes: original_bytes,
            reduction_percent: 0.0,
            original_tokens_est: original_bytes / 4,
            compressed_tokens_est: original_bytes / 4,
            content: content.to_string(),
            level: "smart_log".to_string(),
            language: "log".to_string(),
        };
    }

    let mut keep_mask = vec![false; original_lines];

    // Always keep top N lines (command & initial context) and bottom N lines (summary / exit status),
    // unless the line is pure noise
    let head_count = (max_lines / 4).max(10).min(original_lines);
    let tail_count = (max_lines / 3).max(15).min(original_lines.saturating_sub(head_count));

    for i in 0..head_count {
        if !is_terminal_noise(lines[i]) {
            keep_mask[i] = true;
        }
    }
    for i in (original_lines.saturating_sub(tail_count))..original_lines {
        if !is_terminal_noise(lines[i]) {
            keep_mask[i] = true;
        }
    }

    // Keep error stack lines and context (3 lines before & after for critical errors)
    for (i, line) in lines.iter().enumerate() {
        if is_critical_error_anchor(line) {
            let start = i.saturating_sub(3);
            let end = (i + 3).min(original_lines.saturating_sub(1));
            for j in start..=end {
                keep_mask[j] = true;
            }
        } else {
            let l = line.to_lowercase();
            if l.contains("error") || l.contains("panic") || l.contains("fail")
                || l.contains("exception") || l.contains("traceback")
                || l.contains("warning") || l.contains("at line") || l.contains("-->")
            {
                if i > 0 { keep_mask[i - 1] = true; }
                keep_mask[i] = true;
                if i + 1 < original_lines { keep_mask[i + 1] = true; }
            }
        }
    }

    let mut filtered_lines = Vec::new();
    let mut omitted_count = 0;

    for (i, &keep) in keep_mask.iter().enumerate() {
        if keep {
            if omitted_count > 0 {
                filtered_lines.push(format!("... [RTK SmartLog omitted {} lines] ...", omitted_count));
                omitted_count = 0;
            }
            filtered_lines.push(lines[i].to_string());
        } else {
            omitted_count += 1;
        }
    }
    if omitted_count > 0 {
        filtered_lines.push(format!("... [RTK SmartLog omitted {} lines] ...", omitted_count));
    }

    let filtered = filtered_lines.join("\n");
    let compressed_bytes = filtered.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes.saturating_sub(compressed_bytes)) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines,
        compressed_lines: filtered_lines.len(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: filtered,
        level: "smart_log".to_string(),
        language: "log".to_string(),
    }
}

/// 专用压缩器: Git Status 结构化紧凑视图
pub fn compress_git_status(content: &str) -> CompressionResult {
    let original_bytes = content.len();
    let original_lines = content.lines().count();

    if original_lines <= 5 {
        return CompressionResult {
            original_lines,
            compressed_lines: original_lines,
            original_bytes,
            compressed_bytes: original_bytes,
            reduction_percent: 0.0,
            original_tokens_est: original_bytes / 4,
            compressed_tokens_est: original_bytes / 4,
            content: content.to_string(),
            level: "git_status".to_string(),
            language: "git".to_string(),
        };
    }

    let mut branch = String::new();
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();

    let mut section = "none";

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with("On branch ") {
            branch = trimmed.trim_start_matches("On branch ").to_string();
            continue;
        }

        // 过滤 git 提示语句
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            continue;
        }
        if trimmed.starts_with("Changes to be committed:") {
            section = "staged";
            continue;
        }
        if trimmed.starts_with("Changes not staged for commit:") {
            section = "modified";
            continue;
        }
        if trimmed.starts_with("Untracked files:") {
            section = "untracked";
            continue;
        }

        // 短格式 (porcelain) 支持: "M  file", " M file", "?? file"
        if line.starts_with("?? ") {
            untracked.push(line[3..].trim().to_string());
            continue;
        } else if line.len() > 3 && &line[0..2] == "M " {
            staged.push(line[2..].trim().to_string());
            continue;
        } else if line.len() > 3 && &line[0..2] == " M" {
            modified.push(line[2..].trim().to_string());
            continue;
        }

        // 标准格式项
        match section {
            "staged" => {
                let clean = trimmed.trim_start_matches("modified:").trim_start_matches("new file:").trim_start_matches("deleted:").trim();
                staged.push(clean.to_string());
            }
            "modified" => {
                let clean = trimmed.trim_start_matches("modified:").trim_start_matches("deleted:").trim();
                modified.push(clean.to_string());
            }
            "untracked" => {
                untracked.push(trimmed.to_string());
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    let branch_str = if branch.is_empty() { "unknown" } else { &branch };
    out.push(format!(
        "[Git Status] branch: {} | Staged: {}, Modified: {}, Untracked: {}",
        branch_str,
        staged.len(),
        modified.len(),
        untracked.len()
    ));

    fn format_group(label: &str, files: &[String], out: &mut Vec<String>) {
        if files.is_empty() {
            return;
        }
        out.push(format!("{}:", label));
        let max_show = 15;
        for f in files.iter().take(max_show) {
            out.push(format!("  {}", f));
        }
        if files.len() > max_show {
            out.push(format!("  ... and {} more files", files.len() - max_show));
        }
    }

    format_group("Staged", &staged, &mut out);
    format_group("Modified", &modified, &mut out);
    format_group("Untracked", &untracked, &mut out);

    let compressed_content = never_worse(content, out.join("\n"));
    let compressed_bytes = compressed_content.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes.saturating_sub(compressed_bytes)) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines,
        compressed_lines: compressed_content.lines().count(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: compressed_content,
        level: "git_status".to_string(),
        language: "git".to_string(),
    }
}

/// 专用压缩器: Git Diff 剪枝过滤器 (折叠 lockfile/minified 代码，剪裁过长未修改上下文)
pub fn compress_git_diff(content: &str) -> CompressionResult {
    let original_bytes = content.len();
    let original_lines = content.lines().count();

    if original_lines <= 2 {
        return CompressionResult {
            original_lines,
            compressed_lines: original_lines,
            original_bytes,
            compressed_bytes: original_bytes,
            reduction_percent: 0.0,
            original_tokens_est: original_bytes / 4,
            compressed_tokens_est: original_bytes / 4,
            content: content.to_string(),
            level: "git_diff".to_string(),
            language: "diff".to_string(),
        };
    }

    let mut out_lines = Vec::new();
    let mut in_lockfile = false;
    let mut lockfile_add = 0;
    let mut lockfile_del = 0;
    let mut lockfile_name = String::new();
    let mut unchanged_streak = 0;

    for line in content.lines() {
        if line.starts_with("diff --git ") {
            if in_lockfile {
                out_lines.push(format!(
                    "diff --git {} [Generated/Lockfile diff omitted, +{} -{} lines]",
                    lockfile_name, lockfile_add, lockfile_del
                ));
                in_lockfile = false;
                lockfile_add = 0;
                lockfile_del = 0;
            }

            let path_part = line.trim_start_matches("diff --git ");
            let is_lock = path_part.contains("Cargo.lock")
                || path_part.contains("package-lock.json")
                || path_part.contains("pnpm-lock.yaml")
                || path_part.contains("yarn.lock")
                || path_part.contains(".min.")
                || path_part.contains(".svg");

            if is_lock {
                in_lockfile = true;
                lockfile_name = path_part.to_string();
                continue;
            } else {
                in_lockfile = false;
                out_lines.push(line.to_string());
                unchanged_streak = 0;
                continue;
            }
        }

        if in_lockfile {
            if line.starts_with('+') && !line.starts_with("+++") {
                lockfile_add += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                lockfile_del += 1;
            }
            continue;
        }

        // 普通文件 diff 剪枝
        if line.starts_with("@@") || line.starts_with("index ") || line.starts_with("---") || line.starts_with("+++") {
            out_lines.push(line.to_string());
            unchanged_streak = 0;
            continue;
        }

        if line.starts_with('+') || line.starts_with('-') {
            unchanged_streak = 0;
            out_lines.push(line.to_string());
        } else {
            // 未变更上下文行
            unchanged_streak += 1;
            if unchanged_streak <= 1 {
                out_lines.push(line.to_string());
            } else if unchanged_streak == 2 {
                out_lines.push("  ... [unchanged context lines omitted]".to_string());
            }
        }
    }

    if in_lockfile {
        out_lines.push(format!(
            "diff --git {} [Generated/Lockfile diff omitted, +{} -{} lines]",
            lockfile_name, lockfile_add, lockfile_del
        ));
    }

    let compressed_content = never_worse(content, out_lines.join("\n"));
    let compressed_bytes = compressed_content.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes.saturating_sub(compressed_bytes)) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines,
        compressed_lines: compressed_content.lines().count(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: compressed_content,
        level: "git_diff".to_string(),
        language: "diff".to_string(),
    }
}

/// 专用压缩器: 测试结果智能剪枝 (折叠通过测试，保留失败/panic/堆栈详情与摘要)
pub fn compress_test_output(content: &str) -> CompressionResult {
    let original_bytes = content.len();
    let original_lines = content.lines().count();

    if original_lines <= 15 {
        return CompressionResult {
            original_lines,
            compressed_lines: original_lines,
            original_bytes,
            compressed_bytes: original_bytes,
            reduction_percent: 0.0,
            original_tokens_est: original_bytes / 4,
            compressed_tokens_est: original_bytes / 4,
            content: content.to_string(),
            level: "test_output".to_string(),
            language: "test".to_string(),
        };
    }

    let mut passed_count = 0;
    let mut out_lines = Vec::new();
    let mut in_failures = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // 统计并通过测试折叠
        if trimmed.starts_with("test ") && trimmed.ends_with("... ok") {
            passed_count += 1;
            continue;
        }

        if trimmed.starts_with("failures:") || trimmed.starts_with("FAILURES:") {
            in_failures = true;
        }

        // 识别关键行: 失败详情、panic、summary、compiler 报错
        let is_key_line = in_failures
            || trimmed.contains("FAILED")
            || trimmed.contains("panic")
            || trimmed.contains("assertion failed")
            || trimmed.contains("stack backtrace:")
            || trimmed.starts_with("test result:")
            || trimmed.starts_with("Tests:")
            || trimmed.starts_with("Running ")
            || trimmed.starts_with("Doc-tests");

        if is_key_line {
            out_lines.push(line.to_string());
        }
    }

    if passed_count > 0 {
        out_lines.insert(
            0,
            format!("[RTK TestFilter] {} passing tests omitted (only failures/summary kept)", passed_count),
        );
    }

    let compressed_content = never_worse(content, out_lines.join("\n"));
    let compressed_bytes = compressed_content.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes.saturating_sub(compressed_bytes)) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines,
        compressed_lines: compressed_content.lines().count(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: compressed_content,
        level: "test_output".to_string(),
        language: "test".to_string(),
    }
}

/// 专用压缩器: TypeScript 编译器 (tsc) 报错剪枝
pub fn compress_tsc_output(content: &str) -> CompressionResult {
    let original_bytes = content.len();
    let original_lines = content.lines().count();

    let mut error_lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("error TS") {
            error_lines.push(line.to_string());
        } else if trimmed.starts_with("Found ") && trimmed.contains("error") {
            error_lines.push(line.to_string());
        }
    }

    let compressed_content = if !error_lines.is_empty() {
        never_worse(content, error_lines.join("\n"))
    } else {
        content.to_string()
    };

    let compressed_bytes = compressed_content.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes.saturating_sub(compressed_bytes)) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines,
        compressed_lines: compressed_content.lines().count(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: compressed_content,
        level: "tsc_output".to_string(),
        language: "typescript".to_string(),
    }
}

/// 专用压缩器: Pytest 测试结果压缩 (只保留 FAILED 用例与具体 AssertionError)
pub fn compress_pytest_output(content: &str) -> CompressionResult {
    let original_bytes = content.len();
    let original_lines = content.lines().count();

    let mut out_lines = Vec::new();
    let mut in_failure_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("===") && trimmed.contains("FAILURES") {
            in_failure_section = true;
            out_lines.push(line.to_string());
            continue;
        }

        if trimmed.starts_with("FAILED ")
            || trimmed.contains("AssertionError")
            || (trimmed.starts_with("E   ") && in_failure_section)
            || (trimmed.starts_with("===") && (trimmed.contains("failed") || trimmed.contains("passed")))
        {
            out_lines.push(line.to_string());
        }
    }

    let compressed_content = if !out_lines.is_empty() {
        never_worse(content, out_lines.join("\n"))
    } else {
        compress_test_output(content).content
    };

    let compressed_bytes = compressed_content.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes.saturating_sub(compressed_bytes)) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines,
        compressed_lines: compressed_content.lines().count(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: compressed_content,
        level: "pytest_output".to_string(),
        language: "python".to_string(),
    }
}

/// 专用压缩器: 命令行表格输出压缩 (docker ps, kubectl get 等)
pub fn compress_table_output(content: &str) -> CompressionResult {
    let original_bytes = content.len();
    let original_lines = content.lines().count();

    if original_lines <= 1 {
        return CompressionResult {
            original_lines,
            compressed_lines: original_lines,
            original_bytes,
            compressed_bytes: original_bytes,
            reduction_percent: 0.0,
            original_tokens_est: original_bytes / 4,
            compressed_tokens_est: original_bytes / 4,
            content: content.to_string(),
            level: "table".to_string(),
            language: "table".to_string(),
        };
    }

    let mut out_lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        out_lines.push(parts.join(" | "));
    }

    let compressed_content = never_worse(content, out_lines.join("\n"));
    let compressed_bytes = compressed_content.len();
    let reduction_percent = if original_bytes > 0 {
        ((original_bytes.saturating_sub(compressed_bytes)) as f64 / original_bytes as f64) * 100.0
    } else {
        0.0
    };

    CompressionResult {
        original_lines,
        compressed_lines: compressed_content.lines().count(),
        original_bytes,
        compressed_bytes,
        reduction_percent: (reduction_percent * 10.0).round() / 10.0,
        original_tokens_est: original_bytes / 4,
        compressed_tokens_est: compressed_bytes / 4,
        content: compressed_content,
        level: "table".to_string(),
        language: "table".to_string(),
    }
}

/// 统一命令路由器: 自动识别命令类型并分发到最佳 RTK 压缩算法，自动追踪节省的 Token
pub fn compress_by_command(cmd: &str, output: &str) -> CompressionResult {
    let cmd_lower = cmd.to_lowercase();
    let trimmed_cmd = cmd_lower.trim();

    let res = if output.trim().is_empty() {
        CompressionResult {
            original_lines: 0,
            compressed_lines: 0,
            original_bytes: 0,
            compressed_bytes: 0,
            reduction_percent: 0.0,
            original_tokens_est: 0,
            compressed_tokens_est: 0,
            content: output.to_string(),
            level: "none".to_string(),
            language: "raw".to_string(),
        }
    } else if trimmed_cmd.contains("git status") {
        compress_git_status(output)
    } else if trimmed_cmd.contains("git diff") {
        compress_git_diff(output)
    } else if trimmed_cmd.starts_with("tsc") || trimmed_cmd.contains(" tsc") {
        compress_tsc_output(output)
    } else if trimmed_cmd.contains("pytest") {
        compress_pytest_output(output)
    } else if trimmed_cmd.contains("docker ps")
        || trimmed_cmd.contains("docker image")
        || trimmed_cmd.contains("kubectl get")
    {
        compress_table_output(output)
    } else if trimmed_cmd.contains("cargo test")
        || trimmed_cmd.contains("npm test")
        || trimmed_cmd.contains("go test")
        || trimmed_cmd.contains("mvn test")
        || trimmed_cmd.contains("jest")
    {
        compress_test_output(output)
    } else if let Some(rule) = crate::services::pipeline_filter::PipelineFilterEngine::match_builtin_rule(cmd) {
        let filtered = crate::services::pipeline_filter::PipelineFilterEngine::filter(output, rule);
        let orig_bytes = output.len();
        let comp_bytes = filtered.len();
        let reduction = if orig_bytes > 0 && comp_bytes < orig_bytes {
            ((orig_bytes - comp_bytes) as f64 / orig_bytes as f64) * 100.0
        } else {
            0.0
        };
        CompressionResult {
            original_lines: output.lines().count(),
            compressed_lines: filtered.lines().count(),
            original_bytes: orig_bytes,
            compressed_bytes: comp_bytes,
            reduction_percent: (reduction * 10.0).round() / 10.0,
            original_tokens_est: orig_bytes / 4,
            compressed_tokens_est: comp_bytes / 4,
            content: filtered,
            level: "pipeline".to_string(),
            language: rule.name.clone(),
        }
    } else if trimmed_cmd.contains("mvn ")
        || trimmed_cmd.starts_with("mvn")
        || trimmed_cmd.contains("gradle")
        || trimmed_cmd.contains("cargo build")
        || trimmed_cmd.contains("cargo check")
        || trimmed_cmd.contains("npm run build")
        || trimmed_cmd.contains("pnpm build")
        || trimmed_cmd.contains("yarn build")
    {
        compress_terminal_output(output, 80)
    } else if output.lines().count() > 30 || output.len() > 1500 {
        compress_terminal_output(output, 80)
    } else {
        CompressionResult {
            original_lines: output.lines().count(),
            compressed_lines: output.lines().count(),
            original_bytes: output.len(),
            compressed_bytes: output.len(),
            reduction_percent: 0.0,
            original_tokens_est: output.len() / 4,
            compressed_tokens_est: output.len() / 4,
            content: output.to_string(),
            level: "raw".to_string(),
            language: "raw".to_string(),
        }
    };

    TokenSavingsTracker::global().record_savings(&res);
    res
}

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

static GLOBAL_TRACKER: OnceLock<TokenSavingsTracker> = OnceLock::new();

/// 全局线程安全的 Token 节约追踪器
pub struct TokenSavingsTracker {
    total_original_bytes: AtomicUsize,
    total_compressed_bytes: AtomicUsize,
    total_original_tokens: AtomicUsize,
    total_compressed_tokens: AtomicUsize,
    total_invocations: AtomicUsize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSavingsStats {
    pub total_original_bytes: usize,
    pub total_compressed_bytes: usize,
    pub total_original_tokens: usize,
    pub total_compressed_tokens: usize,
    pub total_tokens_saved: usize,
    pub overall_reduction_percent: f64,
    pub total_invocations: usize,
}

impl TokenSavingsTracker {
    pub fn new() -> Self {
        Self {
            total_original_bytes: AtomicUsize::new(0),
            total_compressed_bytes: AtomicUsize::new(0),
            total_original_tokens: AtomicUsize::new(0),
            total_compressed_tokens: AtomicUsize::new(0),
            total_invocations: AtomicUsize::new(0),
        }
    }

    pub fn global() -> &'static Self {
        GLOBAL_TRACKER.get_or_init(Self::new)
    }

    pub fn record_savings(&self, res: &CompressionResult) {
        self.total_original_bytes
            .fetch_add(res.original_bytes, Ordering::Relaxed);
        self.total_compressed_bytes
            .fetch_add(res.compressed_bytes, Ordering::Relaxed);
        self.total_original_tokens
            .fetch_add(res.original_tokens_est, Ordering::Relaxed);
        self.total_compressed_tokens
            .fetch_add(res.compressed_tokens_est, Ordering::Relaxed);
        self.total_invocations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> TokenSavingsStats {
        let orig_bytes = self.total_original_bytes.load(Ordering::Relaxed);
        let comp_bytes = self.total_compressed_bytes.load(Ordering::Relaxed);
        let orig_tokens = self.total_original_tokens.load(Ordering::Relaxed);
        let comp_tokens = self.total_compressed_tokens.load(Ordering::Relaxed);
        let invocations = self.total_invocations.load(Ordering::Relaxed);
        let saved_tokens = orig_tokens.saturating_sub(comp_tokens);
        let reduction_percent = if orig_bytes > 0 {
            ((orig_bytes.saturating_sub(comp_bytes)) as f64 / orig_bytes as f64) * 100.0
        } else {
            0.0
        };

        TokenSavingsStats {
            total_original_bytes: orig_bytes,
            total_compressed_bytes: comp_bytes,
            total_original_tokens: orig_tokens,
            total_compressed_tokens: comp_tokens,
            total_tokens_saved: saved_tokens,
            overall_reduction_percent: (reduction_percent * 10.0).round() / 10.0,
            total_invocations: invocations,
        }
    }

    pub fn reset(&self) {
        self.total_original_bytes.store(0, Ordering::Relaxed);
        self.total_compressed_bytes.store(0, Ordering::Relaxed);
        self.total_original_tokens.store(0, Ordering::Relaxed);
        self.total_compressed_tokens.store(0, Ordering::Relaxed);
        self.total_invocations.store(0, Ordering::Relaxed);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_parsing() {
        assert_eq!(
            FilterLevel::from_str("aggressive").unwrap(),
            FilterLevel::Aggressive
        );
        assert_eq!(FilterLevel::from_str("min").unwrap(), FilterLevel::Minimal);
        assert!(FilterLevel::from_str("bogus").is_err());
    }

    #[test]
    fn test_language_from_path() {
        assert_eq!(Language::from_path("src/main.rs"), Language::Rust);
        assert_eq!(Language::from_path("a/b/config.json"), Language::Data);
        assert_eq!(Language::from_path("Makefile"), Language::Unknown);
    }

    #[test]
    fn test_minimal_removes_comments() {
        let code = "// a comment\nfn main() {\n    println!(\"hi\");\n}\n";
        let out = filter(code, &Language::Rust, FilterLevel::Minimal);
        assert!(!out.contains("// a comment"));
        assert!(out.contains("fn main()"));
    }

    #[test]
    fn test_aggressive_keeps_signatures_drops_bodies() {
        let code = "use std::fmt;\nfn foo() {\n    let x = 1;\n    do_stuff(x);\n}\n";
        let out = filter(code, &Language::Rust, FilterLevel::Aggressive);
        assert!(out.contains("use std::fmt;"));
        assert!(out.contains("fn foo()"));
        assert!(out.contains("// ... implementation"));
        assert!(!out.contains("do_stuff"));
    }

    #[test]
    fn test_data_not_code_filtered() {
        let json = "{\n  \"a\": \"packages/*\"\n}";
        let out = filter(json, &Language::Data, FilterLevel::Aggressive);
        assert!(out.contains("packages/*"));
    }

    #[test]
    fn test_compress_never_worse() {
        // Tiny file: aggressive would drop everything -> guard keeps raw.
        let content = "x";
        let r = compress(content, "a.rs", FilterLevel::Aggressive);
        assert_eq!(r.content, "x");
        assert!(r.compressed_bytes <= r.original_bytes);
    }

    #[test]
    fn test_smart_log_preserves_error_stacks() {
        let mut log = String::new();
        for i in 0..100 {
            if i == 50 {
                log.push_str("ERROR: Failed to establish database connection\n");
                log.push_str("   at src/db/connect.rs:42\n");
            } else {
                log.push_str(&format!("INFO: Regular log message step {}\n", i));
            }
        }
        let res = compress_terminal_output(&log, 30);
        assert!(res.content.contains("ERROR: Failed to establish database connection"));
        assert!(res.content.contains("at src/db/connect.rs:42"));
        assert!(res.content.contains("[RTK SmartLog omitted"));
        assert!(res.reduction_percent > 0.0);
    }

    #[test]
    fn test_terminal_folding_strips_maven_downloads_and_pins_caused_by() {
        let mut log = String::new();
        log.push_str("[INFO] Scanning for projects...\n");
        log.push_str("[INFO] -------------------< com.example:demo >--------------------\n");
        for i in 0..50 {
            log.push_str(&format!("[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/dep{}/1.0/dep{}-1.0.jar\n", i, i));
            log.push_str(&format!("Progress (1): {} KB\n", i * 10));
            log.push_str(&format!("[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/dep{}/1.0/dep{}-1.0.jar\n", i, i));
        }
        log.push_str("[ERROR] Failed to execute goal org.springframework.boot:spring-boot-maven-plugin:run\n");
        log.push_str("[ERROR] org.springframework.beans.factory.BeanCreationException: Error creating bean\n");
        log.push_str("[ERROR] Caused by: java.lang.NullPointerException: datasource url is required\n");
        log.push_str("    at com.example.demo.config.DbConfig.setup(DbConfig.java:45)\n");
        log.push_str("[INFO] ------------------------------------------------------------------------\n");
        log.push_str("[INFO] BUILD FAILURE\n");
        log.push_str("[INFO] Total time: 12.345 s\n");

        let res = compress_terminal_output(&log, 40);
        assert!(!res.content.contains("Downloading from central"));
        assert!(!res.content.contains("Progress (1):"));
        assert!(res.content.contains("Caused by: java.lang.NullPointerException"));
        assert!(res.content.contains("BUILD FAILURE"));
        assert!(res.content.contains("[RTK SmartLog omitted"));
        assert!(res.reduction_percent > 50.0);
    }

    #[test]
    fn test_compress_git_status() {
        let status_raw = r#"On branch feature/new-graph
Changes to be committed:
  (use "git restore --staged <file>..." to unstage)
	modified:   src/services/graph_service.rs
	new file:   src/services/code_intelligence.rs

Changes not staged for commit:
  (use "git add <file>..." to update what will be committed)
  (use "git restore <file>..." to discard changes in working directory)
	modified:   README.md

Untracked files:
  (use "git add <file>..." to include in what will be committed)
	scratch/test_notes.txt
"#;
        let res = compress_git_status(status_raw);
        assert!(res.content.contains("[Git Status] branch: feature/new-graph"));
        assert!(res.content.contains("Staged: 2, Modified: 1, Untracked: 1"));
        assert!(res.content.contains("src/services/graph_service.rs"));
        assert!(!res.content.contains("to unstage"));
    }

    #[test]
    fn test_compress_git_diff_omits_lockfile() {
        let diff_raw = r#"diff --git a/Cargo.lock b/Cargo.lock
index abc..def 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -10,3 +10,5 @@
+new-dep 1.0.0
+new-dep-sub 2.0.0
-old-dep 0.9.0
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-    println!("old");
+    println!("new");
 }
"#;
        let res = compress_git_diff(diff_raw);
        assert!(res.content.contains("Cargo.lock [Generated/Lockfile diff omitted"));
        assert!(res.content.contains("println!(\"new\");"));
    }

    #[test]
    fn test_compress_test_output_omits_passing_tests() {
        let mut test_out = String::new();
        test_out.push_str("running 100 tests\n");
        for i in 0..98 {
            test_out.push_str(&format!("test tests::test_{} ... ok\n", i));
        }
        test_out.push_str("test tests::test_fail_1 ... FAILED\n");
        test_out.push_str("test tests::test_fail_2 ... FAILED\n");
        test_out.push_str("\nfailures:\n");
        test_out.push_str("---- tests::test_fail_1 stdout ----\n");
        test_out.push_str("thread 'main' panicked at 'assertion failed: 1 == 2'\n");
        test_out.push_str("test result: FAILED. 2 failed; 98 passed;\n");

        let res = compress_test_output(&test_out);
        assert!(res.content.contains("[RTK TestFilter] 98 passing tests omitted"));
        assert!(res.content.contains("test tests::test_fail_1 ... FAILED"));
        assert!(res.content.contains("panicked at 'assertion failed: 1 == 2'"));
        assert!(res.content.contains("test result: FAILED"));
        assert!(res.reduction_percent > 50.0);
    }

    #[test]
    fn test_compress_by_command_and_tracker() {
        TokenSavingsTracker::global().reset();
        let cmd = "git status";
        let status_raw = "On branch master\nChanges not staged for commit:\n  (use git...)\n\tmodified:   foo.rs\n";
        let res = compress_by_command(cmd, status_raw);
        assert_eq!(res.level, "git_status");

        // Verify pipeline rule routing
        let clippy_raw = "   Compiling app v0.1.0\nwarning: variable does not need to be mutable\n --> src/lib.rs:5:9\n";
        let clippy_res = compress_by_command("cargo clippy", clippy_raw);
        assert_eq!(clippy_res.level, "pipeline");
        assert!(!clippy_res.content.contains("Compiling"));
        assert!(clippy_res.content.contains("warning: variable"));

        let stats = TokenSavingsTracker::global().get_stats();
        assert_eq!(stats.total_invocations, 2);
        assert!(stats.total_original_bytes > 0);
    }

    #[test]
    fn test_compress_tsc_output() {
        let raw = r#"
src/app.ts:12:5 - error TS2322: Type 'number' is not assignable to type 'string'.
12     name = 123;
            ~~~~
src/routes.ts:30:10 - error TS2554: Expected 2 arguments, but got 1.
30     call();
            ~
Found 2 errors in 2 files.
"#;
        let res = compress_tsc_output(raw);
        assert_eq!(res.level, "tsc_output");
        assert!(res.content.contains("src/app.ts:12:5 - error TS2322"));
        assert!(res.content.contains("src/routes.ts:30:10 - error TS2554"));
        assert!(res.content.contains("Found 2 errors in 2 files."));
        assert!(!res.content.contains("~~~~"));
        assert!(res.reduction_percent > 20.0);
    }

    #[test]
    fn test_compress_pytest_output() {
        let raw = r#"
============================= test session starts ==============================
rootdir: /repo
collected 50 items

tests/test_basic.py .................................................... [100%]
=================================== FAILURES ===================================
_________________________________ test_login ___________________________________
    def test_login():
>       assert response.status_code == 200
E       AssertionError: assert 401 == 200
tests/test_basic.py:25: AssertionError
=========================== short test summary info ============================
FAILED tests/test_basic.py::test_login - AssertionError: assert 401 == 200
========================= 1 failed, 49 passed in 0.45s =========================
"#;
        let res = compress_pytest_output(raw);
        assert_eq!(res.level, "pytest_output");
        assert!(res.content.contains("FAILED tests/test_basic.py::test_login"));
        assert!(res.content.contains("AssertionError: assert 401 == 200"));
        assert!(res.reduction_percent > 30.0);
    }

    #[test]
    fn test_compress_table_output() {
        let raw = r#"
CONTAINER ID   IMAGE          COMMAND                  CREATED         STATUS         PORTS                    NAMES
a1b2c3d4e5f6   nginx:latest   "/docker-entrypoint.…"   2 hours ago     Up 2 hours     0.0.0.0:80->80/tcp       web-server
f6e5d4c3b2a1   redis:alpine   "docker-entrypoint.s…"   5 hours ago     Up 5 hours     0.0.0.0:6379->6379/tcp   redis-cache
"#;
        let res = compress_table_output(raw);
        assert_eq!(res.level, "table");
        assert!(res.content.contains("CONTAINER | ID | IMAGE | COMMAND"));
        assert!(res.content.contains("a1b2c3d4e5f6 | nginx:latest"));
        assert!(res.reduction_percent > 5.0);
    }
}

