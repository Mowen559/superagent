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

/// Compress raw terminal output / build logs while preserving error tracebacks, panic messages,
/// compiler warnings and key status lines.
pub fn compress_terminal_output(content: &str, max_lines: usize) -> CompressionResult {
    let lines: Vec<&str> = content.lines().collect();
    let original_bytes = content.len();
    let original_lines = lines.len();

    if original_lines <= max_lines {
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

    // Always keep top N lines (command & initial context) and bottom N lines (summary / exit status)
    let head_count = (max_lines / 4).max(10).min(original_lines);
    let tail_count = (max_lines / 3).max(15).min(original_lines.saturating_sub(head_count));

    for i in 0..head_count {
        keep_mask[i] = true;
    }
    for i in (original_lines.saturating_sub(tail_count))..original_lines {
        keep_mask[i] = true;
    }

    // Keep error stack lines and context (1 line before & after error lines)
    for (i, line) in lines.iter().enumerate() {
        let l = line.to_lowercase();
        if l.contains("error") || l.contains("panic") || l.contains("fail")
            || l.contains("exception") || l.contains("traceback") || l.contains("caused by")
            || l.contains("warning") || l.contains("at line") || l.contains("-->")
        {
            if i > 0 { keep_mask[i - 1] = true; }
            keep_mask[i] = true;
            if i + 1 < original_lines { keep_mask[i + 1] = true; }
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
        ((original_bytes - compressed_bytes) as f64 / original_bytes as f64) * 100.0
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
}
