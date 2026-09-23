use super::*;
use std::str::FromStr;

#[test]
fn test_level_parsing() {
    assert_eq!(
        FilterLevel::from_str("aggressive").unwrap(),
        FilterLevel::Aggressive
    );
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
    let content = "x";
    let r = compress(content, "a.rs", FilterLevel::Aggressive);
    assert_eq!(r.content, "x");
    assert!(r.compressed_bytes <= r.original_bytes);
}

#[test]
fn test_smart_log_preserves_error_stacks() {
    let mut log = String::new();
    for i in 0..100 {
        if i == 50 {
            log.push_str("ERROR: Failed to establish database connection\n");
            log.push_str("   at src/db/connect.rs:42\n");
        } else {
            log.push_str(&format!("INFO: Regular log message step {}\n", i));
        }
    }
    let res = compress_terminal_output(&log, 30);
    assert!(res
        .content
        .contains("ERROR: Failed to establish database connection"));
    assert!(res.content.contains("at src/db/connect.rs:42"));
    assert!(res.content.contains("[RTK SmartLog omitted"));
    assert!(res.reduction_percent > 0.0);
}

#[test]
fn test_terminal_folding_strips_maven_downloads_and_pins_caused_by() {
    let mut log = String::new();
    log.push_str("[INFO] Scanning for projects...\n");
    log.push_str("[INFO] -------------------< com.example:demo >--------------------\n");
    for i in 0..50 {
        log.push_str(&format!("[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/dep{}/1.0/dep{}-1.0.jar\n", i, i));
        log.push_str(&format!("Progress (1): {} KB\n", i * 10));
        log.push_str(&format!("[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/dep{}/1.0/dep{}-1.0.jar\n", i, i));
    }
    log.push_str("[ERROR] Failed to execute goal org.springframework.boot:spring-boot-maven-plugin:run\n");
    log.push_str("[ERROR] org.springframework.beans.factory.BeanCreationException: Error creating bean\n");
    log.push_str("[ERROR] Caused by: java.lang.NullPointerException: datasource url is required\n");
    log.push_str("    at com.example.demo.config.DbConfig.setup(DbConfig.java:45)\n");
    log.push_str("[INFO] ------------------------------------------------------------------------\n");
    log.push_str("[INFO] BUILD FAILURE\n");
    log.push_str("[INFO] Total time: 12.345 s\n");

    let res = compress_terminal_output(&log, 40);
    assert!(!res.content.contains("Downloading from central"));
    assert!(!res.content.contains("Progress (1):"));
    assert!(res
        .content
        .contains("Caused by: java.lang.NullPointerException"));
    assert!(res.content.contains("BUILD FAILURE"));
    assert!(res.content.contains("[RTK SmartLog omitted"));
    assert!(res.reduction_percent > 50.0);
}

#[test]
fn test_compress_git_status() {
    let status_raw = r#"On branch feature/new-graph
Changes to be committed:
  (use "git restore --staged <file>..." to unstage)
	modified:   src/services/graph_service.rs
	new file:   src/services/code_intelligence.rs

Changes not staged for commit:
  (use "git add <file>..." to update what will be committed)
  (use "git restore <file>..." to discard changes in working directory)
	modified:   README.md

Untracked files:
  (use "git add <file>..." to include in what will be committed)
	scratch/test_notes.txt
"#;
    let res = compress_git_status(status_raw);
    assert!(res
        .content
        .contains("[Git Status] branch: feature/new-graph"));
    assert!(res
        .content
        .contains("Staged: 2, Modified: 1, Untracked: 1"));
    assert!(res.content.contains("src/services/graph_service.rs"));
    assert!(!res.content.contains("to unstage"));
}

#[test]
fn test_compress_git_diff_omits_lockfile() {
    let diff_raw = r#"diff --git a/Cargo.lock b/Cargo.lock
index abc..def 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -10,3 +10,5 @@
+new-dep 1.0.0
+new-dep-sub 2.0.0
-old-dep 0.9.0
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-    println!("old");
+    println!("new");
 }
"#;
    let res = compress_git_diff(diff_raw);
    assert!(res
        .content
        .contains("Cargo.lock [Generated/Lockfile diff omitted"));
    assert!(res.content.contains("println!(\"new\");"));
}

#[test]
fn test_compress_test_output_omits_passing_tests() {
    let mut test_out = String::new();
    test_out.push_str("running 100 tests\n");
    for i in 0..98 {
        test_out.push_str(&format!("test tests::test_{} ... ok\n", i));
    }
    test_out.push_str("test tests::test_fail_1 ... FAILED\n");
    test_out.push_str("test tests::test_fail_2 ... FAILED\n");
    test_out.push_str("\nfailures:\n");
    test_out.push_str("---- tests::test_fail_1 stdout ----\n");
    test_out.push_str("thread 'main' panicked at 'assertion failed: 1 == 2'\n");
    test_out.push_str("test result: FAILED. 2 failed; 98 passed;\n");

    let res = compress_test_output(&test_out);
    assert!(res
        .content
        .contains("[RTK TestFilter] 98 passing tests omitted"));
    assert!(res.content.contains("test tests::test_fail_1 ... FAILED"));
    assert!(res
        .content
        .contains("panicked at 'assertion failed: 1 == 2'"));
    assert!(res.content.contains("test result: FAILED"));
    assert!(res.reduction_percent > 50.0);
}

#[test]
fn test_compress_by_command_and_tracker() {
    TokenSavingsTracker::global().reset();
    let cmd = "git status";
    let status_raw =
        "On branch master\nChanges not staged for commit:\n  (use git...)\n\tmodified:   foo.rs\n";
    let res = compress_by_command(cmd, status_raw);
    assert_eq!(res.level, "git_status");

    let clippy_raw = "   Compiling app v0.1.0\nwarning: variable does not need to be mutable\n --> src/lib.rs:5:9\n";
    let clippy_res = compress_by_command("cargo clippy", clippy_raw);
    assert_eq!(clippy_res.level, "pipeline");
    assert!(!clippy_res.content.contains("Compiling"));
    assert!(clippy_res.content.contains("warning: variable"));

    let stats = TokenSavingsTracker::global().get_stats();
    assert_eq!(stats.total_invocations, 2);
    assert!(stats.total_original_bytes > 0);
}

#[test]
fn test_compress_tsc_output() {
    let raw = r#"
src/app.ts:12:5 - error TS2322: Type 'number' is not assignable to type 'string'.
12     name = 123;
            ~~~~
src/routes.ts:30:10 - error TS2554: Expected 2 arguments, but got 1.
30     call();
            ~
Found 2 errors in 2 files.
"#;
    let res = compress_tsc_output(raw);
    assert_eq!(res.level, "tsc_output");
    assert!(res.content.contains("src/app.ts:12:5 - error TS2322"));
    assert!(res.content.contains("src/routes.ts:30:10 - error TS2554"));
    assert!(res.content.contains("Found 2 errors in 2 files."));
    assert!(!res.content.contains("~~~~"));
    assert!(res.reduction_percent > 20.0);
}

#[test]
fn test_compress_pytest_output() {
    let raw = r#"
============================= test session starts ==============================
rootdir: /repo
collected 50 items

tests/test_basic.py .................................................... [100%]
=================================== FAILURES ===================================
_________________________________ test_login ___________________________________
    def test_login():
>       assert response.status_code == 200
E       AssertionError: assert 401 == 200
tests/test_basic.py:25: AssertionError
=========================== short test summary info ============================
FAILED tests/test_basic.py::test_login - AssertionError: assert 401 == 200
========================= 1 failed, 49 passed in 0.45s =========================
"#;
    let res = compress_pytest_output(raw);
    assert_eq!(res.level, "pytest_output");
    assert!(res
        .content
        .contains("FAILED tests/test_basic.py::test_login"));
    assert!(res.content.contains("AssertionError: assert 401 == 200"));
    assert!(res.reduction_percent > 30.0);
}

#[test]
fn test_compress_table_output() {
    let raw = r#"
CONTAINER ID   IMAGE          COMMAND                  CREATED         STATUS         PORTS                    NAMES
a1b2c3d4e5f6   nginx:latest   "/docker-entrypoint.…"   2 hours ago     Up 2 hours     0.0.0.0:80->80/tcp       web-server
f6e5d4c3b2a1   redis:alpine   "docker-entrypoint.s…"   5 hours ago     Up 5 hours     0.0.0.0:6379->6379/tcp   redis-cache
"#;
    let res = compress_table_output(raw);
    assert_eq!(res.level, "table");
    assert!(res.content.contains("CONTAINER | ID | IMAGE | COMMAND"));
    assert!(res.content.contains("a1b2c3d4e5f6 | nginx:latest"));
    assert!(res.reduction_percent > 5.0);
}
