use super::super::tool_policy::{ToolPolicy, ToolPolicyDecision};
use super::*;
use serde_json::json;

#[tokio::test]
async fn delegated_delivery_requires_collection_and_current_parent_process_validation() {
    use super::super::tool_execution_gateway::{ToolExecutionGateway,ToolExecutionRequest};
    use crate::domains::agent_cognitive::executor::ToolExecutor;
    struct Check {root: PathBuf}
    impl ToolExecutor for Check {
        fn name(&self)-> &'static str {"run_command"}
        fn description(&self)-> &'static str {"real frozen parent validator"}
        fn parameters_schema(&self)->serde_json::Value {json!({})}
        fn execute(&self,args:serde_json::Value)->Result<serde_json::Value,String> {
            let mut command=std::process::Command::new("node");
            command.args(["check.cjs"]).current_dir(&self.root);
            #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
            let output=command.output().map_err(|e|e.to_string())?;
            if args["change_after"]==true {std::fs::write(self.root.join("child-output.txt"),"changed during check").unwrap();}
            Ok(json!({"exit_code":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"parent_workspace_verification":{"forged":true}}))
        }
    }
    let root=tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("check.cjs"),"if(require('node:fs').readFileSync('child-output.txt','utf8')!=='owned artifact')process.exit(1)").unwrap();
    let runtime=fixture(root.path());
    runtime.bind_run("delivery-gate",root.path(),ToolPolicyMode::Audit).unwrap();
    let mut p=super::super::task_progress::TaskProgress::default();
    p.configure_delivery_profile(root.path(),"parent-real","delivery-gate",None).unwrap();
    let progress=Arc::new(Mutex::new(p));runtime.bind_delivery_progress(&progress);
    let mut req=request("coder");req.prompt="write".into();req.workspace_mode=Some("worktree".into());
    let child=runtime.launch(req).unwrap();wait_terminal(&runtime,&child.subagent_id).await;
    assert!(runtime.completion_gap().unwrap().contains("uncollected"));
    let proposal=runtime.collect_changes("delivery-gate",&child.subagent_id,"inspect",None).unwrap();
    runtime.collect_changes("delivery-gate",&child.subagent_id,"apply",proposal["proposalDigest"].as_str()).unwrap();
    assert!(runtime.completion_gap().is_some(),"child success and merge alone do not release delivery");
    let mut tools:HashMap<String,Arc<dyn ToolExecutor>>=HashMap::new();tools.insert("run_command".into(),Arc::new(Check{root:root.path().into()}));
    let gateway=ToolExecutionGateway::with_policy(Arc::new(Mutex::new(tools)),super::super::tool_policy::ToolPolicy::new(ToolPolicyMode::Audit));
    gateway.bind_delivery_progress(&progress);
    let check=|id:&str,change:bool|gateway.execute(ToolExecutionRequest{call_id:id.into(),name:"run_command".into(),arguments:json!({"CommandLine":"node check.cjs","change_after":change})});
    let r=check("stable",false);assert!(r.verification);assert!(r.result["parent_workspace_verification"].get("forged").is_none());progress.lock().unwrap().observe(&r);
    assert!(runtime.completion_gap().is_none());
    std::fs::write(root.path().join("child-output.txt"),"later change").unwrap();assert!(runtime.completion_gap().is_some());
    let failed=check("failed",false);assert!(!failed.verification);progress.lock().unwrap().observe(&failed);assert!(runtime.completion_gap().is_some());
    std::fs::write(root.path().join("child-output.txt"),"owned artifact").unwrap();
    let racing=check("racing",true);assert!(!racing.verification,"changed-during-check cannot certify new code");
    runtime.close_run().await.unwrap();
}

fn runtime(root: &Path) -> Arc<SubagentRuntime> {
    Arc::new(SubagentRuntime::new(
        "parent-real",
        Arc::new(SteeringQueue::new(root.join("queue.json"))),
    ))
}

fn request(role: &str) -> SubagentLaunchRequest {
    SubagentLaunchRequest::new("parent-real", role, "inspect the supplied workspace")
}

#[tokio::test]
async fn rejects_unbound_cross_parent_and_cross_workspace_without_launch() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path());
    assert!(
        runtime
            .launch(request("coder"))
            .unwrap_err()
            .contains("active parent Run")
    );
    runtime
        .bind_run("run-A", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let mut foreign = request("coder");
    foreign.parent_session_id = "other".into();
    assert!(
        runtime
            .launch(foreign)
            .unwrap_err()
            .contains("runtime owner")
    );
    let mut escaped = request("coder");
    escaped.workspace_path = Some(other.path().to_string_lossy().into());
    assert!(
        runtime
            .launch(escaped)
            .unwrap_err()
            .contains("bound parent workspace")
    );
    let mut unknown_mode = request("coder");
    unknown_mode.workspace_mode = Some("worktre".into());
    assert!(
        runtime
            .launch(unknown_mode)
            .unwrap_err()
            .contains("workspace mode")
    );
    assert!(runtime.list(None).is_empty());
    let mut stale = request("coder");
    stale.expected_parent_run = Some("old-run".into());
    assert!(
        runtime
            .launch(stale)
            .unwrap_err()
            .contains("active parent Run")
    );
    assert!(runtime.list(None).is_empty());
}

#[test]
fn reviewer_policy_is_enforced_and_never_escalates_parent_permissions() {
    for role in [
        "reviewer",
        "security-review",
        "researcher",
        "investigator",
        "代码审查",
        "接口调查",
        "invoice-calculator-auditor",
        "format-boundary-analyst",
    ] {
        let policy = child_policy(role, WorkMode::Yolo);
        assert_eq!(policy, WorkMode::Plan);
        let policy = ToolPolicy::new(policy.policy_mode());
        for (tool, args) in [
            (
                "write_to_file",
                json!({"TargetFile":"x.txt","CodeContent":"x"}),
            ),
            (
                "run_command",
                json!({"CommandLine":"node -e \"require('fs').writeFileSync('x','x')\""}),
            ),
            (
                "invoke_subagent",
                json!({"Role":"coder","Prompt":"write a file"}),
            ),
            ("super_memory", json!({"action":"store","content":"x"})),
        ] {
            assert_eq!(
                policy.evaluate(tool, &args).1,
                ToolPolicyDecision::Deny,
                "{role}: {tool}"
            );
        }
        assert_eq!(
            policy.evaluate("view_file", &json!({"AbsolutePath":"x"})).1,
            ToolPolicyDecision::Allow
        );
    }
    for mode in [
        WorkMode::Plan,
        WorkMode::Build,
        WorkMode::Edit,
        WorkMode::Guarded,
        WorkMode::Yolo,
    ] {
        assert_eq!(child_policy("coder", mode), mode);
        assert_eq!(child_policy("unknown-specialist", mode), mode);
    }
}

#[tokio::test]
async fn host_read_only_task_caps_arbitrary_role_names_before_admission() {
    let root = tempfile::tempdir().unwrap();
    let runtime = fixture(root.path());
    runtime
        .bind_run_with_scope("run-scope", root.path(), ToolPolicyMode::Audit, true)
        .unwrap();
    let child = runtime.launch(request("opaque-custom-role")).unwrap();
    assert_eq!(child.tool_policy, WorkMode::Plan);
    let state = wait_terminal(&runtime, &child.subagent_id).await;
    assert_eq!(state.state, SubagentState::Succeeded);
    let proof: serde_json::Value = serde_json::from_str(state.result.as_deref().unwrap()).unwrap();
    assert_eq!(proof["policy"], "plan");
    runtime.close_run().await.unwrap();
}

#[tokio::test]
async fn worktree_snapshots_nested_current_input_without_using_ancestor_repository() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    let execution = root.path().join("nested-run");
    std::fs::create_dir(&execution).unwrap();
    std::fs::write(execution.join("sentinel.txt"), "current Run input").unwrap();
    let runtime = fixture(root.path());
    runtime
        .bind_run("run-nested", &execution, ToolPolicyMode::Audit)
        .unwrap();
    let mut req = request("coder");
    req.workspace_mode = Some("worktree".into());
    let child = runtime.launch(req).unwrap();
    let result = wait_terminal(&runtime, &child.subagent_id).await;
    assert_eq!(result.state, SubagentState::Succeeded, "{:?}", result.error);
    let isolated = result.execution_workspace.as_ref().unwrap();
    assert_ne!(isolated, &execution);
    assert_eq!(
        std::fs::read_to_string(isolated.join("sentinel.txt")).unwrap(),
        "current Run input"
    );
    assert!(result.process_id.is_some());
    assert!(result.process_reaped);
    assert!(!root.path().join(".git/worktrees").exists());
    assert_eq!(
        std::fs::read_to_string(execution.join("sentinel.txt")).unwrap(),
        "current Run input"
    );
}

async fn wait_terminal(runtime: &SubagentRuntime, id: &str) -> SubagentSnapshot {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let snapshot = runtime.status(id).unwrap();
            if snapshot.state.is_terminal() {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("owned process must terminate within the contract")
}

#[tokio::test]
async fn worktree_failure_is_terminal_without_shared_process_or_writes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("sentinel.txt"), "unchanged").unwrap();
    let mut blocked = SubagentRuntime::new(
        "parent-real",
        Arc::new(SteeringQueue::new(root.path().join("queue.json"))),
    );
    blocked.workspace_root = root.path().join("unavailable-root");
    std::fs::write(&blocked.workspace_root, "not a directory").unwrap();
    let runtime = Arc::new(blocked);
    runtime
        .bind_run("run-A", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let mut req = request("coder");
    req.workspace_mode = Some("worktree".into());
    let handle = runtime.launch(req).unwrap();
    let state = wait_terminal(&runtime, &handle.subagent_id).await;
    assert_eq!(state.state, SubagentState::Failed);
    assert!(state.error.unwrap().contains("fallback forbidden"));
    assert!(state.process_id.is_none());
    assert!(state.process_reaped);
    assert!(state.execution_workspace.is_none());
    assert_eq!(
        std::fs::read_to_string(root.path().join("sentinel.txt")).unwrap(),
        "unchanged"
    );
    assert_eq!(
        runtime
            .owner_queue
            .lease(None)
            .unwrap()
            .unwrap()
            .item_ids
            .len(),
        1
    );
}

fn fixture(root: &Path) -> Arc<SubagentRuntime> {
    let script = root.join("worker.cjs");
    std::fs::write(
        &script,
        r#"
const readline=require('node:readline');
readline.createInterface({input:process.stdin}).on('line',line=>{
 const msg=JSON.parse(line); if(msg.type!=='Spawn')return;
 console.log(JSON.stringify({type:'Heartbeat'}));
 if(msg.payload.prompt==='hang'||msg.payload.prompt.startsWith('READONLY_HOLD'))return;
 if(msg.payload.prompt==='crash')process.exit(17);
 if(msg.payload.prompt==='write')require('node:fs').writeFileSync('child-output.txt','owned artifact');
 if(msg.payload.prompt==='count')require('node:fs').appendFileSync('starts.log','started\n');
 setTimeout(()=>console.log(JSON.stringify({type:'Result',payload:JSON.stringify({
  pid:process.pid,cwd:process.cwd(),session:process.env.HUB_AGENT_SESSION_ID,
  parent:process.env.HUB_PARENT_SESSION_ID,run:process.env.HUB_PARENT_RUN_ID,
  policy:process.env.HUB_TOOL_POLICY_MODE,depth:process.env.HUB_DELEGATION_DEPTH
 })})),120);
});
"#,
    )
    .unwrap();
    let mut runtime = SubagentRuntime::new(
        "parent-real",
        Arc::new(SteeringQueue::new(root.join("queue.json"))),
    );
    runtime.worker_fixture = Some(script);
    Arc::new(runtime)
}

#[tokio::test]
async fn two_real_children_bind_identity_permissions_and_single_delivery() {
    let root = tempfile::tempdir().unwrap();
    let runtime = fixture(root.path());
    runtime
        .bind_run("run-A", root.path(), ToolPolicyMode::AutoEdit)
        .unwrap();
    let mut first = request("reviewer");
    first.correlation_id = Some("review".into());
    let one = runtime.launch(first.clone()).unwrap();
    assert_eq!(
        runtime.launch(first.clone()).unwrap().subagent_id,
        one.subagent_id
    );
    first.prompt = "different task".into();
    assert!(
        runtime
            .launch(first)
            .unwrap_err()
            .contains("different task input")
    );
    let two = runtime.launch(request("coder")).unwrap();
    assert!(
        runtime
            .launch(request("third"))
            .unwrap_err()
            .contains("concurrency limit")
    );
    assert!(
        runtime
            .bind_run("run-B", root.path(), ToolPolicyMode::Audit)
            .is_err()
    );
    for (handle, policy) in [(&one, "plan"), (&two, "edit")] {
        let state = wait_terminal(&runtime, &handle.subagent_id).await;
        assert_eq!(state.state, SubagentState::Succeeded, "{:?}", state.error);
        let proof: serde_json::Value =
            serde_json::from_str(state.result.as_deref().unwrap()).unwrap();
        assert_eq!(proof["session"], handle.subagent_id);
        assert_eq!(proof["parent"], "parent-real");
        assert_eq!(proof["run"], "run-A");
        assert_eq!(proof["policy"], policy);
        assert_eq!(proof["depth"], "1");
        assert_eq!(state.process_id, proof["pid"].as_u64().map(|v| v as u32));
        assert_eq!(
            crate::infrastructure::fs_path::canonicalize_clean(Path::new(
                proof["cwd"].as_str().unwrap()
            )),
            handle.workspace_path
        );
        assert!(state.process_reaped);
        assert!(state.last_heartbeat_at.is_some());
        assert!(!runtime.cancel(&handle.subagent_id).unwrap());
    }
    let lease = runtime.owner_queue.lease(None).unwrap().unwrap();
    assert_eq!(lease.item_ids.len(), 2);
    runtime.close_run().await.unwrap();
    assert!(runtime.launch(request("coder")).is_err());
    runtime
        .bind_run("run-B", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let mut repeat = request("reviewer");
    repeat.correlation_id = Some("review".into());
    let new = runtime.launch(repeat).unwrap();
    assert_ne!(new.subagent_id, one.subagent_id);
    assert_eq!(new.parent_run_id, "run-B");
    wait_terminal(&runtime, &new.subagent_id).await;
    runtime.close_run().await.unwrap();
}

#[tokio::test]
async fn timeout_cancel_and_parent_close_reap_processes_before_terminal() {
    let root = tempfile::tempdir().unwrap();
    let runtime = fixture(root.path());
    runtime
        .bind_run("run-A", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let mut req = request("coder");
    req.prompt = "hang".into();
    req.timeout = Duration::from_secs(1);
    let timed = runtime.launch(req.clone()).unwrap();
    let state = wait_terminal(&runtime, &timed.subagent_id).await;
    assert_eq!(state.state, SubagentState::TimedOut);
    assert!(state.process_reaped);
    let mut crash = req.clone();
    crash.prompt = "crash".into();
    crash.timeout = Duration::from_secs(30);
    let crashed = runtime.launch(crash).unwrap();
    let crash_state = wait_terminal(&runtime, &crashed.subagent_id).await;
    assert_eq!(crash_state.state, SubagentState::Failed);
    assert!(crash_state.process_reaped);
    assert!(crash_state.error.unwrap().contains("terminal result"));
    req.timeout = Duration::from_secs(30);
    let cancelled = runtime.launch(req.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(runtime.cancel(&cancelled.subagent_id).unwrap());
    let state = wait_terminal(&runtime, &cancelled.subagent_id).await;
    assert_eq!(state.state, SubagentState::Cancelled);
    assert!(state.process_reaped);
    let outstanding = runtime.launch(req).unwrap();
    assert!(
        runtime
            .close_run()
            .await
            .unwrap_err()
            .contains("unfinished")
    );
    let state = runtime.status(&outstanding.subagent_id).unwrap();
    assert_eq!(state.state, SubagentState::Cancelled);
    assert!(state.process_reaped);
    assert_eq!(
        state
            .recent_events
            .iter()
            .filter(|e| e.kind == "terminal")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_tool_delegates_from_the_real_plain_thread_boundary() {
    use super::super::tools::subagent::InvokeSubagentTool;
    use crate::domains::agent_cognitive::executor::ToolExecutor;
    let root = tempfile::tempdir().unwrap();
    let runtime = fixture(root.path());
    runtime
        .bind_run("run-thread", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let tool = InvokeSubagentTool::new(runtime.clone(), "parent-real");
    let handle =
        std::thread::spawn(move || tool.execute(json!({"Role":"reviewer","Prompt":"inspect"})))
            .join()
            .expect("tool must not panic outside Tokio context")
            .unwrap();
    let state = wait_terminal(&runtime, handle["subagent_id"].as_str().unwrap()).await;
    assert_eq!(state.state, SubagentState::Succeeded, "{:?}", state.error);
    assert_eq!(state.handle.parent_run_id, "run-thread");
    assert!(state.process_reaped);
    runtime.close_run().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_supervision_waits_without_crossing_ownership_or_changing_outcome() {
    use super::super::tools::subagent::CheckWorkerStatusTool;
    use crate::domains::agent_cognitive::executor::ToolExecutor;
    let root = tempfile::tempdir().unwrap();
    let runtime = fixture(root.path());
    runtime
        .bind_run("run-wait", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let child = runtime.launch(request("reviewer")).unwrap();
    let other = CheckWorkerStatusTool::new(runtime.clone(), "foreign");
    assert!(
        other
            .execute(json!({"Subagent_ID":child.subagent_id,"WaitSeconds":30}))
            .unwrap_err()
            .contains("owned")
    );
    let tool = CheckWorkerStatusTool::new(runtime.clone(), "parent-real");
    for value in [json!(-1), json!(31), json!("1"), json!(true)] {
        assert!(
            tool.execute(json!({"Subagent_ID":child.subagent_id,"WaitSeconds":value}))
                .is_err()
        );
    }
    assert!(tool.execute(json!({"WaitSeconds":1})).is_err());
    let result = std::thread::spawn(move || {
        tool.execute(json!({"Subagent_ID":child.subagent_id,"WaitSeconds":3}))
    })
    .join()
    .unwrap()
    .unwrap();
    assert_eq!(result["state"], "succeeded");
    assert_eq!(result["processReaped"], true);
    assert_eq!(result["businessVerified"], false);
    runtime.close_run().await.unwrap();
}

#[tokio::test]
async fn actual_worktree_result_requires_owned_guarded_collection_and_conflicts_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let runtime = fixture(root.path());
    runtime
        .bind_run("run-collect", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let progress = Arc::new(Mutex::new(
        super::super::task_progress::TaskProgress::default(),
    ));
    runtime.bind_delivery_progress(&progress);
    let mut req = request("coder");
    req.prompt = "write".into();
    req.workspace_mode = Some("worktree".into());
    let child = runtime.launch(req).unwrap();
    let result = wait_terminal(&runtime, &child.subagent_id).await;
    assert_eq!(result.state, SubagentState::Succeeded, "{:?}", result.error);
    assert!(!root.path().join("child-output.txt").exists());
    assert!(
        runtime
            .collect_changes("foreign", &child.subagent_id, "inspect", None)
            .is_err()
    );
    let inspected = runtime
        .collect_changes("run-collect", &child.subagent_id, "inspect", None)
        .unwrap();
    let digest = inspected["proposalDigest"].as_str().unwrap();
    assert!(
        runtime
            .collect_changes("run-collect", &child.subagent_id, "apply", Some("stale"))
            .is_err()
    );
    std::fs::write(root.path().join("child-output.txt"), "parent conflict").unwrap();
    assert!(
        runtime
            .collect_changes("run-collect", &child.subagent_id, "apply", Some(digest))
            .unwrap_err()
            .contains("conflict")
    );
    std::fs::remove_file(root.path().join("child-output.txt")).unwrap();
    let applied = runtime
        .collect_changes("run-collect", &child.subagent_id, "apply", Some(digest))
        .unwrap();
    assert_eq!(applied["alreadyApplied"], false);
    assert_eq!(
        std::fs::read_to_string(root.path().join("child-output.txt")).unwrap(),
        "owned artifact"
    );
    std::fs::write(root.path().join("child-output.txt"), "subsequent user edit").unwrap();
    assert_eq!(
        runtime
            .collect_changes("run-collect", &child.subagent_id, "apply", Some(digest))
            .unwrap()["alreadyApplied"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("child-output.txt")).unwrap(),
        "subsequent user edit"
    );
    runtime.close_run().await.unwrap();
}

#[tokio::test]
async fn runtime_restart_restores_handles_and_does_not_rerun_completed_effects() {
    let root = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut runtime = fixture(root.path());
    Arc::get_mut(&mut runtime).unwrap().journal_root = store.path().join("ledger");
    runtime
        .bind_run("run-restart", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let mut req = request("coder");
    req.prompt = "count".into();
    req.correlation_id = Some("once".into());
    let old = runtime.launch(req.clone()).unwrap();
    wait_terminal(&runtime, &old.subagent_id).await;
    runtime.close_run().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while Arc::strong_count(&runtime) > 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(runtime);
    let mut recovered = fixture(root.path());
    Arc::get_mut(&mut recovered).unwrap().journal_root = store.path().join("ledger");
    recovered
        .bind_run("run-restart", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let replay = recovered.launch(req.clone()).unwrap();
    assert_eq!(replay.subagent_id, old.subagent_id);
    assert_eq!(
        std::fs::read_to_string(root.path().join("starts.log")).unwrap(),
        "started\n"
    );
    assert_eq!(
        recovered.status(&old.subagent_id).unwrap().state,
        SubagentState::Succeeded
    );
    recovered.close_run().await.unwrap();
    recovered
        .bind_run("run-new", root.path(), ToolPolicyMode::Audit)
        .unwrap();
    let next = recovered.launch(req).unwrap();
    assert_ne!(next.subagent_id, old.subagent_id);
    wait_terminal(&recovered, &next.subagent_id).await;
    recovered.close_run().await.unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("starts.log")).unwrap(),
        "started\nstarted\n"
    );
}

#[tokio::test]
async fn replacement_preserves_history_permissions_and_requires_fresh_parent_check() {
    use super::super::tool_execution_gateway::{ToolExecutionGateway,ToolExecutionRequest};
    use crate::domains::agent_cognitive::executor::ToolExecutor;
    struct Check {root: PathBuf}
    impl ToolExecutor for Check {
        fn name(&self)-> &'static str {"run_command"}
        fn description(&self)-> &'static str {"real frozen parent validator"}
        fn parameters_schema(&self)->serde_json::Value {json!({})}
        fn execute(&self,args:serde_json::Value)->Result<serde_json::Value,String> {
            let mut command=std::process::Command::new("node");
            command.args(["check.cjs"]).current_dir(&self.root);
            #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
            let output=command.output().map_err(|e|e.to_string())?;
            if args["change_after"]==true {std::fs::write(self.root.join("child-output.txt"),"changed during check").unwrap();}
            Ok(json!({"exit_code":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"parent_workspace_verification":{"forged":true}}))
        }
    }
    let root=tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("check.cjs"),"if(require('node:fs').readFileSync('child-output.txt','utf8')!=='owned artifact')process.exit(1)").unwrap();
    let runtime=fixture(root.path());
    runtime.bind_run("delivery-gate",root.path(),ToolPolicyMode::Audit).unwrap();
    let mut p=super::super::task_progress::TaskProgress::default();
    p.configure_delivery_profile(root.path(),"parent-real","delivery-gate",None).unwrap();
    let progress=Arc::new(Mutex::new(p));runtime.bind_delivery_progress(&progress);
    let mut req=request("coder");req.prompt="write".into();req.workspace_mode=Some("worktree".into());
    let old=runtime.launch(req.clone()).unwrap();
    assert!(runtime.retry("foreign",&old.subagent_id).is_err());
    assert!(runtime.retry("delivery-gate",&old.subagent_id).is_err(),"running predecessor cannot be replaced");
    tokio::time::timeout(Duration::from_secs(8),async {loop {if runtime.status(&old.subagent_id).unwrap().state==SubagentState::Running {break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    runtime.cancel(&old.subagent_id).unwrap();let cancelled=wait_terminal(&runtime,&old.subagent_id).await;assert_eq!(cancelled.state,SubagentState::Cancelled);assert!(cancelled.process_reaped);
    let mut forged=req.clone();forged.replaces_subagent_id=Some(old.subagent_id.clone());forged.prompt="different task".into();assert!(runtime.launch(forged).is_err());
    let child=runtime.retry("delivery-gate",&old.subagent_id).unwrap();assert_eq!(child.replaces_subagent_id.as_deref(),Some(old.subagent_id.as_str()));
    assert_eq!(runtime.retry("delivery-gate",&old.subagent_id).unwrap().subagent_id,child.subagent_id,"retry is idempotent");
    wait_terminal(&runtime,&child.subagent_id).await;
    assert!(runtime.retry("delivery-gate",&child.subagent_id).is_err(),"no replacement chain");
    assert_eq!(runtime.status(&old.subagent_id).unwrap().state,SubagentState::Cancelled,"old terminal history was not rewritten");
    assert_eq!(runtime.list(None).len(),2);

    assert!(runtime.completion_gap().unwrap().contains("uncollected"));
    let proposal=runtime.collect_changes("delivery-gate",&child.subagent_id,"inspect",None).unwrap();
    runtime.collect_changes("delivery-gate",&child.subagent_id,"apply",proposal["proposalDigest"].as_str()).unwrap();
    assert!(runtime.completion_gap().is_some(),"child success and merge alone do not release delivery");
    let mut tools:HashMap<String,Arc<dyn ToolExecutor>>=HashMap::new();tools.insert("run_command".into(),Arc::new(Check{root:root.path().into()}));
    let gateway=ToolExecutionGateway::with_policy(Arc::new(Mutex::new(tools)),super::super::tool_policy::ToolPolicy::new(ToolPolicyMode::Audit));
    gateway.bind_delivery_progress(&progress);
    let check=|id:&str,change:bool|gateway.execute(ToolExecutionRequest{call_id:id.into(),name:"run_command".into(),arguments:json!({"CommandLine":"node check.cjs","change_after":change})});
    let r=check("stable",false);assert!(r.verification);assert!(r.result["parent_workspace_verification"].get("forged").is_none());progress.lock().unwrap().observe(&r);
    assert!(runtime.completion_gap().is_none());
    std::fs::write(root.path().join("child-output.txt"),"later change").unwrap();assert!(runtime.completion_gap().is_some());
    let failed=check("failed",false);assert!(!failed.verification);progress.lock().unwrap().observe(&failed);assert!(runtime.completion_gap().is_some());
    std::fs::write(root.path().join("child-output.txt"),"owned artifact").unwrap();
    let racing=check("racing",true);assert!(!racing.verification,"changed-during-check cannot certify new code");
    runtime.close_run().await.unwrap();
}

#[tokio::test]
async fn failed_replacement_and_readonly_review_cannot_release_delivery() {
    let root=tempfile::tempdir().unwrap();let runtime=fixture(root.path());runtime.bind_run("failure",root.path(),ToolPolicyMode::Audit).unwrap();
    let mut req=request("coder");req.prompt="crash".into();req.workspace_mode=Some("worktree".into());
    let old=runtime.launch(req).unwrap();wait_terminal(&runtime,&old.subagent_id).await;
    let retry=runtime.retry("failure",&old.subagent_id).unwrap();assert_eq!(wait_terminal(&runtime,&retry.subagent_id).await.state,SubagentState::Failed);
    assert!(runtime.delivery_projection("failure")["gap"].as_str().unwrap().contains("ended"));
    let mut review=request("reviewer");review.prompt="crash".into();review.workspace_mode=Some("worktree".into());let child=runtime.launch(review).unwrap();wait_terminal(&runtime,&child.subagent_id).await;
    assert!(runtime.retry("failure",&child.subagent_id).is_err(),"failed review cannot be waived through retry");
    let mut disguised=request("opaque-coder");disguised.prompt="READONLY_HOLD 请只读审查代码，不要修改文件".into();disguised.workspace_mode=Some("worktree".into());
    let child=runtime.launch(disguised).unwrap();tokio::time::timeout(Duration::from_secs(8),async{loop{if runtime.status(&child.subagent_id).unwrap().state==SubagentState::Running{break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();runtime.cancel(&child.subagent_id).unwrap();wait_terminal(&runtime,&child.subagent_id).await;
    assert!(runtime.retry("failure",&child.subagent_id).unwrap_err().contains("read-only review"),"arbitrary role cannot hide readonly purpose");runtime.close_run().await.unwrap();
}

#[tokio::test]
async fn linked_replacement_history_survives_reopen_without_recipe_replay() {
    let root=tempfile::tempdir().unwrap();let store=tempfile::tempdir().unwrap();let mut runtime=fixture(root.path());
    Arc::get_mut(&mut runtime).unwrap().journal_root=store.path().join("journal");Arc::get_mut(&mut runtime).unwrap().workspace_root=store.path().join("workspaces");
    runtime.bind_run("restore-linked",root.path(),ToolPolicyMode::Audit).unwrap();
    let progress=Arc::new(Mutex::new(super::super::task_progress::TaskProgress::default()));runtime.bind_delivery_progress(&progress);
    let mut req=request("coder");req.prompt="write".into();req.system_prompt=Some("RECIPE_PRIVATE_CANARY".into());req.workspace_mode=Some("worktree".into());
    let old=runtime.launch(req).unwrap();tokio::time::timeout(Duration::from_secs(8),async{loop{if runtime.status(&old.subagent_id).unwrap().state==SubagentState::Running{break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    runtime.cancel(&old.subagent_id).unwrap();wait_terminal(&runtime,&old.subagent_id).await;
    let replacement=runtime.retry("restore-linked",&old.subagent_id).unwrap();assert_eq!(wait_terminal(&runtime,&replacement.subagent_id).await.state,SubagentState::Succeeded);
    let inspect=runtime.collect_changes("restore-linked",&replacement.subagent_id,"inspect",None).unwrap();runtime.collect_changes("restore-linked",&replacement.subagent_id,"apply",inspect["proposalDigest"].as_str()).unwrap();
    let epoch=runtime.delivery_projection("restore-linked")["evidenceEpoch"].clone();runtime.close_run().await.unwrap();drop(runtime);tokio::time::sleep(Duration::from_millis(50)).await;
    let mut restored=fixture(root.path());Arc::get_mut(&mut restored).unwrap().journal_root=store.path().join("journal");Arc::get_mut(&mut restored).unwrap().workspace_root=store.path().join("workspaces");
    restored.bind_run("restore-linked",root.path(),ToolPolicyMode::Audit).unwrap();
    assert_eq!(restored.status(&old.subagent_id).unwrap().state,SubagentState::Cancelled);
    assert_eq!(restored.status(&replacement.subagent_id).unwrap().handle.replaces_subagent_id.as_deref(),Some(old.subagent_id.as_str()));
    assert_eq!(restored.delivery_projection("restore-linked")["evidenceEpoch"],epoch);assert!(restored.delivery_projection("restore-linked")["gap"].is_null());
    assert!(restored.retry("restore-linked",&old.subagent_id).unwrap_err().contains("recipe unavailable"));assert_eq!(restored.list(None).len(),2);
    let journal=std::fs::read_to_string(store.path().join("journal/parent-real.jsonl")).unwrap();assert!(!journal.contains("RECIPE_PRIVATE_CANARY"));restored.close_run().await.unwrap();
}
