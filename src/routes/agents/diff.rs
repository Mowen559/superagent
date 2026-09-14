//! Local coding-agent endpoints (`/api/agents/*`).
//!
//! Surfaces the agent adapter layer (`services::agent`) over HTTP:
//! - `GET  /api/agents`                     — detect every known CLI on PATH.
//! - `POST /api/agents/run`                 — launch a backend as a background
//!   task; structured events land in `{task_id}.events.jsonl` and fan out over
//!   the TaskManager event bus.
//! - `GET  /api/agents/tasks/:id/events`    — SSE stream: replay persisted
//!   events, then follow the live broadcast until the run finishes.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::models::task::TaskStatus;
use crate::services::agent::{
    self, types::SyncConfigPayload, AgentKind, AgentMessage, ExecOptions,
};
use crate::AppState;


use super::*;

pub(crate) struct DevelopmentRunPaths {
    project: std::path::PathBuf,
    run_dir: std::path::PathBuf,
    baseline: crate::services::change_set::BaselineManifest,
}

pub(crate) fn development_run_paths(
    state: &Arc<AppState>,
    task: &crate::models::task::Task,
) -> Result<DevelopmentRunPaths, String> {
    if task.task_type != "agent" {
        return Err("Task is not an Agent development run".to_string());
    }
    if task.status != TaskStatus::Completed {
        return Err(format!(
            "Agent task must be completed before reviewing changes; current status is {}",
            task.status.as_str()
        ));
    }
    if task.project_path.is_empty() {
        return Err("Agent task has no workspace".to_string());
    }
    let project = state.workspace_capabilities.resolve(&task.project_path)?;
    let baseline_path = task
        .workspace_baseline
        .as_deref()
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| {
            state
                .app_paths
                .data_path(&["runs", &task.id, "baseline.json"])
        });
    let run_dir = baseline_path
        .parent()
        .map(|parent| parent.join("workspace"))
        .filter(|path| path.is_dir())
        // Compatibility with development runs created by the previous layout.
        .unwrap_or_else(|| {
            state
                .runtime_config
                .projects_root
                .join(format!(".deepcloud/runs/{}", task.id))
        });
    if !run_dir.is_dir() {
        return Err("Isolated Agent workspace is unavailable".to_string());
    }
    let baseline = crate::services::change_set::load_baseline(&baseline_path)?;
    Ok(DevelopmentRunPaths {
        project,
        run_dir,
        baseline,
    })
}

pub(crate) async fn get_task_diff(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.task_manager.get_task(&task_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    };
    let paths = match development_run_paths(&state, &task) {
        Ok(paths) => paths,
        Err(error) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": error, "kind": "development_run_not_ready" })),
            )
                .into_response()
        }
    };
    let change_set = match crate::services::change_set::inspect_changes(
        &paths.baseline,
        &paths.project,
        &paths.run_dir,
    ) {
        Ok(change_set) => change_set,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error })),
            )
                .into_response()
        }
    };
    state.task_manager.update_development_run(
        &task_id,
        Some(json!(change_set.clone())),
        Some("pending".to_string()),
    );

    let output = tokio::process::Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--")
        .arg(&paths.project)
        .arg(&paths.run_dir)
        .output()
        .await;
    let diff = output
        .as_ref()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let truncated = diff.chars().count() > MAX_DIFF_CHARS;
    let diff = diff.chars().take(MAX_DIFF_CHARS).collect::<String>();
    Json(json!({
        "diff": diff,
        "diffTruncated": truncated,
        "files": change_set.files,
        "conflicts": change_set.conflicts,
    }))
    .into_response()
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyBody {
    pub files: Option<Vec<String>>,
    pub force: Option<bool>,
}

#[derive(Clone)]
pub(crate) struct VerificationCommand {
    label: String,
    program: String,
    args: Vec<String>,
}

pub(crate) fn verification_plan(run_dir: &std::path::Path) -> Vec<VerificationCommand> {
    verification_plan_with_changes(run_dir, &[])
}

pub(crate) fn verification_plan_with_changes(
    run_dir: &std::path::Path,
    changed_files: &[String],
) -> Vec<VerificationCommand> {
    let mut plan = Vec::new();

    // 1. TDD 确定性验证门禁：智能探测添加或修改的针对性测试用例
    for path_str in changed_files {
        let normalized = path_str.replace('\\', "/");
        let path = std::path::Path::new(&normalized);
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Java 测试感知: e.g. *Test.java / *IT.java
        if (file_name.ends_with("Test.java") || file_name.ends_with("IT.java")) && run_dir.join("pom.xml").is_file() {
            let test_class_name = file_name.trim_end_matches(".java");
            let mut args = vec!["test".to_string(), format!("-Dtest={}", test_class_name)];
            // 针对多模块 Maven 工程，如 common/src/test/... 提取子模块 -pl common
            if let Some(submodule) = normalized.split("/src/").next() {
                if !submodule.is_empty() && submodule != "." && run_dir.join(submodule).join("pom.xml").is_file() {
                    args.insert(0, "-pl".to_string());
                    args.insert(1, submodule.to_string());
                }
            }
            plan.push(VerificationCommand {
                label: format!("TDD Targeted Maven Test: {}", test_class_name),
                program: if cfg!(windows) { "mvn.cmd" } else { "mvn" }.to_string(),
                args,
            });
            break;
        }

        // 前端测试感知: e.g. *.test.ts / *.spec.ts / *.test.tsx
        if (file_name.ends_with(".test.ts") || file_name.ends_with(".test.tsx") || file_name.ends_with(".spec.ts"))
            && run_dir.join("package.json").is_file()
        {
            plan.push(VerificationCommand {
                label: format!("TDD Targeted Frontend Test: {}", file_name),
                program: if cfg!(windows) { "npm.cmd" } else { "npm" }.to_string(),
                args: vec!["test".to_string(), "--".to_string(), normalized.clone()],
            });
            break;
        }

        // Python 测试感知: e.g. test_*.py / *_test.py
        if (file_name.starts_with("test_") || file_name.ends_with("_test.py")) && file_name.ends_with(".py") {
            plan.push(VerificationCommand {
                label: format!("TDD Targeted Python Test: {}", file_name),
                program: "pytest".to_string(),
                args: vec![normalized.clone()],
            });
            break;
        }

        // Rust 针对性测试感知: e.g. tests/*.rs
        if normalized.starts_with("tests/") && file_name.ends_with(".rs") && run_dir.join("Cargo.toml").is_file() {
            let test_target = file_name.trim_end_matches(".rs");
            plan.push(VerificationCommand {
                label: format!("TDD Targeted Rust Test: {}", test_target),
                program: "cargo".to_string(),
                args: vec!["test".to_string(), "--test".to_string(), test_target.to_string()],
            });
            break;
        }
    }

    // 2. 基线构建编译门禁 (Baseline Build Verification)
    if run_dir.join("Cargo.toml").is_file() && plan.len() < 3 {
        plan.push(VerificationCommand {
            label: "Rust cargo check".to_string(),
            program: "cargo".to_string(),
            args: vec!["check".to_string(), "--quiet".to_string()],
        });
    }
    let package_json = run_dir.join("package.json");
    if let Ok(payload) = std::fs::read_to_string(package_json) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) {
            if let Some(scripts) = value.get("scripts").and_then(|value| value.as_object()) {
                for name in ["typecheck", "lint", "test", "build"] {
                    if scripts.contains_key(name) && plan.len() < 3 {
                        plan.push(VerificationCommand {
                            label: format!("Node.js npm run {name}"),
                            program: if cfg!(windows) { "npm.cmd" } else { "npm" }.to_string(),
                            args: vec!["run".to_string(), name.to_string()],
                        });
                    }
                }
            }
        }
    }
    if run_dir.join("pom.xml").is_file() && plan.len() < 3 {
        plan.push(VerificationCommand {
            label: "Maven compile".to_string(),
            program: if cfg!(windows) { "mvn.cmd" } else { "mvn" }.to_string(),
            args: vec![
                "-q".to_string(),
                "-DskipTests".to_string(),
                "compile".to_string(),
            ],
        });
    }
    if (run_dir.join("pyproject.toml").is_file() || run_dir.join("requirements.txt").is_file())
        && plan.len() < 3
    {
        plan.push(VerificationCommand {
            label: "Python compileall".to_string(),
            program: "python".to_string(),
            args: vec![
                "-m".to_string(),
                "compileall".to_string(),
                "-q".to_string(),
                ".".to_string(),
            ],
        });
    }
    plan
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    pub file: String,
    pub line_hint: Option<usize>,
    pub severity: String, // "blocker", "warning", "info"
    pub rule: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CriticReview {
    pub status: String, // "approved", "changes_requested", "warning"
    pub passed: bool,
    pub summary: String,
    pub findings: Vec<ReviewFinding>,
}

pub fn review_diff_architecture(diff_content: &str, _changed_files: &[String]) -> CriticReview {
    let mut findings = Vec::new();
    let mut current_file = String::new();
    let mut line_num = 0usize;

    for line in diff_content.lines() {
        if line.starts_with("+++ b/") {
            current_file = line.trim_start_matches("+++ b/").to_string();
            line_num = 0;
            continue;
        } else if line.starts_with("--- ")
            || line.starts_with("diff --git")
            || line.starts_with("index ")
        {
            continue;
        } else if line.starts_with("@@ ") {
            if let Some(plus_idx) = line.find('+') {
                let sub = &line[plus_idx + 1..];
                let num_str: String = sub.chars().take_while(|c| c.is_ascii_digit()).collect();
                line_num = num_str.parse().unwrap_or(0);
            }
            continue;
        }

        let is_addition = line.starts_with('+');
        if is_addition {
            line_num += 1;
            let added_code = &line[1..];

            // 1. 架构红线：严禁 JPA / Hibernate (MyBatis-Plus 唯一)
            if current_file.ends_with(".java") {
                if added_code.contains("@Entity")
                    || added_code.contains("javax.persistence")
                    || added_code.contains("jakarta.persistence")
                    || added_code.contains("org.springframework.data.jpa")
                {
                    findings.push(ReviewFinding {
                        file: current_file.clone(),
                        line_hint: Some(line_num),
                        severity: "blocker".to_string(),
                        rule: "ARCH-01: No JPA/Hibernate".to_string(),
                        message: "DeepCloud 严禁使用 JPA/Hibernate，持久层必须统一使用 MyBatis-Plus LambdaQueryWrapper。".to_string(),
                    });
                }

                // 2. 契约红线：台账 #5 手写数字 code
                if added_code.contains(".put(\"code\", 200)")
                    || added_code.contains(".put(\"code\", 200L)")
                {
                    findings.push(ReviewFinding {
                        file: current_file.clone(),
                        line_hint: Some(line_num),
                        severity: "blocker".to_string(),
                        rule: "API-01: Standard Result Structure".to_string(),
                        message: "严禁返回数字型 code: 200，前端统一拦截器严格匹配字符串 \"200\"，请直接继承 BaseController 返回 Result.ok()。".to_string(),
                    });
                }

                // 3. 架构红线：禁止在 common 模块引入具体业务模块
                if current_file.starts_with("common/")
                    && (added_code.contains("import com.deepcloud.engine")
                        || added_code.contains("import com.deepcloud.iam")
                        || added_code.contains("import com.deepcloud.system"))
                {
                    findings.push(ReviewFinding {
                        file: current_file.clone(),
                        line_hint: Some(line_num),
                        severity: "blocker".to_string(),
                        rule: "ARCH-02: Common Circular Dependency".to_string(),
                        message: "严禁在 common 模块引入具体业务微服务，避免造成循环依赖。".to_string(),
                    });
                }
            }

            // 4. 前端红线：严禁硬编码固定颜色 (需使用 Tailwind 语义类)
            if current_file.ends_with(".tsx")
                || current_file.ends_with(".jsx")
                || current_file.ends_with(".ts")
            {
                let fixed_color_needles = [
                    "bg-amber-",
                    "bg-blue-",
                    "bg-red-",
                    "bg-green-",
                    "bg-emerald-",
                    "bg-purple-",
                    "text-amber-",
                    "text-blue-",
                    "text-emerald-",
                    "text-purple-",
                ];
                for needle in fixed_color_needles {
                    if added_code.contains(needle) {
                        findings.push(ReviewFinding {
                            file: current_file.clone(),
                            line_hint: Some(line_num),
                            severity: "warning".to_string(),
                            rule: "UI-01: Semantic Colors Required".to_string(),
                            message: format!("检测到硬编码 Tailwind 颜色类 '{}'，必须使用 Tailwind 语义类（如 bg-muted, text-muted-foreground, bg-primary）。", needle),
                        });
                        break;
                    }
                }
            }

            // 5. 安全红线：硬编码密码 / 敏感 Secret
            let secret_needles = [
                "password = \"",
                "secret_key = \"",
                "password: \"",
                "secretKey: \"",
            ];
            for s in secret_needles {
                if added_code.to_lowercase().contains(s) && !current_file.contains("test") {
                    findings.push(ReviewFinding {
                        file: current_file.clone(),
                        line_hint: Some(line_num),
                        severity: "blocker".to_string(),
                        rule: "SEC-01: No Hardcoded Secrets".to_string(),
                        message: "检测到疑似明文凭据/密码硬编码，凭据必须接入 Vault 或通过环境变量/配置文件外部化注入。".to_string(),
                    });
                    break;
                }
            }
        } else if !line.starts_with('-') {
            line_num += 1;
        }
    }

    let has_blocker = findings.iter().any(|f| f.severity == "blocker");
    let has_warning = findings.iter().any(|f| f.severity == "warning");

    let status = if has_blocker {
        "changes_requested".to_string()
    } else if has_warning {
        "warning".to_string()
    } else {
        "approved".to_string()
    };

    let passed = !has_blocker;
    let summary = if findings.is_empty() {
        "All architectural and convention checks passed with zero violations.".to_string()
    } else {
        format!(
            "Found {} finding(s): {} blocker(s), {} warning(s).",
            findings.len(),
            findings.iter().filter(|f| f.severity == "blocker").count(),
            findings.iter().filter(|f| f.severity == "warning").count()
        )
    };

    CriticReview {
        status,
        passed,
        summary,
        findings,
    }
}

pub(crate) async fn review_task_changes(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.task_manager.get_task(&task_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    };
    let paths = match development_run_paths(&state, &task) {
        Ok(paths) => paths,
        Err(error) => {
            return (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response()
        }
    };
    let changed_files: Vec<String> = crate::services::change_set::inspect_changes(
        &paths.baseline,
        &paths.project,
        &paths.run_dir,
    )
    .map(|cs| cs.files.into_iter().map(|f| f.path).collect())
    .unwrap_or_default();

    let output = tokio::process::Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--")
        .arg(&paths.project)
        .arg(&paths.run_dir)
        .output()
        .await;
    let diff_text = output
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let review = review_diff_architecture(&diff_text, &changed_files);
    Json(json!({ "review": review })).into_response()
}

pub(crate) async fn verify_task_changes(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.task_manager.get_task(&task_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    };
    let paths = match development_run_paths(&state, &task) {
        Ok(paths) => paths,
        Err(error) => {
            return (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response()
        }
    };
    let fingerprint = match crate::services::change_set::workspace_fingerprint(&paths.run_dir) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error })),
            )
                .into_response()
        }
    };
    let changed_files: Vec<String> = crate::services::change_set::inspect_changes(
        &paths.baseline,
        &paths.project,
        &paths.run_dir,
    )
    .map(|cs| cs.files.into_iter().map(|f| f.path).collect())
    .unwrap_or_default();

    // 1. 对抗审查门禁：扫描 Diff 中的架构红线 (Critic Review Gate)
    let diff_output = tokio::process::Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--")
        .arg(&paths.project)
        .arg(&paths.run_dir)
        .output()
        .await;
    let diff_text = diff_output
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let critic_review = review_diff_architecture(&diff_text, &changed_files);

    let plan = verification_plan_with_changes(&paths.run_dir, &changed_files);
    let mut results = Vec::new();
    let mut passed = !plan.is_empty() && critic_review.passed;
    for spec in plan {
        let started = std::time::Instant::now();
        let mut command = tokio::process::Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&paths.run_dir)
            .kill_on_drop(true);
        let outcome = tokio::time::timeout(
            Duration::from_secs(VERIFICATION_TIMEOUT_SECONDS),
            command.output(),
        )
        .await;
        let result = match outcome {
            Ok(Ok(output)) => {
                let success = output.status.success();
                passed &= success;
                json!({
                    "label": spec.label,
                    "program": spec.program,
                    "args": spec.args,
                    "status": if success { "passed" } else { "failed" },
                    "exitCode": output.status.code(),
                    "stdout": String::from_utf8_lossy(&output.stdout).chars().take(20_000).collect::<String>(),
                    "stderr": String::from_utf8_lossy(&output.stderr).chars().take(20_000).collect::<String>(),
                    "durationMs": started.elapsed().as_millis(),
                })
            }
            Ok(Err(error)) => {
                passed = false;
                json!({ "label": spec.label, "program": spec.program, "args": spec.args, "status": "failed", "error": error.to_string(), "durationMs": started.elapsed().as_millis() })
            }
            Err(_) => {
                passed = false;
                json!({ "label": spec.label, "program": spec.program, "args": spec.args, "status": "timeout", "durationMs": started.elapsed().as_millis() })
            }
        };
        results.push(result);
    }
    let status = if results.is_empty() && critic_review.passed {
        "skipped"
    } else if passed {
        "passed"
    } else {
        "failed"
    };
    let receipt = json!({
        "schemaVersion": 1,
        "kind": "verification",
        "status": status,
        "sourceFingerprint": fingerprint,
        "completedAt": chrono::Utc::now().to_rfc3339(),
        "commands": results,
        "criticReview": critic_review,
    });
    state
        .task_manager
        .update_verification(&task_id, receipt.clone());
    Json(json!({ "success": status == "passed", "verification": receipt })).into_response()
}

pub(crate) async fn apply_task_changes(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    payload: Option<Json<ApplyBody>>,
) -> Response {
    let Some(task) = state.task_manager.get_task(&task_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
            .into_response();
    };
    if task.approval.as_deref() == Some("approved") {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Change set has already been applied", "kind": "already_applied" })),
        )
            .into_response();
    }
    let paths = match development_run_paths(&state, &task) {
        Ok(paths) => paths,
        Err(error) => {
            return (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response()
        }
    };
    let body = payload.map(|Json(body)| body).unwrap_or_default();
    let forced = body.force.unwrap_or(false);
    let proposed_fingerprint =
        match crate::services::change_set::workspace_fingerprint(&paths.run_dir) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response()
            }
        };
    let verification_passed = task.verification.as_ref().is_some_and(|receipt| {
        receipt.get("status").and_then(|value| value.as_str()) == Some("passed")
            && receipt
                .get("sourceFingerprint")
                .and_then(|value| value.as_str())
                == Some(proposed_fingerprint.as_str())
    });
    if !verification_passed && !forced {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "A passing, current Verification Receipt is required before Apply",
                "kind": "verification_required",
            })),
        )
            .into_response();
    }
    let change_set = match crate::services::change_set::inspect_changes(
        &paths.baseline,
        &paths.project,
        &paths.run_dir,
    ) {
        Ok(change_set) => change_set,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error })),
            )
                .into_response()
        }
    };
    if change_set.files.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Agent task produced no file changes", "kind": "empty_change_set" })),
        )
            .into_response();
    }
    if !change_set.conflicts.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Original workspace has conflicting changes", "kind": "workspace_conflict", "conflicts": change_set.conflicts })),
        )
            .into_response();
    }
    let snapshot_hash =
        match crate::services::git_service::record_ai_snapshot(
            &task.project_path,
            &format!("Before applying AI task {task_id}"),
        ) {
            Ok(hash) => hash,
            Err(error) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    json!({ "error": format!("Cannot create pre-apply shadow snapshot: {error}") }),
                ),
            )
                .into_response(),
        };
    let apply_receipt = match crate::services::change_set::apply_changes(
        &change_set,
        &paths.project,
        &paths.run_dir,
        body.files.as_deref(),
        forced,
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": error, "kind": "apply_rejected" })),
            )
                .into_response()
        }
    };
    state.task_manager.update_development_run(
        &task_id,
        Some(json!(change_set)),
        Some("approved".to_string()),
    );

    let applied_count = apply_receipt.applied_files.len() + apply_receipt.deleted_files.len();
    let summary =
        format!("AI Changes Applied: {task_id}. Files changed: {applied_count}. Forced: {forced}");

    // Save to supermemory
    let store = crate::services::supermemory_store::SupermemoryStore::new(
        &state.runtime_config,
        Some(&task.project_path),
    );
    let now = chrono::Utc::now().to_rfc3339();
    let record = crate::models::memory::SupermemoryRecord {
        id: uuid::Uuid::new_v4().to_string(),
        file_path: "".to_string(),
        name: format!("AI Application Task: {}", task_id),
        summary: serde_json::to_string(&json!({
            "summary": summary,
            "snapshotHash": snapshot_hash,
            "apply": apply_receipt,
        }))
        .unwrap_or(summary),
        tags: vec!["ai_applied_change".to_string(), task_id.clone()],
        complexity: "low".to_string(),
        node_type: "ai_applied_change".to_string(),
        source_graph_path: "".to_string(),
        source_label: "".to_string(),
        updated_at: now,
        embedding: None,
    };
    let memory_synced = store.write_memory_result(record).is_ok();

    Json(json!({
        "status": "success",
        "files_applied": applied_count,
        "snapshotHash": snapshot_hash,
        "memorySynced": memory_synced,
        "receipt": apply_receipt,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_plan_with_java_test_changes() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path();
        std::fs::write(run_dir.join("pom.xml"), "<project></project>").unwrap();

        let changed = vec!["common/src/test/java/common/agent/AgentSdkTest.java".to_string()];
        let plan = verification_plan_with_changes(run_dir, &changed);

        assert!(!plan.is_empty());
        let first = &plan[0];
        assert!(first.label.contains("TDD Targeted Maven Test: AgentSdkTest"));
        assert_eq!(first.args, vec!["-pl", "common", "test", "-Dtest=AgentSdkTest"]);
    }

    #[test]
    fn test_verification_plan_with_frontend_test_changes() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path();
        std::fs::write(run_dir.join("package.json"), r#"{"scripts":{"test":"vitest"}}"#).unwrap();

        let changed = vec!["src/app/agent.test.ts".to_string()];
        let plan = verification_plan_with_changes(run_dir, &changed);

        assert!(!plan.is_empty());
        let first = &plan[0];
        assert!(first.label.contains("TDD Targeted Frontend Test"));
    }

    #[test]
    fn test_review_diff_architecture_detects_jpa_and_colors() {
        let diff_content = r#"
diff --git a/service/src/main/java/com/deepcloud/User.java b/service/src/main/java/com/deepcloud/User.java
--- a/service/src/main/java/com/deepcloud/User.java
+++ b/service/src/main/java/com/deepcloud/User.java
@@ -1,3 +1,5 @@
+import javax.persistence.Entity;
+@Entity
 public class User {}
diff --git a/ui/src/App.tsx b/ui/src/App.tsx
--- a/ui/src/App.tsx
+++ b/ui/src/App.tsx
@@ -10,3 +10,4 @@
+  return <div className="bg-amber-500">Hello</div>;
"#;
        let review = review_diff_architecture(diff_content, &[]);
        assert!(!review.passed);
        assert_eq!(review.status, "changes_requested");
        assert_eq!(review.findings.len(), 2);

        let jpa_finding = review.findings.iter().find(|f| f.rule.contains("JPA")).unwrap();
        assert_eq!(jpa_finding.severity, "blocker");
        assert!(jpa_finding.message.contains("MyBatis-Plus"));

        let color_finding = review.findings.iter().find(|f| f.rule.contains("UI-01")).unwrap();
        assert_eq!(color_finding.severity, "warning");
        assert!(color_finding.message.contains("bg-amber-"));
    }
}

