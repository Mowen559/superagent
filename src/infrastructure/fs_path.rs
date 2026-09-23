//! 统一文件系统路径规范化与工作区安全边界解析器
//! 收敛全库散落的 Windows UNC `\\?\` 剥离与工作区沙箱逃逸检测逻辑。

use std::path::{Path, PathBuf};

/// 剥离字符串形式的 Windows Extended-Length Path (`\\?\`) 前缀
#[inline]
pub fn strip_unc_prefix(s: &str) -> &str {
    s.strip_prefix(r"\\?\").unwrap_or(s)
}

/// 剥离 Windows UNC `\\?\` 前缀并返回规范化 `PathBuf`
pub fn normalize_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    PathBuf::from(strip_unc_prefix(&s))
}

/// 当运行在隔离沙箱目录（`.../runs/<id>/workspace`）时，尝试通过父级 `baseline.json` 还原主工程根目录；
/// 若当前目录已包含任意 `marker_dirs`（如 `.supermemory`, `skills`, `.agents`）或未找到 `baseline.json`，则返回原目录。
pub fn resolve_origin_project_from_baseline(
    workspace_root: &Path,
    marker_dirs: &[&str],
) -> PathBuf {
    if marker_dirs
        .iter()
        .any(|marker| workspace_root.join(marker).exists())
    {
        return workspace_root.to_path_buf();
    }
    if let Some(parent) = workspace_root.parent() {
        let baseline_file = parent.join("baseline.json");
        if baseline_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&baseline_file) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(p) = v.get("projectPath").and_then(|p| p.as_str()) {
                        return PathBuf::from(strip_unc_prefix(p));
                    }
                }
            }
        }
    }
    workspace_root.to_path_buf()
}

/// 对路径进行 `canonicalize` 并自动剥离 Windows `\\?\` 前缀；
/// 若目标尚不存在，则尝试对其存在的父目录 `canonicalize` 后拼接文件名。
pub fn canonicalize_clean(p: &Path) -> PathBuf {
    if p.exists() {
        std::fs::canonicalize(p)
            .map(|cp| normalize_path(&cp))
            .unwrap_or_else(|_| normalize_path(p))
    } else if let Some(parent) = p.parent() {
        if parent.exists() {
            if let Ok(cp) = std::fs::canonicalize(parent) {
                return normalize_path(&cp).join(p.file_name().unwrap_or_default());
            }
        }
        normalize_path(p)
    } else {
        normalize_path(p)
    }
}

/// 智能且安全的路径自愈解析（Auto-healing Safe Path Resolver）
/// 保证相对路径、历史沙箱泄露路径、Windows UNC 路径均安全收敛至 `workspace_root` 内，
/// 并防御 `../../` 目录逃逸。支持尚未创建的待写入文件路径校验。
pub fn resolve_safe_path(raw_path: &str, workspace_root: &Path) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == "./" || trimmed == ".\\" {
        return Ok(canonicalize_clean(workspace_root));
    }

    let norm_slash = trimmed.replace('\\', "/");

    // 1. 历史沙箱路径自愈：若误传了历史 run 的 workspace 绝对路径（如 .../runs/<old_id>/workspace/src/...）
    if norm_slash.contains(".superagent")
        && norm_slash.contains("/runs/")
        && norm_slash.contains("/workspace")
    {
        if let Some(pos) = norm_slash.find("/workspace") {
            let sub = &norm_slash[pos + "/workspace".len()..];
            let sub_clean = sub.trim_start_matches('/');
            if sub_clean.is_empty() || sub_clean == "." {
                return Ok(workspace_root.to_path_buf());
            } else {
                return Ok(workspace_root.join(sub_clean));
            }
        }
    }

    // 2. 相对与绝对路径解析
    let raw_path_obj = Path::new(trimmed);
    let resolved = if raw_path_obj.is_relative() {
        workspace_root.join(raw_path_obj)
    } else {
        let norm_target = normalize_path(raw_path_obj);
        let norm_root = normalize_path(workspace_root);
        if norm_target.starts_with(&norm_root) {
            norm_target
        } else if let Some(pos) = norm_slash.rfind("/workspace/") {
            let sub = &norm_slash[pos + "/workspace/".len()..];
            workspace_root.join(sub)
        } else {
            let root_components: Vec<_> = norm_root.components().collect();
            let last_root_name = root_components
                .last()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .unwrap_or_default();

            let mut matched_candidate = None;
            if !last_root_name.is_empty() {
                if let Some(pos) = norm_slash.rfind(&format!("/{}", last_root_name)) {
                    let sub = &norm_slash[pos + last_root_name.len() + 1..];
                    let sub_clean = sub.trim_start_matches('/');
                    let candidate = workspace_root.join(sub_clean);
                    if candidate.exists() {
                        matched_candidate = Some(candidate);
                    }
                }
            }

            if let Some(candidate) = matched_candidate {
                candidate
            } else {
                return Err(format!(
                    "Security Error: Path '{}' is outside the workspace root",
                    trimmed
                ));
            }
        }
    };

    // 3. 最终沙箱逃逸检查（防御 ../../ 逃逸）
    let canonical_root = canonicalize_clean(workspace_root);
    let canonical_target = canonicalize_clean(&resolved);

    if canonical_target.starts_with(&canonical_root) {
        Ok(canonical_target)
    } else {
        let norm_resolved = normalize_path(&resolved);
        if norm_resolved.starts_with(&canonical_root)
            && !norm_resolved
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            Ok(norm_resolved)
        } else {
            Err(format!(
                "Security Error: Path '{}' escapes workspace boundary '{}'",
                trimmed,
                canonical_root.display()
            ))
        }
    }
}
