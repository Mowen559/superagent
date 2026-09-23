use super::*;
use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::json;
use std::path::Path;

#[test]
fn test_path_traversal_prevention() {
    let tool = WriteFileTool {
        workspace_root: std::env::current_dir().unwrap(),
    };
    let malicious_path = if cfg!(windows) {
        "C:\\Windows\\System32\\drivers\\etc\\hosts"
    } else {
        "../../../../etc/passwd"
    };

    let args = json!({
        "TargetFile": malicious_path,
        "CodeContent": "malicious"
    });

    let result = tool.execute(args);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Security Error"));
}

#[test]
fn read_and_list_reject_paths_outside_workspace() {
    let outside = if cfg!(windows) {
        "C:\\Windows\\win.ini"
    } else {
        "/etc/hosts"
    };
    assert!(ReadFileTool {
        workspace_root: std::env::current_dir().unwrap()
    }
    .execute(json!({ "AbsolutePath": outside }))
    .is_err());
    assert!(ListDirectoryTool {
        workspace_root: std::env::current_dir().unwrap()
    }
    .execute(json!({ "path": outside }))
    .is_err());
}

#[test]
fn test_collect_file_diagnostics() {
    let valid_json = Path::new("test.json");
    assert!(collect_file_diagnostics(valid_json, r#"{"valid": true}"#).is_empty());
    let invalid_json = collect_file_diagnostics(valid_json, r#"{"invalid": }"#);
    assert!(!invalid_json.is_empty());
    assert_eq!(invalid_json[0]["severity"], "error");

    let valid_rs = Path::new("main.rs");
    assert!(collect_file_diagnostics(valid_rs, "fn main() { println!(\"hello\"); }").is_empty());

    let invalid_rs = collect_file_diagnostics(valid_rs, "fn main() { let x = (1 + 2; }");
    assert!(!invalid_rs.is_empty());
    assert_eq!(invalid_rs[0]["severity"], "error");

    let unclosed_rs =
        collect_file_diagnostics(valid_rs, "fn main() { if true { println!(\"yes\"); }");
    assert!(!unclosed_rs.is_empty());
    assert_eq!(unclosed_rs[0]["severity"], "error");
}

#[test]
fn test_extract_symbol_outline_java() {
    let java_code = r#"
package com.deepcloud.engine.api;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/api/v1/jobs")
public class SpiderJobController extends BaseController {

    @PostMapping
    @RequirePermission("engine:job:create")
    public Result<Long> createJob(@Valid @RequestBody CreateJobCmd cmd) {
        // Implementation with 50 lines
        return Result.ok(1L);
    }

    @GetMapping("/{id}")
    public Result<SpiderJobVO> getJob(@PathVariable("id") Long id) {
        // Implementation
        return Result.ok(null);
    }
}
"#;
    let outline = extract_symbol_outline(java_code, "java");
    let joined = outline.join("\n");
    assert!(joined.contains("package com.deepcloud.engine.api;"));
    assert!(joined.contains("@RestController"));
    assert!(joined.contains("@RequestMapping(\"/api/v1/jobs\")"));
    assert!(joined.contains("public class SpiderJobController extends BaseController"));
    assert!(joined.contains("@PostMapping"));
    assert!(joined.contains("public Result<Long> createJob(@Valid @RequestBody CreateJobCmd cmd)"));
    assert!(joined.contains("@GetMapping(\"/{id}\")"));
    assert!(!joined.contains("Implementation with 50 lines"));
}

#[test]
fn test_extract_symbol_outline_rust() {
    let rs_code = r#"
pub mod engine;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SpiderJob {
    pub id: String,
}

impl SpiderJob {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    fn internal_helper(&self) {
        // secret logic
    }
}
"#;
    let outline = extract_symbol_outline(rs_code, "rs");
    let joined = outline.join("\n");
    assert!(joined.contains("pub mod engine;"));
    assert!(joined.contains("#[derive(Debug, Clone)]"));
    assert!(joined.contains("pub struct SpiderJob"));
    assert!(joined.contains("impl SpiderJob"));
    assert!(joined.contains("pub fn new(id: String) -> Self"));
    assert!(joined.contains("fn internal_helper(&self)"));
    assert!(!joined.contains("secret logic"));
}
