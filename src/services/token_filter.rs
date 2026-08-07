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

/// Map hub's `FilterLevel` onto rtk's equivalent.
fn to_rtk_level(level: FilterLevel) -> rtk::core::filter::FilterLevel {
    match level {
        FilterLevel::None => rtk::core::filter::FilterLevel::None,
        FilterLevel::Minimal => rtk::core::filter::FilterLevel::Minimal,
        FilterLevel::Aggressive => rtk::core::filter::FilterLevel::Aggressive,
    }
}

/// Map hub's `Language` onto rtk's equivalent (identical variant set).
fn to_rtk_lang(lang: &Language) -> rtk::core::filter::Language {
    use rtk::core::filter::Language as R;
    match lang {
        Language::Rust => R::Rust,
        Language::Python => R::Python,
        Language::JavaScript => R::JavaScript,
        Language::TypeScript => R::TypeScript,
        Language::Go => R::Go,
        Language::C => R::C,
        Language::Cpp => R::Cpp,
        Language::Java => R::Java,
        Language::Ruby => R::Ruby,
        Language::Shell => R::Shell,
        Language::Data => R::Data,
        Language::Unknown => R::Unknown,
    }
}

/// Apply a filter level to `content`, delegating to rtk's filter strategies.
pub fn filter(content: &str, lang: &Language, level: FilterLevel) -> String {
    if level == FilterLevel::None {
        return content.to_string();
    }
    rtk::core::filter::get_filter(to_rtk_level(level)).filter(content, &to_rtk_lang(lang))
}

/// Truncate to `max_lines`, prioritizing structurally important lines
/// (delegates to rtk).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_parsing() {
        assert_eq!(FilterLevel::from_str("aggressive").unwrap(), FilterLevel::Aggressive);
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
}
