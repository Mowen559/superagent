use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_five_gate_filtering() {
    // 门 0: 忽略目录
    assert!(!CodeReviewEngine::is_file_eligible_for_review("target/debug/app.rs"));
    assert!(!CodeReviewEngine::is_file_eligible_for_review("node_modules/pkg/index.js"));
    assert!(!CodeReviewEngine::is_file_eligible_for_review(".git/config"));

    // 门 1: 二进制与静态资源
    assert!(!CodeReviewEngine::is_file_eligible_for_review("assets/logo.png"));
    assert!(!CodeReviewEngine::is_file_eligible_for_review("doc/spec.pdf"));
    assert!(!CodeReviewEngine::is_file_eligible_for_review("Cargo.lock"));

    // 门 2: 测试文件排除
    assert!(!CodeReviewEngine::is_file_eligible_for_review("src/domain_test.rs"));
    assert!(!CodeReviewEngine::is_file_eligible_for_review("pkg/auth/login_test.go"));
    assert!(!CodeReviewEngine::is_file_eligible_for_review("web/components/Button.test.tsx"));
    assert!(!CodeReviewEngine::is_file_eligible_for_review("src/tests/integration.rs"));

    // 门 3: 白名单代码文件
    assert!(CodeReviewEngine::is_file_eligible_for_review("src/main.rs"));
    assert!(CodeReviewEngine::is_file_eligible_for_review("internal/service/order.go"));
    assert!(CodeReviewEngine::is_file_eligible_for_review("src/components/Header.tsx"));
    assert!(CodeReviewEngine::is_file_eligible_for_review("scripts/deploy.py"));
}

#[test]
fn test_builtin_rules_detection() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("vulnerable.rs");
    let code = r#"
fn process(id: &str) {
    let sql = format!("SELECT * FROM users WHERE id = '{}'", id);
    let val: Option<String> = None;
    let res = val.unwrap();
}
"#;
    fs::write(&file_path, code).unwrap();

    let engine = CodeReviewEngine::new(temp.path().to_path_buf());
    let comments = engine.scan_file_content("vulnerable.rs", code);

    assert!(comments.len() >= 2);
    assert!(comments.iter().any(|c| c.rule_id == Some("SEC-SQLI-001".to_string())));
    assert!(comments.iter().any(|c| c.rule_id == Some("BUG-NPE-001".to_string())));
}

#[test]
fn test_review_workspace_report() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("service.rs");
    let code = "fn run() {\n    // clean code\n}\n";
    fs::write(&file_path, code).unwrap();

    let engine = CodeReviewEngine::new(temp.path().to_path_buf());
    let report = engine.review_workspace(Some(&["service.rs".to_string()])).unwrap();

    assert_eq!(report.mode, "workspace");
    assert_eq!(report.total_files_analyzed, 1);
    assert_eq!(report.risk_level, "LOW");
}

#[test]
fn test_review_current_project_workspace() {
    let cwd = std::env::current_dir().unwrap();
    let engine = CodeReviewEngine::new(cwd);
    let report = engine.review_workspace(None).unwrap();
    assert_eq!(report.mode, "workspace");
    assert!(report.total_files_analyzed > 0 || report.filtered_files_count > 0 || !report.summary.is_empty());
}

