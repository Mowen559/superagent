//! 文件变更前后置钩子：影子备份、即时语法/LSP/架构契约诊断与符号影响面推演

use serde_json::{json, Value};
use std::path::Path;

/// 触发前置影子 Git 快照备份（Spinal Reflex - Shadow Backup）
pub fn record_shadow_backup(workspace_root: &Path, target_path: &Path, op_label: &str) {
    let cwd_str = workspace_root.to_string_lossy().to_string();
    let _ = crate::domains::version_control::git_service::record_ai_snapshot(
        &cwd_str,
        &format!("[Shadow Backup] Auto-backup before {} {}", op_label, target_path.display()),
    );
}

/// 格式化架构规范检查建议文本
pub fn format_arch_advisories(path: &Path, content: &str) -> Option<String> {
    let advisories =
        crate::services::brain::architectural_linter::ArchitecturalLinter::lint(path, content);
    crate::services::brain::architectural_linter::ArchitecturalLinter::format_advisories(
        &advisories,
    )
}

/// 统一推演被修改代码片段中声明符号的外部调用波及面（Call-Graph Impact Radar）
pub fn analyze_symbol_impact_radar(
    workspace_root: &Path,
    file_path: &Path,
    snippet: &str,
) -> Option<Value> {
    let sym = crate::domains::version_control::diff_impact::extract_declared_symbol(snippet)?;
    let app_paths = crate::config::app_paths::AppPaths::new();
    let config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
    let cwd_str = workspace_root.to_string_lossy().to_string();
    let db_path = config.codebase_memory_db_path(Some(&cwd_str));
    let db_path_str = db_path.to_string_lossy().to_string();

    if let Ok(radar) =
        crate::domains::version_control::diff_impact::DiffImpactAnalyzer::analyze_symbol_impact(
            workspace_root,
            &db_path_str,
            &sym,
            Some(file_path),
        )
    {
        if radar.total_callers > 0 {
            eprintln!(
                "\x1b[33m[Impact Radar] Warning: Modified symbol '{}' is referenced by {} external location(s): {:?}\x1b[0m",
                sym, radar.total_callers, radar.affected_files
            );
            return Some(serde_json::to_value(radar).unwrap_or(Value::Null));
        }
    }
    None
}

/// 收集已写入文件的即时语法与行级编译器诊断（Compiler/Syntax in the Loop）
pub fn collect_file_diagnostics(path: &Path, content: &str) -> Vec<Value> {
    let mut diagnostics = Vec::new();

    // 1. LSP 全局活跃客户端诊断
    let path_str = path.to_string_lossy().to_string();
    let lsp_diags =
        crate::services::brain::tools::lsp_client::get_global_diagnostics_for_file(&path_str);
    diagnostics.extend(lsp_diags);

    // 2. 内置快速轻量语法结构校验
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "json" => {
            if let Err(e) = serde_json::from_str::<Value>(content) {
                diagnostics.push(json!({
                    "severity": "error",
                    "line": e.line(),
                    "column": e.column(),
                    "source": "SyntaxValidator",
                    "message": format!("JSON 语法错误: {}", e)
                }));
            }
        }
        "toml" => {
            if let Err(e) = toml::from_str::<toml::Value>(content) {
                diagnostics.push(json!({
                    "severity": "error",
                    "line": e.span().map(|s| s.start).unwrap_or(1),
                    "source": "SyntaxValidator",
                    "message": format!("TOML 语法错误: {}", e.message())
                }));
            }
        }
        "rs" => {
            let compiler_diags = check_rust_file(path);
            if !compiler_diags.is_empty() {
                diagnostics.extend(compiler_diags);
            } else {
                run_fallback_bracket_check(content, &mut diagnostics);
            }
        }
        "py" => {
            let py_diags = check_python_file(path);
            if !py_diags.is_empty() {
                diagnostics.extend(py_diags);
            }
        }
        "ts" | "tsx" | "js" | "jsx" => {
            let js_diags = check_js_ts_file(path);
            if !js_diags.is_empty() {
                diagnostics.extend(js_diags);
            } else {
                run_fallback_bracket_check(content, &mut diagnostics);
            }
        }
        "java" | "c" | "cpp" | "go" => {
            run_fallback_bracket_check(content, &mut diagnostics);
        }
        _ => {}
    }

    // 3. 架构契约实时 Linter
    let arch_advisories =
        crate::services::brain::architectural_linter::ArchitecturalLinter::lint(path, content);
    for adv in arch_advisories {
        diagnostics.push(json!({
            "severity": "warning",
            "line": 1,
            "column": 1,
            "source": "ArchitectureLinter",
            "rule": adv.rule_name,
            "message": format!("[{}] {}. Fix: {}", adv.rule_name, adv.violation, adv.prescribed_fix)
        }));
    }

    diagnostics
}

/// 快速执行带超时的子进程命令
fn run_command_with_timeout(mut cmd: std::process::Command, timeout_secs: u64) -> Option<std::process::Output> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let res = cmd.output();
        let _ = tx.send(res);
    });

    match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        Ok(Ok(out)) => Some(out),
        _ => None,
    }
}

/// 寻找最近的父级 Cargo.toml 目录
fn find_cargo_manifest_dir(start: &Path) -> Option<std::path::PathBuf> {
    let mut curr = start.parent();
    while let Some(dir) = curr {
        if dir.join("Cargo.toml").exists() {
            return Some(dir.to_path_buf());
        }
        curr = dir.parent();
    }
    None
}

/// 针对 Rust 文件触发真实的 cargo check
fn check_rust_file(path: &Path) -> Vec<Value> {
    let mut diags = Vec::new();
    let manifest_dir = match find_cargo_manifest_dir(path) {
        Some(d) => d,
        None => return diags,
    };

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("check")
        .arg("--message-format=json")
        .arg("--quiet")
        .current_dir(&manifest_dir);

    if let Some(output) = run_command_with_timeout(cmd, 6) {
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let path_file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        for line in stdout_str.lines() {
            if let Ok(val) = serde_json::from_str::<Value>(line) {
                if val.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                    if let Some(msg) = val.get("message") {
                        let level = msg.get("level").and_then(|l| l.as_str()).unwrap_or("");
                        let text = msg.get("message").and_then(|m| m.as_str()).unwrap_or("");
                        
                        // 只关注当前文件或具有 error 级别的诊断
                        let spans = msg.get("spans").and_then(|s| s.as_array());
                        let mut line_num = 1;
                        let mut col_num = 1;
                        let mut belongs_to_current = false;

                        if let Some(spans_arr) = spans {
                            for span in spans_arr {
                                if let Some(file_name) = span.get("file_name").and_then(|f| f.as_str()) {
                                    if !path_file_name.is_empty() && file_name.contains(path_file_name) {
                                        belongs_to_current = true;
                                        line_num = span.get("line_start").and_then(|l| l.as_u64()).unwrap_or(1);
                                        col_num = span.get("column_start").and_then(|c| c.as_u64()).unwrap_or(1);
                                        break;
                                    }
                                }
                            }
                        }

                        if belongs_to_current && (level == "error" || level == "warning") {
                            diags.push(json!({
                                "severity": level,
                                "line": line_num,
                                "column": col_num,
                                "source": "rustc",
                                "message": text
                            }));
                        }
                    }
                }
            }
        }
    }
    diags
}

/// 针对 Python 文件触发真实的 py_compile 语法检查
fn check_python_file(path: &Path) -> Vec<Value> {
    let mut diags = Vec::new();
    let mut cmd = std::process::Command::new("python");
    cmd.arg("-m")
        .arg("py_compile")
        .arg(path.to_string_lossy().to_string());

    if let Some(output) = run_command_with_timeout(cmd, 3) {
        if !output.status.success() {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let mut line_num = 1;
            for line in stderr_str.lines() {
                if line.contains("line ") {
                    let parts: Vec<&str> = line.split("line ").collect();
                    if parts.len() > 1 {
                        let num_part = parts[1].split_whitespace().next().unwrap_or("1");
                        if let Ok(num) = num_part.trim_matches(|c: char| !c.is_ascii_digit()).parse::<u64>() {
                            line_num = num;
                        }
                    }
                }
            }
            diags.push(json!({
                "severity": "error",
                "line": line_num,
                "column": 1,
                "source": "PythonSyntaxChecker",
                "message": stderr_str.trim().to_string()
            }));
        }
    }
    diags
}

/// 针对 JS/TS 文件触发快速语法自检
fn check_js_ts_file(path: &Path) -> Vec<Value> {
    let mut diags = Vec::new();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    
    if ext == "js" || ext == "mjs" || ext == "cjs" {
        let mut cmd = std::process::Command::new("node");
        cmd.arg("-c").arg(path.to_string_lossy().to_string());
        if let Some(output) = run_command_with_timeout(cmd, 3) {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                diags.push(json!({
                    "severity": "error",
                    "line": 1,
                    "column": 1,
                    "source": "NodeSyntaxChecker",
                    "message": stderr.trim().to_string()
                }));
            }
        }
    }
    diags
}

/// 兜底结构化括号平衡性检查器
fn run_fallback_bracket_check(content: &str, diagnostics: &mut Vec<Value>) {
    let mut stack = Vec::new();
    let mut line = 1usize;
    let mut col = 0usize;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        col += 1;
        if ch == '\n' {
            line += 1;
            col = 0;
            in_line_comment = false;
            continue;
        }
        if in_line_comment {
            continue;
        }
        if in_block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                col += 1;
                in_block_comment = false;
            }
            continue;
        }
        if in_string {
            if ch == '\\' {
                let _ = chars.next();
                col += 1;
            } else if ch == string_char {
                in_string = false;
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            col += 1;
            in_line_comment = true;
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            col += 1;
            in_block_comment = true;
            continue;
        }
        if ch == '"' || ch == '\'' || ch == '`' {
            in_string = true;
            string_char = ch;
            continue;
        }

        match ch {
            '{' | '(' | '[' => stack.push((ch, line, col)),
            '}' | ')' | ']' => {
                let (expected_open, label) = match ch {
                    '}' => ('{', "右花括号 '}'"),
                    ')' => ('(', "右圆括号 ')'"),
                    _ => ('[', "右方括号 ']'"),
                };
                if let Some((open, _, _)) = stack.pop() {
                    if open != expected_open {
                        diagnostics.push(json!({
                            "severity": "error",
                            "line": line,
                            "column": col,
                            "source": "SyntaxValidator",
                            "message": format!("括号不匹配: 遇到 '{}'，但此前未闭合的左括号为 '{}'", ch, open)
                        }));
                        return;
                    }
                } else {
                    diagnostics.push(json!({
                        "severity": "error",
                        "line": line,
                        "column": col,
                        "source": "SyntaxValidator",
                        "message": format!("括号不匹配: 出现多余未配对的{}", label)
                    }));
                    return;
                }
            }
            _ => {}
        }
    }

    if !stack.is_empty() {
        if let Some((open, o_line, o_col)) = stack.last() {
            diagnostics.push(json!({
                "severity": "error",
                "line": o_line,
                "column": o_col,
                "source": "SyntaxValidator",
                "message": format!("结构未闭合: 位于第 {} 行的 '{}' 未找到对应的闭合右括号", o_line, open)
            }));
        }
    }
}
