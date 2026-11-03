use super::*;
use crate::services::brain::tools::{fs::{ReadFileTool,WriteFileTool},skill_tool::SkillTool};
use std::{fs,path::{Path,PathBuf}};

fn fixture() -> (tempfile::TempDir,PathBuf,PathBuf,PathBuf,ToolExecutionGateway) {
    let dir=tempfile::tempdir().unwrap();let work=dir.path().join("workspace");fs::create_dir_all(&work).unwrap();
    let catalog=dir.path().join("中文技能库");
    let selected=catalog.join(".agents/skills/routing-fixture");let shadowed=catalog.join("skills/routing-fixture");
    for root in [&selected,&shadowed] {
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(root.join("SKILL.md"),"---\nname: read-routing-fixture\ndescription: routing regression fixture\n---\nInstructions").unwrap();
        fs::write(root.join("scripts/probe.cjs"),"line-one\nline-two\nline-three\n").unwrap();
    }
    let registry:ToolRegistry=Arc::new(Mutex::new(HashMap::new()));
    for tool in [Arc::new(SkillTool::new(catalog)) as Arc<dyn ToolExecutor>,Arc::new(ReadFileTool::new(work.clone())),Arc::new(WriteFileTool::new(work.clone()))] {
        registry.lock().unwrap().insert(tool.name().into(),tool);
    }
    (dir,work,selected,shadowed,ToolExecutionGateway::with_policy(registry,ToolPolicy::new(ToolPolicyMode::Audit)))
}
fn call(gateway:&ToolExecutionGateway,name:&str,args:Value)->ToolExecutionReceipt {
    gateway.execute(ToolExecutionRequest{call_id:format!("route-{name}"),name:name.into(),arguments:args})
}

#[test]
fn selected_skill_read_alias_uses_real_reader_and_preserves_line_range() {
    let (_dir,_work,selected,_shadowed,gateway)=fixture();
    let receipt=call(&gateway,"read_file",json!({"path":selected.join("scripts/probe.cjs"),"StartLine":2,"EndLine":2}));
    assert_eq!(receipt.status,ToolExecutionStatus::Succeeded);assert_eq!(receipt.name,"skill_tool");
    assert_eq!(receipt.result["content"],"line-two\n");assert_eq!(receipt.result["skill_name"],"read-routing-fixture");
    assert_eq!(receipt.result["resource_read_route"]["requested_tool"],"read_file");assert!(!receipt.verification);
    assert!(!selected.join("executed.marker").exists(),"reading a script is not executing it");
    let main=call(&gateway,"view_file",json!({"AbsolutePath":selected.join("SKILL.md")}));
    assert_eq!(main.name,"skill_tool");assert!(main.result["content"].as_str().unwrap().contains("Instructions"));
}

#[test]
fn ordinary_external_files_and_writes_keep_workspace_boundary() {
    let (dir,work,selected,_shadowed,gateway)=fixture();let outside=dir.path().join("outside.txt");fs::write(&outside,"private").unwrap();
    let rejected=call(&gateway,"view_file",json!({"AbsolutePath":outside}));
    assert_eq!(rejected.status,ToolExecutionStatus::Failed);assert_eq!(rejected.name,"view_file");assert!(rejected.result["error"].as_str().unwrap().contains("outside"));
    let script=selected.join("scripts/probe.cjs");let before=fs::read(&script).unwrap();
    let write=call(&gateway,"write_file",json!({"path":script,"content":"changed"}));
    assert_eq!(write.status,ToolExecutionStatus::Failed);assert_eq!(write.name,"write_to_file");assert_eq!(fs::read(&script).unwrap(),before);
    fs::write(work.join("local.txt"),"local").unwrap();let local=call(&gateway,"view_file",json!({"AbsolutePath":work.join("local.txt")}));
    assert_eq!(local.status,ToolExecutionStatus::Succeeded);assert_eq!(local.name,"view_file");
}

#[test]
fn shadowed_skill_and_invalid_ranges_do_not_gain_access() {
    let (_dir,_work,selected,shadowed,gateway)=fixture();
    let shadow=call(&gateway,"view_file",json!({"AbsolutePath":shadowed.join("scripts/probe.cjs")}));
    assert_eq!(shadow.status,ToolExecutionStatus::Failed);assert_eq!(shadow.name,"view_file");
    for (start,end) in [(0,1),(3,2)] {
        let invalid=call(&gateway,"view_file",json!({"AbsolutePath":selected.join("scripts/probe.cjs"),"start_line":start,"end_line":end}));
        assert_eq!(invalid.status,ToolExecutionStatus::Failed);assert_eq!(invalid.name,"skill_tool");
    }
}

#[test]
fn escaped_resource_link_never_routes_to_skill_reader() {
    let (dir,_work,selected,_shadowed,gateway)=fixture();let outside=dir.path().join("outside-dir");fs::create_dir_all(&outside).unwrap();fs::write(outside.join("private.txt"),"secret").unwrap();
    let link=selected.join("scripts/escape");create_link(&outside,&link);
    let rejected=call(&gateway,"view_file",json!({"AbsolutePath":link.join("private.txt")}));
    assert_eq!(rejected.status,ToolExecutionStatus::Failed);assert_eq!(rejected.name,"view_file");
}
fn create_link(target:&Path,link:&Path) {
    #[cfg(unix)] std::os::unix::fs::symlink(target,link).unwrap();
    #[cfg(windows)] {
        let output=std::process::Command::new("powershell.exe")
            .args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:HUB_TEST_LINK -Target $env:HUB_TEST_TARGET | Out-Null"])
            .env("HUB_TEST_LINK",link).env("HUB_TEST_TARGET",target).output().unwrap();
        assert!(output.status.success(),"Could not create junction fixture: {}",String::from_utf8_lossy(&output.stderr));
    }
}
