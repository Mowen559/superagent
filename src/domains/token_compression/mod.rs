//! Token-killer 源码与命令输出压缩领域门面
//! - 核心源码 AST/签名过滤委托至 `rtk::core::filter`
//! - 终端与命令输出剪枝位于 [`terminal_compressors`]
//! - 全局 Token 节省指标追踪位于 [`tracker`]

pub mod terminal_compressors;
pub mod tracker;

#[cfg(test)]
mod tests;

use std::str::FromStr;

pub use terminal_compressors::{
    compress_by_command, compress_git_diff, compress_git_status, compress_pytest_output,
    compress_table_output, compress_terminal_output, compress_test_output, compress_tsc_output,
    is_critical_error_anchor, is_terminal_noise,
};
pub use tracker::{TokenSavingsStats, TokenSavingsTracker};

/// 压缩强度等级
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterLevel {
    /// 原样返回
    None,
    /// 剥离注释并折叠多余空行，保留全部逻辑代码
    Minimal,
    /// 仅保留导入、类型声明、函数签名与常量（丢弃函数体实现）
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

/// 源码语言分类
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

    pub fn from_path(path: &str) -> Self {
        path.rsplit('.')
            .next()
            .filter(|_| path.contains('.'))
            .map(Language::from_extension)
            .unwrap_or(Language::Unknown)
    }
}

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

pub fn filter(content: &str, lang: &Language, level: FilterLevel) -> String {
    let rtk_level = to_rtk_level(level);
    let rtk_lang = to_rtk_lang(lang);
    rtk::core::filter::get_filter(rtk_level).filter(content, &rtk_lang)
}

#[allow(dead_code)]
pub fn smart_truncate(content: &str, max_lines: usize, lang: &Language) -> String {
    rtk::core::filter::smart_truncate(content, max_lines, &to_rtk_lang(lang))
}

/// 文本压缩结果及统计收据
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
    pub reduction_percent: f64,
    pub original_tokens_est: usize,
    pub compressed_tokens_est: usize,
}

impl CompressionResult {
    /// 统一构造对比压缩结果（消除原本散落 9 处的重复统计与百分比计算样板）
    pub fn from_comparison(raw: &str, compressed: String, level: &str, language: &str) -> Self {
        let original_bytes = raw.len();
        let compressed_bytes = compressed.len();
        let reduction_percent = if original_bytes > 0 && compressed_bytes < original_bytes {
            ((original_bytes - compressed_bytes) as f64 / original_bytes as f64) * 100.0
        } else {
            0.0
        };

        Self {
            original_lines: raw.lines().count(),
            compressed_lines: compressed.lines().count(),
            original_bytes,
            compressed_bytes,
            reduction_percent: (reduction_percent * 10.0).round() / 10.0,
            original_tokens_est: original_bytes / 4,
            compressed_tokens_est: compressed_bytes / 4,
            content: compressed,
            level: level.to_string(),
            language: language.to_string(),
        }
    }

    /// 构造未发生压缩的原样结果
    pub fn unchanged(raw: &str, level: &str, language: &str) -> Self {
        let bytes = raw.len();
        let lines = raw.lines().count();
        Self {
            original_lines: lines,
            compressed_lines: lines,
            original_bytes: bytes,
            compressed_bytes: bytes,
            reduction_percent: 0.0,
            original_tokens_est: bytes / 4,
            compressed_tokens_est: bytes / 4,
            content: raw.to_string(),
            level: level.to_string(),
            language: language.to_string(),
        }
    }
}

/// 劣化保护门控：若过滤后内容为空或体积不降反升，则保留原始文本
pub(crate) fn never_worse(raw: &str, filtered: String) -> String {
    if filtered.trim().is_empty() && !raw.trim().is_empty() {
        return raw.to_string();
    }
    if filtered.len() >= raw.len() {
        return raw.to_string();
    }
    filtered
}

/// 按文件路径/扩展名与指定压缩等级压缩源码
pub fn compress(content: &str, path_or_ext: &str, level: FilterLevel) -> CompressionResult {
    let lang = Language::from_path(path_or_ext);
    let filtered = never_worse(content, filter(content, &lang, level));
    CompressionResult::from_comparison(
        content,
        filtered,
        &level.to_string(),
        &format!("{:?}", lang).to_lowercase(),
    )
}
