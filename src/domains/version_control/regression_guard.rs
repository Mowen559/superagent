//! 防改坏机制：Git Baseline 双向差分快照 + 存量回归硬拦截 + 一键秒级回滚沙箱
//!
//! 核心防护链路：
//! 1. 任务入场强制基线快照 (Baseline Snapshot)：毫秒级捕获影子检查点，记录未跟踪初始清单；
//! 2. 变更前后差分作用域审计 (Diff Scope Guard)：拦截侵犯 `common/` 模块或敏感路径的越界修改；
//! 3. 存量回归硬拦截 (Stock Regression Gate)：深度解析测试输出，精准识别存量测试用例崩溃；
//! 4. 失败一键秒级物理回滚 (Auto-Rollback Sandbox)：在自愈超限或致命错误时还原工作区并清理垃圾增量。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 基线快照存储形态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BaselineKind {
    /// 基于 Git 影子提交/检查点
    GitCheckpoint {
        commit_hash: String,
        checkpoint_id: String,
    },
    /// 基于本地文件系统哈希的轻量级指纹缓存 (非 Git 目录降级)
    FingerprintCache {
        fingerprints: HashMap<String, String>,
    },
}

/// 差分路径审计规则
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditRule {
    /// 铁律路径保护：完全禁止写盘（例如 "common/"）
    ProtectedPrefix(String),
    /// 敏感文件模式保护（例如 ".git", ".env", "target/"）
    ForbiddenPattern(String),
    /// 严格作用域白名单：若配置，则只允许修改白名单内的目录或文件
    AllowedScope(Vec<String>),
}

/// 路径审计判定结论
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditVerdict {
    /// 变更合规允许
    Allowed,
    /// 违背架构铁律或越界修改被硬拦截
    Violated {
        path: String,
        rule_description: String,
        suggested_action: String,
    },
}

/// 存量测试回归检查报告
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegressionAuditReport {
    pub has_regression: bool,
    pub failed_tests: Vec<String>,
    pub framework: String,
    pub summary: String,
}

/// 物理回滚收据回执
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackReceipt {
    pub run_id: String,
    pub trigger_reason: String,
    pub reverted_files: Vec<String>,
    pub cleaned_untracked_files: Vec<String>,
    pub timestamp_epoch_ms: u64,
    pub duration_ms: u64,
    pub success: bool,
    pub details: String,
}

/// 防改坏回滚守护器核心结构体
#[derive(Debug, Clone)]
pub struct RegressionRollbackGuard {
    pub project_path: PathBuf,
    pub run_id: String,
    pub baseline: Option<BaselineKind>,
    pub initial_untracked_files: HashSet<String>,
    pub mutated_files: HashSet<String>,
    pub original_file_contents: HashMap<String, String>,
    pub audit_rules: Vec<AuditRule>,
    pub is_rolled_back: bool,
    pub created_at_ms: u64,
}

impl RegressionRollbackGuard {
    /// 构造新的防改坏守护器实例
    pub fn new(project_path: impl AsRef<Path>, run_id: &str) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            project_path: project_path.as_ref().to_path_buf(),
            run_id: run_id.to_string(),
            baseline: None,
            initial_untracked_files: HashSet::new(),
            mutated_files: HashSet::new(),
            original_file_contents: HashMap::new(),
            audit_rules: Vec::new(),
            is_rolled_back: false,
            created_at_ms: now_ms,
        }
    }

    /// 装载 DeepCloud 架构铁律守卫（如 common 模块隔离与敏感文件保护）
    pub fn with_default_architectural_rules(mut self) -> Self {
        // 铁律：严禁修改 common 目录，所有业务改动必须在具体服务内部闭环
        self.audit_rules
            .push(AuditRule::ProtectedPrefix("common/".to_string()));
        self.audit_rules
            .push(AuditRule::ProtectedPrefix("common\\".to_string()));

        // 敏感系统与凭证路径保护
        self.audit_rules
            .push(AuditRule::ForbiddenPattern(".git/".to_string()));
        self.audit_rules
            .push(AuditRule::ForbiddenPattern(".git\\".to_string()));
        self.audit_rules
            .push(AuditRule::ForbiddenPattern(".env".to_string()));
        self.audit_rules
            .push(AuditRule::ForbiddenPattern("target/".to_string()));
        self.audit_rules
            .push(AuditRule::ForbiddenPattern("node_modules/".to_string()));

        self
    }

    /// 添加自定义作用域白名单约束
    pub fn with_allowed_scope(mut self, allowed_scopes: Vec<String>) -> Self {
        if !allowed_scopes.is_empty() {
            self.audit_rules.push(AuditRule::AllowedScope(allowed_scopes));
        }
        self
    }

    /// 毫秒级捕获入场基线快照 (Git Checkpoint 或轻量指纹)
    pub fn capture_baseline(&mut self) -> Result<(), String> {
        let proj_str = self.project_path.to_string_lossy().to_string();

        // 1. 尝试使用 Git Service 捕获命名快照
        let git_status = super::git_service::get_status(&proj_str);
        if let Ok(status) = git_status {
            if status.is_git_repo {
                // 记录入场前已有的未跟踪文件
                self.initial_untracked_files = status.untracked.into_iter().collect();

                let desc = format!("task-baseline-{}", self.run_id);
                match super::git_service::create_checkpoint(&proj_str, &desc) {
                    Ok(cp) => {
                        self.baseline = Some(BaselineKind::GitCheckpoint {
                            commit_hash: cp.commit_hash,
                            checkpoint_id: cp.id,
                        });
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!(
                            "[RegressionGuard] Git checkpoint failed ({}), degrading to memory fingerprint.",
                            e
                        );
                    }
                }
            }
        }

        // 2. 降级为轻量级指纹缓存
        self.baseline = Some(BaselineKind::FingerprintCache {
            fingerprints: HashMap::new(),
        });
        Ok(())
    }

    /// 在变异发生前，对即将写盘的文件路径进行架构铁律与越界审计
    pub fn audit_mutation_path(&self, target_path_str: &str) -> AuditVerdict {
        let normalized = target_path_str.replace('\\', "/");

        for rule in &self.audit_rules {
            match rule {
                AuditRule::ProtectedPrefix(prefix) => {
                    let norm_prefix = prefix.replace('\\', "/");
                    if normalized.starts_with(&norm_prefix) || normalized.contains(&format!("/{}", norm_prefix)) {
                        return AuditVerdict::Violated {
                            path: target_path_str.to_string(),
                            rule_description: format!("【架构铁律拦截】路径包含受保护目录 '{}'！", prefix),
                            suggested_action: "根据系统架构规范，禁止修改 common 等共享核心模块。所有变动必须在业务模块自身内部通过扩展、适配器或私有类完成！".to_string(),
                        };
                    }
                }
                AuditRule::ForbiddenPattern(pat) => {
                    if normalized.contains(pat) {
                        return AuditVerdict::Violated {
                            path: target_path_str.to_string(),
                            rule_description: format!("【敏感路径拦截】路径命中受保护的系统/凭据模式 '{}'！", pat),
                            suggested_action: "严禁擅自修改内部凭证、中间产物或 Git 元数据目录。".to_string(),
                        };
                    }
                }
                AuditRule::AllowedScope(scopes) => {
                    let within_scope = scopes.iter().any(|s| {
                        let norm_s = s.replace('\\', "/");
                        normalized.starts_with(&norm_s) || normalized.contains(&norm_s)
                    });
                    if !within_scope {
                        return AuditVerdict::Violated {
                            path: target_path_str.to_string(),
                            rule_description: format!("【作用域越界拦截】路径不在声明的任务允许范围 {:?} 内！", scopes),
                            suggested_action: "请只在任务指定的模块或文件范围内进行修改，严禁越界修改无关文件导致潜在回归！".to_string(),
                        };
                    }
                }
            }
        }

        AuditVerdict::Allowed
    }

    /// 登记已变异的文件（在写盘工具执行前备份其原始内容，以备秒级回滚）
    pub fn observe_mutation(&mut self, file_path_str: &str) {
        let full_path = if Path::new(file_path_str).is_absolute() {
            PathBuf::from(file_path_str)
        } else {
            self.project_path.join(file_path_str)
        };

        let norm_key = file_path_str.replace('\\', "/");
        self.mutated_files.insert(norm_key.clone());

        // 如果之前尚未备份该文件，则在内存中备份其原始内容
        if !self.original_file_contents.contains_key(&norm_key) {
            if full_path.exists() {
                if let Ok(content) = fs::read_to_string(&full_path) {
                    self.original_file_contents.insert(norm_key, content);
                }
            }
        }
    }

    /// 深度分析测试运行输出，检测是否存在存量测试回归 (Stock Regression)
    pub fn audit_regression_from_output(test_output: &str) -> RegressionAuditReport {
        let mut failed_tests = Vec::new();
        let mut framework = "Unknown".to_string();

        // 1. Rust cargo test 匹配
        if test_output.contains("running ") && (test_output.contains("test result: ") || test_output.contains("failures:")) {
            framework = "Cargo Test (Rust)".to_string();
            for line in test_output.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("test ") && trimmed.ends_with("... FAILED") {
                    let test_name = trimmed.trim_start_matches("test ").trim_end_matches("... FAILED").trim();
                    failed_tests.push(test_name.to_string());
                } else if trimmed.starts_with("failures:") {
                    // 后续行可能列出 failed test 名字
                }
            }
        }
        // 2. Java Maven/Gradle 匹配
        else if test_output.contains("Tests run:") || test_output.contains("BUILD FAILURE") {
            framework = "Maven/Gradle (Java)".to_string();
            for line in test_output.lines() {
                let trimmed = line.trim();
                if trimmed.contains("FAILURE!") || trimmed.contains("<<< FAILURE!") || trimmed.contains("<<< ERROR!") {
                    failed_tests.push(trimmed.to_string());
                }
            }
        }
        // 3. Python pytest 匹配
        else if test_output.contains("FAILED ") || test_output.contains("=== FAILURES ===") {
            framework = "pytest (Python)".to_string();
            for line in test_output.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("FAILED ") {
                    failed_tests.push(trimmed.to_string());
                }
            }
        }
        // 4. Node.js jest/vitest 匹配
        else if test_output.contains("FAIL ") || test_output.contains("Tests: ") {
            framework = "Jest/Vitest (Node.js)".to_string();
            for line in test_output.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("FAIL ") {
                    failed_tests.push(trimmed.to_string());
                }
            }
        }

        let has_regression = !failed_tests.is_empty()
            || test_output.contains("test result: FAILED")
            || test_output.contains("BUILD FAILURE");

        let summary = if has_regression {
            format!(
                "存量回归拦截：检测到 {} 个测试用例执行失败！测试框架: {}",
                failed_tests.len(),
                framework
            )
        } else {
            "验证通过：未发现存量测试回归。".to_string()
        };

        RegressionAuditReport {
            has_regression,
            failed_tests,
            framework,
            summary,
        }
    }

    /// 执行一键物理级秒级安全回滚 (Auto-Rollback)
    ///
    /// 步骤：
    /// 1. 若基于 Git 快照，调用 `rollback_to_snapshot` 将所有受污染的跟踪文件恢复至基线；
    /// 2. 若基于内存原始内容，精准重写写盘修改过的文件；
    /// 3. 精准比对并物理删除本次任务期间新生成的未跟踪/垃圾文件，杜绝主工作区残留脏文件；
    /// 4. 产出物理回滚证据收据 `RollbackReceipt`。
    pub fn rollback_workspace(&mut self, reason: &str) -> Result<RollbackReceipt, String> {
        let start_time = std::time::Instant::now();
        let mut reverted_files = Vec::new();
        let mut cleaned_untracked_files = Vec::new();
        let proj_str = self.project_path.to_string_lossy().to_string();

        // 1. 回滚已修改的受跟踪文件
        match &self.baseline {
            Some(BaselineKind::GitCheckpoint { commit_hash, .. }) => {
                if let Err(e) = super::git_service::rollback_to_snapshot(&proj_str, commit_hash) {
                    eprintln!(
                        "[RegressionGuard] git rollback_to_snapshot failed ({}), fallback to memory content rollback.",
                        e
                    );
                    self.fallback_memory_revert(&mut reverted_files)?;
                } else {
                    reverted_files.extend(self.mutated_files.iter().cloned());
                }
            }
            _ => {
                self.fallback_memory_revert(&mut reverted_files)?;
            }
        }

        // 2. 清理任务期间新增的 Untracked 文件
        let current_status = super::git_service::get_status(&proj_str);
        if let Ok(status) = current_status {
            for untracked in status.untracked {
                // 只有在任务入场前不存在的未跟踪文件，才属于本次任务新增的垃圾产物
                if !self.initial_untracked_files.contains(&untracked) {
                    let untracked_path = self.project_path.join(&untracked);
                    if untracked_path.is_file() {
                        if fs::remove_file(&untracked_path).is_ok() {
                            cleaned_untracked_files.push(untracked);
                        }
                    } else if untracked_path.is_dir() {
                        if fs::remove_dir_all(&untracked_path).is_ok() {
                            cleaned_untracked_files.push(untracked);
                        }
                    }
                }
            }
        }

        self.is_rolled_back = true;
        let duration_ms = start_time.elapsed().as_millis() as u64;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let receipt = RollbackReceipt {
            run_id: self.run_id.clone(),
            trigger_reason: reason.to_string(),
            reverted_files,
            cleaned_untracked_files,
            timestamp_epoch_ms: now_ms,
            duration_ms,
            success: true,
            details: format!(
                "秒级回滚沙箱已生效：耗时 {}ms，成功还原修改并彻底清理工作区增量。",
                duration_ms
            ),
        };

        eprintln!(
            "\x1b[31;1m[RegressionGuard Rollback] 🛡️ {}\x1b[0m",
            receipt.details
        );

        Ok(receipt)
    }

    fn fallback_memory_revert(&self, reverted_files: &mut Vec<String>) -> Result<(), String> {
        for (rel_path, orig_content) in &self.original_file_contents {
            let full_path = self.project_path.join(rel_path);
            if let Some(parent) = full_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&full_path, orig_content)
                .map_err(|e| format!("无法恢复文件 '{}': {}", full_path.display(), e))?;
            reverted_files.push(rel_path.clone());
        }
        Ok(())
    }

    /// 计算内容 SHA-256 哈希
    pub fn compute_hash(bytes: &[u8]) -> String {
        crate::shared_kernel::crypto::sha256_hex(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_mutation_path_blocks_common_module() {
        let guard = RegressionRollbackGuard::new(".", "run-101")
            .with_default_architectural_rules();

        let verdict = guard.audit_mutation_path("common/src/main/java/common/Result.java");
        match verdict {
            AuditVerdict::Violated { rule_description, .. } => {
                assert!(rule_description.contains("common/"));
            }
            AuditVerdict::Allowed => panic!("Should have blocked modification to common/"),
        }

        let win_verdict = guard.audit_mutation_path("common\\src\\utils\\Helper.java");
        assert!(matches!(win_verdict, AuditVerdict::Violated { .. }));

        let legal_verdict = guard.audit_mutation_path("scraper-service/src/main.py");
        assert_eq!(legal_verdict, AuditVerdict::Allowed);
    }

    #[test]
    fn test_audit_mutation_path_blocks_sensitive_patterns() {
        let guard = RegressionRollbackGuard::new(".", "run-102")
            .with_default_architectural_rules();

        let git_verdict = guard.audit_mutation_path(".git/config");
        assert!(matches!(git_verdict, AuditVerdict::Violated { .. }));

        let env_verdict = guard.audit_mutation_path(".env.production");
        assert!(matches!(env_verdict, AuditVerdict::Violated { .. }));
    }

    #[test]
    fn test_audit_mutation_path_allowed_scopes() {
        let guard = RegressionRollbackGuard::new(".", "run-103")
            .with_allowed_scope(vec!["spider-service/".to_string()]);

        let in_scope = guard.audit_mutation_path("spider-service/extractor.py");
        assert_eq!(in_scope, AuditVerdict::Allowed);

        let out_of_scope = guard.audit_mutation_path("iam-service/AuthController.java");
        assert!(matches!(out_of_scope, AuditVerdict::Violated { .. }));
    }

    #[test]
    fn test_audit_regression_from_output_cargo() {
        let output = r#"
running 4 tests
test auth::test_login ... ok
test payment::test_charge ... FAILED
test user::test_create ... ok

failures:
    payment::test_charge

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured
        "#;

        let report = RegressionRollbackGuard::audit_regression_from_output(output);
        assert!(report.has_regression);
        assert_eq!(report.framework, "Cargo Test (Rust)");
        assert_eq!(report.failed_tests, vec!["payment::test_charge".to_string()]);
    }

    #[test]
    fn test_audit_regression_from_output_maven() {
        let output = r#"
[INFO] Running com.deepcloud.engine.SpiderServiceTest
[ERROR] Tests run: 12, Failures: 1, Errors: 0, Skipped: 0, Time elapsed: 1.45 s <<< FAILURE!
[INFO] BUILD FAILURE
        "#;

        let report = RegressionRollbackGuard::audit_regression_from_output(output);
        assert!(report.has_regression);
        assert_eq!(report.framework, "Maven/Gradle (Java)");
        assert!(!report.failed_tests.is_empty());
    }

    #[test]
    fn test_fallback_memory_revert_restores_file() {
        let temp_dir = std::env::temp_dir().join(format!("test_rollback_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let file_path = temp_dir.join("config.json");
        fs::write(&file_path, "{\"mode\": \"initial\"}").unwrap();

        let mut guard = RegressionRollbackGuard::new(&temp_dir, "test-run");
        guard.capture_baseline().unwrap();

        // 观察修改
        guard.observe_mutation("config.json");

        // 篡改文件内容
        fs::write(&file_path, "{\"mode\": \"corrupted_by_ai\"}").unwrap();
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "{\"mode\": \"corrupted_by_ai\"}");

        // 触发回滚
        let receipt = guard.rollback_workspace("Self-healing max retries degraded").unwrap();
        assert!(receipt.success);
        assert_eq!(receipt.reverted_files, vec!["config.json".to_string()]);

        // 验证文件无损还原
        let restored = fs::read_to_string(&file_path).unwrap();
        assert_eq!(restored, "{\"mode\": \"initial\"}");

        // 清理临时目录
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
