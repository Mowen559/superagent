//! 文件系统工具集（Filesystem Tools）模块化门面
//! 按职责解耦为：
//! - `read_tools`：只读文件浏览、目录列举、符号大纲提取
//! - `search_tools`：文件名与文本内容检索
//! - `write_tools`：单文件写入、单块替换、多区块重构
//! - `diagnostics`：写入前置影子快照、后置语法/架构诊断与影响面推演
//! 路径规范化与工作区安全门控统一委托至 `crate::infrastructure::fs_path`。

pub mod diagnostics;
pub mod read_tools;
pub mod search_tools;
pub mod write_tools;

#[cfg(test)]
mod tests;

pub use crate::infrastructure::fs_path::{normalize_path, resolve_safe_path};
pub use diagnostics::{
    analyze_symbol_impact_radar, collect_file_diagnostics, format_arch_advisories,
    record_shadow_backup,
};
pub use read_tools::{
    extract_symbol_outline, ListDirectoryTool, ReadFileTool, ViewOutlineTool,
};
pub use search_tools::{AstGrepTool, FindByNameTool, GrepSearchTool};
pub use write_tools::{MultiReplaceTool, ReplaceInFileTool, WriteFileTool};
