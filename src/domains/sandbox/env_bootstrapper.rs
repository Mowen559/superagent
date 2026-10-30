//! 环境探针与依赖自治垫片 (Environment Probe & Dependency Bootstrapper)
//!
//! 赋予 Hub 本地智能体对宿主机器环境的深度感知能力：
//! 1. 自动探测本地 Node.js / Python 运行时及版本；
//! 2. 静态分析爬虫/代码中引入的第三方模块（Python pip 与 Node npm）；
//! 3. 过滤标准内置库，识别潜在缺失的外部依赖；
//! 4. 支持静默安装补齐或生成自愈诊断建议，彻底消灭 `command not found` 与 `ModuleNotFoundError`。

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::process::Command;

/// 运行时类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeKind {
    Python,
    Node,
}

/// 探测到的运行时状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedRuntime {
    pub kind: RuntimeKind,
    pub available: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
}

/// 依赖探测与分析报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyReport {
    pub runtime_kind: RuntimeKind,
    pub runtime_available: bool,
    pub extracted_imports: Vec<String>,
    pub third_party_deps: Vec<String>,
    pub missing_deps: Vec<String>,
    pub bootstrap_command: Option<String>,
}

pub struct EnvBootstrapper;

impl EnvBootstrapper {
    /// 探测系统中的 Python 运行时
    pub fn probe_python() -> DetectedRuntime {
        let candidates = ["python3", "python", "py"];
        for cmd in &candidates {
            if let Ok(output) = Command::new(cmd).arg("--version").output() {
                if output.status.success() {
                    let version_str = String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .to_string();
                    let final_version = if version_str.is_empty() {
                        String::from_utf8_lossy(&output.stderr).trim().to_string()
                    } else {
                        version_str
                    };
                    return DetectedRuntime {
                        kind: RuntimeKind::Python,
                        available: true,
                        binary_path: Some(cmd.to_string()),
                        version: Some(final_version),
                    };
                }
            }
        }

        DetectedRuntime {
            kind: RuntimeKind::Python,
            available: false,
            binary_path: None,
            version: None,
        }
    }

    /// 探测系统中的 Node.js 运行时
    pub fn probe_node() -> DetectedRuntime {
        let candidates = ["node", "nodejs"];
        for cmd in &candidates {
            if let Ok(output) = Command::new(cmd).arg("--version").output() {
                if output.status.success() {
                    let version_str = String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .to_string();
                    return DetectedRuntime {
                        kind: RuntimeKind::Node,
                        available: true,
                        binary_path: Some(cmd.to_string()),
                        version: Some(version_str),
                    };
                }
            }
        }

        DetectedRuntime {
            kind: RuntimeKind::Node,
            available: false,
            binary_path: None,
            version: None,
        }
    }

    /// 从 Python 代码中提取所有 import 依赖，并过滤标准内置库
    pub fn inspect_python_dependencies(code: &str) -> (Vec<String>, Vec<String>) {
        let mut all_imports = HashSet::new();

        for raw_line in code.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("import ") {
                let clean_rest = rest.split('#').next().unwrap_or("").trim();
                for part in clean_rest.split(',') {
                    let root_pkg = part.trim().split('.').next().unwrap_or("").trim();
                    let clean = root_pkg.split_whitespace().next().unwrap_or("");
                    if !clean.is_empty() {
                        all_imports.insert(clean.to_string());
                    }
                }
            } else if let Some(rest) = line.strip_prefix("from ") {
                let clean_rest = rest.split('#').next().unwrap_or("").trim();
                if let Some(pkg) = clean_rest.split_whitespace().next() {
                    let root_pkg = pkg.split('.').next().unwrap_or("").trim();
                    if !root_pkg.is_empty() {
                        all_imports.insert(root_pkg.to_string());
                    }
                }
            }
        }

        // Python 常用标准库名单 (内置免安装)
        let stdlib: HashSet<&'static str> = [
            "os", "sys", "re", "json", "time", "datetime", "math", "random", "urllib",
            "hashlib", "base64", "collections", "itertools", "functools", "pathlib",
            "shutil", "tempfile", "glob", "typing", "io", "copy", "logging", "asyncio",
            "threading", "subprocess", "socket", "http", "ssl", "csv", "xml", "html",
            "unittest", "argparse", "uuid", "sqlite3", "pickle", "traceback", "inspect",
        ].into_iter().collect();

        let mut all_vec: Vec<String> = all_imports.iter().cloned().collect();
        all_vec.sort();

        let third_party: Vec<String> = all_imports
            .into_iter()
            .filter(|pkg| !stdlib.contains(pkg.as_str()))
            .collect();

        (all_vec, third_party)
    }

    /// 从 JavaScript / Node.js 代码中提取所有 require / import 依赖，并过滤标准内置库
    pub fn inspect_node_dependencies(code: &str) -> (Vec<String>, Vec<String>) {
        let mut all_imports = HashSet::new();

        // 匹配 require('xxx') 或 require("xxx")
        let re_require = Regex::new(r#"require\s*\(\s*['"]([^'"./\\]+)['"]\s*\)"#).unwrap();
        for caps in re_require.captures_iter(code) {
            if let Some(m) = caps.get(1) {
                all_imports.insert(m.as_str().trim().to_string());
            }
        }

        // 匹配 import ... from 'xxx'
        let re_import = Regex::new(r#"from\s+['"]([^'"./\\]+)['"]"#).unwrap();
        for caps in re_import.captures_iter(code) {
            if let Some(m) = caps.get(1) {
                all_imports.insert(m.as_str().trim().to_string());
            }
        }

        // Node.js 标准内置库 (免安装)
        let stdlib: HashSet<&'static str> = [
            "fs", "path", "http", "https", "url", "crypto", "os", "events", "stream",
            "util", "child_process", "cluster", "net", "dgram", "dns", "readline",
            "zlib", "buffer", "assert", "querystring", "vm", "perf_hooks", "worker_threads",
        ].into_iter().collect();

        let mut all_vec: Vec<String> = all_imports.iter().cloned().collect();
        all_vec.sort();

        let third_party: Vec<String> = all_imports
            .into_iter()
            .filter(|pkg| !stdlib.contains(pkg.as_str()))
            .collect();

        (all_vec, third_party)
    }

    /// 评估执行准备度，并生成依赖自愈报告与安装指令
    pub fn evaluate_execution_readiness(kind: RuntimeKind, code: &str) -> DependencyReport {
        let runtime = match kind {
            RuntimeKind::Python => Self::probe_python(),
            RuntimeKind::Node => Self::probe_node(),
        };

        let (all_imports, third_party) = match kind {
            RuntimeKind::Python => Self::inspect_python_dependencies(code),
            RuntimeKind::Node => Self::inspect_node_dependencies(code),
        };

        // 检查哪些第三方依赖可能缺失并生成自愈指令
        let bootstrap_command = if !third_party.is_empty() {
            match kind {
                RuntimeKind::Python => {
                    let py_cmd = runtime.binary_path.as_deref().unwrap_or("python");
                    Some(format!("{} -m pip install --quiet {}", py_cmd, third_party.join(" ")))
                }
                RuntimeKind::Node => {
                    Some(format!("npm install --no-audit --quiet {}", third_party.join(" ")))
                }
            }
        } else {
            None
        };

        DependencyReport {
            runtime_kind: kind,
            runtime_available: runtime.available,
            extracted_imports: all_imports,
            third_party_deps: third_party.clone(),
            missing_deps: third_party, // 潜在需要安装的非内置依赖
            bootstrap_command,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inspect_python_dependencies() {
        let py_code = r#"
import os
import sys
import requests
from bs4 import BeautifulSoup
import json, time, re
from playwright.sync_api import sync_playwright

def crawl():
    print("crawling")
"#;

        let (all_imports, third_party) = EnvBootstrapper::inspect_python_dependencies(py_code);
        assert!(all_imports.contains(&"requests".to_string()));
        assert!(all_imports.contains(&"os".to_string()));

        // 标准库已过滤，只剩下第三方依赖
        assert!(third_party.contains(&"requests".to_string()));
        assert!(third_party.contains(&"bs4".to_string()));
        assert!(third_party.contains(&"playwright".to_string()));
        assert!(!third_party.contains(&"os".to_string()));
        assert!(!third_party.contains(&"sys".to_string()));
        assert!(!third_party.contains(&"json".to_string()));
    }

    #[test]
    fn test_inspect_node_dependencies() {
        let js_code = r#"
const fs = require('fs');
const path = require('path');
const axios = require('axios');
const cheerio = require('cheerio');
import lodash from 'lodash';
"#;

        let (all_imports, third_party) = EnvBootstrapper::inspect_node_dependencies(js_code);
        assert!(all_imports.contains(&"fs".to_string()));
        assert!(all_imports.contains(&"axios".to_string()));

        assert!(third_party.contains(&"axios".to_string()));
        assert!(third_party.contains(&"cheerio".to_string()));
        assert!(third_party.contains(&"lodash".to_string()));
        assert!(!third_party.contains(&"fs".to_string()));
        assert!(!third_party.contains(&"path".to_string()));
    }

    #[test]
    fn test_evaluate_execution_readiness() {
        let code = "import requests\nimport json\n";
        let report = EnvBootstrapper::evaluate_execution_readiness(RuntimeKind::Python, code);
        assert_eq!(report.runtime_kind, RuntimeKind::Python);
        assert_eq!(report.third_party_deps, vec!["requests".to_string()]);
        assert!(report.bootstrap_command.is_some());
        assert!(report.bootstrap_command.unwrap().contains("install"));
    }
}
