//! 终端构建日志与 CLI 命令输出专用智能剪枝压缩器

use super::{never_worse, CompressionResult, TokenSavingsTracker};

/// 判断单行是否为无价值的终端下载或构建进度条噪声
pub fn is_terminal_noise(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.starts_with("[INFO] Downloading from ")
        || trimmed.starts_with("[INFO] Downloaded from ")
        || trimmed.starts_with("Progress (")
        || trimmed.starts_with("Downloading: ")
        || trimmed.starts_with("Downloaded: ")
        || trimmed.starts_with("Downloading from ")
    {
        return true;
    }

    if trimmed.contains("[===")
        || trimmed.contains("npm http fetch GET")
        || trimmed.contains("fetching packages")
        || trimmed.starts_with("⸨===")
        || (trimmed.starts_with('[') && trimmed.contains("#####"))
        || (trimmed.starts_with('[') && trimmed.contains("░░░"))
    {
        return true;
    }

    if (trimmed.contains("Downloading [") || trimmed.contains("Extracting ["))
        && trimmed.contains(']')
    {
        return true;
    }

    if trimmed.starts_with("Compiling ")
        && !trimmed.contains("error")
        && !trimmed.contains("warning")
    {
        return true;
    }

    false
}

/// 判断单行是否为关键报错/异常堆栈锚点
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
        || l.contains("error[e")
        || l.contains("error ts")
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

/// 压缩终端输出/构建日志：保留首尾摘要与错误堆栈，剔除进度条与下载刷屏噪声
pub fn compress_terminal_output(content: &str, max_lines: usize) -> CompressionResult {
    let lines: Vec<&str> = content.lines().collect();
    let original_lines = lines.len();

    let has_noise = lines.iter().any(|l| is_terminal_noise(l));
    if original_lines <= max_lines && !has_noise {
        return CompressionResult::unchanged(content, "smart_log", "log");
    }

    let mut keep_mask = vec![false; original_lines];
    let head_count = (max_lines / 4).max(10).min(original_lines);
    let tail_count = (max_lines / 3)
        .max(15)
        .min(original_lines.saturating_sub(head_count));

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

    for (i, line) in lines.iter().enumerate() {
        if is_critical_error_anchor(line) {
            let start = i.saturating_sub(3);
            let end = (i + 3).min(original_lines.saturating_sub(1));
            for j in start..=end {
                if j == i || !is_terminal_noise(lines[j]) {
                    keep_mask[j] = true;
                }
            }
        } else {
            let l = line.to_lowercase();
            if l.contains("error")
                || l.contains("panic")
                || l.contains("fail")
                || l.contains("exception")
                || l.contains("traceback")
                || l.contains("warning")
                || l.contains("at line")
                || l.contains("-->")
            {
                if i > 0 && !is_terminal_noise(lines[i - 1]) {
                    keep_mask[i - 1] = true;
                }
                if !is_terminal_noise(lines[i]) {
                    keep_mask[i] = true;
                }
                if i + 1 < original_lines && !is_terminal_noise(lines[i + 1]) {
                    keep_mask[i + 1] = true;
                }
            }
        }
    }

    let mut filtered_lines = Vec::new();
    let mut omitted_count = 0;

    for (i, &keep) in keep_mask.iter().enumerate() {
        if keep {
            if omitted_count > 0 {
                filtered_lines.push(format!(
                    "... [RTK SmartLog omitted {} lines] ...",
                    omitted_count
                ));
                omitted_count = 0;
            }
            filtered_lines.push(lines[i].to_string());
        } else {
            omitted_count += 1;
        }
    }
    if omitted_count > 0 {
        filtered_lines.push(format!(
            "... [RTK SmartLog omitted {} lines] ...",
            omitted_count
        ));
    }

    let filtered = filtered_lines.join("\n");
    CompressionResult::from_comparison(content, filtered, "smart_log", "log")
}

/// 专用压缩器: Git Status 结构化紧凑视图
pub fn compress_git_status(content: &str) -> CompressionResult {
    if content.lines().count() <= 5 {
        return CompressionResult::unchanged(content, "git_status", "git");
    }

    let mut branch = String::new();
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();
    let mut section = "none";

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || (trimmed.starts_with('(') && trimmed.ends_with(')')) {
            continue;
        }
        if trimmed.starts_with("On branch ") {
            branch = trimmed.trim_start_matches("On branch ").to_string();
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

        if let Some(rest) = line.strip_prefix("?? ") {
            untracked.push(rest.trim().to_string());
            continue;
        } else if line.len() > 3 && &line[0..2] == "M " {
            staged.push(line[2..].trim().to_string());
            continue;
        } else if line.len() > 3 && &line[0..2] == " M" {
            modified.push(line[2..].trim().to_string());
            continue;
        }

        match section {
            "staged" => {
                let clean = trimmed
                    .trim_start_matches("modified:")
                    .trim_start_matches("new file:")
                    .trim_start_matches("deleted:")
                    .trim();
                staged.push(clean.to_string());
            }
            "modified" => {
                let clean = trimmed
                    .trim_start_matches("modified:")
                    .trim_start_matches("deleted:")
                    .trim();
                modified.push(clean.to_string());
            }
            "untracked" => untracked.push(trimmed.to_string()),
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

    let compressed = never_worse(content, out.join("\n"));
    CompressionResult::from_comparison(content, compressed, "git_status", "git")
}

/// 专用压缩器: Git Diff 剪枝过滤器
pub fn compress_git_diff(content: &str) -> CompressionResult {
    if content.lines().count() <= 2 {
        return CompressionResult::unchanged(content, "git_diff", "diff");
    }

    let mut out_lines = Vec::new();
    let mut in_lockfile = false;
    let mut lockfile_add = 0;
    let mut lockfile_del = 0;
    let mut lockfile_name = String::new();
    let mut unchanged_streak = 0;

    for line in content.lines() {
        if let Some(path_part) = line.strip_prefix("diff --git ") {
            if in_lockfile {
                out_lines.push(format!(
                    "diff --git {} [Generated/Lockfile diff omitted, +{} -{} lines]",
                    lockfile_name, lockfile_add, lockfile_del
                ));
                lockfile_add = 0;
                lockfile_del = 0;
            }

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

        if line.starts_with("@@")
            || line.starts_with("index ")
            || line.starts_with("---")
            || line.starts_with("+++")
        {
            out_lines.push(line.to_string());
            unchanged_streak = 0;
            continue;
        }

        if line.starts_with('+') || line.starts_with('-') {
            unchanged_streak = 0;
            out_lines.push(line.to_string());
        } else {
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

    let compressed = never_worse(content, out_lines.join("\n"));
    CompressionResult::from_comparison(content, compressed, "git_diff", "diff")
}

/// 专用压缩器: 测试结果智能剪枝
pub fn compress_test_output(content: &str) -> CompressionResult {
    if content.lines().count() <= 15 {
        return CompressionResult::unchanged(content, "test_output", "test");
    }

    let mut passed_count = 0;
    let mut out_lines = Vec::new();
    let mut in_failures = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("test ") && trimmed.ends_with("... ok") {
            passed_count += 1;
            continue;
        }
        if trimmed.starts_with("failures:") || trimmed.starts_with("FAILURES:") {
            in_failures = true;
        }

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
            format!(
                "[RTK TestFilter] {} passing tests omitted (only failures/summary kept)",
                passed_count
            ),
        );
    }

    let compressed = never_worse(content, out_lines.join("\n"));
    CompressionResult::from_comparison(content, compressed, "test_output", "test")
}

/// 专用压缩器: TypeScript 编译器 (tsc) 报错剪枝
pub fn compress_tsc_output(content: &str) -> CompressionResult {
    let mut error_lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("error TS")
            || (trimmed.starts_with("Found ") && trimmed.contains("error"))
        {
            error_lines.push(line.to_string());
        }
    }

    let compressed = if !error_lines.is_empty() {
        never_worse(content, error_lines.join("\n"))
    } else {
        content.to_string()
    };
    CompressionResult::from_comparison(content, compressed, "tsc_output", "typescript")
}

/// 专用压缩器: Pytest 测试结果压缩
pub fn compress_pytest_output(content: &str) -> CompressionResult {
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
            || (trimmed.starts_with("===")
                && (trimmed.contains("failed") || trimmed.contains("passed")))
        {
            out_lines.push(line.to_string());
        }
    }

    let compressed = if !out_lines.is_empty() {
        never_worse(content, out_lines.join("\n"))
    } else {
        compress_test_output(content).content
    };
    CompressionResult::from_comparison(content, compressed, "pytest_output", "python")
}

/// 专用压缩器: 命令行表格输出压缩
pub fn compress_table_output(content: &str) -> CompressionResult {
    if content.lines().count() <= 1 {
        return CompressionResult::unchanged(content, "table", "table");
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

    let compressed = never_worse(content, out_lines.join("\n"));
    CompressionResult::from_comparison(content, compressed, "table", "table")
}

/// 统一命令路由器: 自动识别命令类型并分发到最佳 RTK 压缩算法，自动追踪节省的 Token
pub fn compress_by_command(cmd: &str, output: &str) -> CompressionResult {
    let cmd_lower = cmd.to_lowercase();
    let trimmed_cmd = cmd_lower.trim();

    let res = if output.trim().is_empty() {
        CompressionResult::unchanged(output, "none", "raw")
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
    } else if let Some(rule) =
        crate::services::pipeline_filter::PipelineFilterEngine::match_builtin_rule(cmd)
    {
        let filtered = crate::services::pipeline_filter::PipelineFilterEngine::filter(output, rule);
        CompressionResult::from_comparison(output, filtered, "pipeline", &rule.name)
    } else if trimmed_cmd.contains("mvn ")
        || trimmed_cmd.starts_with("mvn")
        || trimmed_cmd.contains("gradle")
        || trimmed_cmd.contains("cargo build")
        || trimmed_cmd.contains("cargo check")
        || trimmed_cmd.contains("npm run build")
        || trimmed_cmd.contains("pnpm build")
        || trimmed_cmd.contains("yarn build")
        || output.lines().count() > 30
        || output.len() > 1500
    {
        compress_terminal_output(output, 80)
    } else {
        CompressionResult::unchanged(output, "raw", "raw")
    };

    TokenSavingsTracker::global().record_savings(&res);
    res
}
