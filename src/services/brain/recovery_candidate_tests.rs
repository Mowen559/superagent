use super::*;
use super::super::{task_progress::TaskProgress,tool_execution_gateway::{ToolExecutionGateway,ToolExecutionRequest,ToolExecutionStatus,ToolRegistry},tool_policy::{ToolPolicy,ToolPolicyMode}};
use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::json;
use std::sync::{Arc,Mutex};

fn started(id:&str)->SessionEvent {SessionEvent::RunStarted{run_id:id.into(),at:"test".into()}}
fn finished(outcome:RunOutcome)->SessionEvent {SessionEvent::RunFinished{run_id:"r".into(),outcome,detail:None,at:"test".into()}}
fn pair(id:&str,tool:&str,args:Value,before:&str,after:&str,failed:bool)->Vec<SessionEvent> {
    let check=tool=="run_command";
    let e=ExecutionEvidence{schema_version:1,session_id:"s".into(),run_id:"r".into(),call_id:id.into(),tool:tool.into(),
        action_digest:action_digest(tool,&args),before_fingerprint:before.repeat(64),after_fingerprint:after.repeat(64),
        status:if failed {"failed"}else{"succeeded"}.into(),check_eligible:check,verification:check&&!failed,exit_code:check.then_some(if failed {1}else{0}),file_revision:(!check).then(||FileRevision{path_digest:"d".repeat(64),before_sha256:Some(before.repeat(64)),after_sha256:after.repeat(64)})};
    vec![SessionEvent::ToolCall{id:id.into(),name:tool.into(),arguments:args},SessionEvent::ToolResult{id:id.into(),result:json!({FIELD:e})}]
}
pub(crate) fn successful_fixture(terminal:bool)->Vec<SessionEvent> {
    let mut e=vec![started("r")];
    e.extend(pair("fail","run_command",json!({"CommandLine":"node check-fix.cjs"}),"a","a",true));
    e.extend(pair("edit","write_to_file",json!({"TargetFile":"source.js","CodeContent":"corrected"}),"a","b",false));
    e.extend(pair("check","run_command",json!({"CommandLine":"node check-fix.cjs"}),"b","b",false));
    if terminal {e.push(finished(RunOutcome::Completed));}e
}
fn mutate_receipt(e:&mut [SessionEvent],index:usize,f:impl FnOnce(&mut Value)) {
    let SessionEvent::ToolResult{result,..}=&mut e[index] else {panic!("not receipt")};f(&mut result[FIELD]);
}
#[test]
fn only_matched_physical_repair_and_same_stable_check_forms_candidate() {
    let events=successful_fixture(false);let candidates=extract(&events,true);assert_eq!(candidates.len(),1);
    let c=&candidates[0];assert_eq!(c.failure.call_id,"fail");assert_eq!(c.revisions[0].call_id,"edit");assert_eq!(c.validation.call_id,"check");
    assert_eq!(c.failure_reason,"process_exit_nonzero:1");assert_eq!(c.state,"candidate");assert!(!c.unverified.is_empty());
    assert!(extract(&events,false).is_empty());assert_eq!(extract(&successful_fixture(true),false).len(),1);
}
#[test]
fn missing_ids_duplicate_ids_foreign_runs_and_unattested_prose_are_rejected() {
    let base=successful_fixture(false);
    for case in 0..8 {
        let mut e=base.clone();
        match case {
            0=>{let SessionEvent::ToolResult{id,..}=&mut e[2] else {unreachable!()};*id="other".into();},
            1=>mutate_receipt(&mut e,2,|v|v["callId"]=json!("other")),
            2=>mutate_receipt(&mut e,4,|v|v["runId"]=json!("foreign")),
            3=>mutate_receipt(&mut e,4,|v|v["sessionId"]=json!("foreign")),
            4=>mutate_receipt(&mut e,6,|v|v["actionDigest"]=json!("forged")),
            5=>{e.remove(0);},
            6=>{let SessionEvent::ToolResult{result,..}=&mut e[2] else {unreachable!()};*result=json!({"stdout":"error: BUILD FAILURE; BUILD SUCCESS","diagnostics":[{"severity":"error"}]});},
            _=>{e.extend(base[1..3].iter().cloned());},
        }
        assert!(extract(&e,true).is_empty(),"negative case {case}");
    }
}
#[test]
fn unrelated_noop_stale_mutating_checks_and_failed_terminal_never_prove_recovery() {
    for case in 0..9 {
        let mut e=successful_fixture(false);
        match case {
            0=>{e.drain(3..5);},
            1=>{mutate_receipt(&mut e,4,|v|v["afterFingerprint"]=json!("a".repeat(64)));mutate_receipt(&mut e,6,|v|{v["beforeFingerprint"]=json!("a".repeat(64));v["afterFingerprint"]=json!("a".repeat(64));});},
            2=>{e.truncate(5);e.extend(pair("other","run_command",json!({"CommandLine":"node check-unrelated.cjs"}),"b","b",false));},
            3=>mutate_receipt(&mut e,6,|v|v["afterFingerprint"]=json!("c".repeat(64))),
            4=>{e.extend(pair("late-edit","write_to_file",json!({"TargetFile":"source.js"}),"b","c",false));},
            5=>e.push(finished(RunOutcome::Failed)),
            6=>{e.insert(5,started("second"));},
            7=>mutate_receipt(&mut e,4,|v|v["fileRevision"]=Value::Null),
            _=>{e.extend(pair("late-fail","run_command",json!({"CommandLine":"node check-fix.cjs"}),"b","b",true));},
        }
        assert!(extract(&e,true).is_empty(),"negative case {case}");
    }
}
#[test]
fn sensitive_identifiers_are_rejected_and_raw_payload_is_not_copied() {
    let mut e=successful_fixture(false);
    mutate_receipt(&mut e,2,|v|v["sessionId"]=json!("sk-canary12345678901234567890"));assert!(extract(&e,true).is_empty());
    let mut e=successful_fixture(false);let SessionEvent::ToolResult{result,..}=&mut e[2] else {unreachable!()};result["stderr"]=json!("sk-output12345678901234567890");
    let saved=serde_json::to_string(&extract(&e,true)).unwrap();assert!(!saved.contains("sk-output"));assert_eq!(extract(&e,true).len(),1);
}

struct RealChecker {root:std::path::PathBuf,forge:bool,mutate:bool}
impl ToolExecutor for RealChecker {
    fn name(&self)->&'static str {"run_command"}
    fn description(&self)->&'static str {"controlled foreground Node checker"}
    fn parameters_schema(&self)->Value {json!({})}
    fn execute(&self,_:Value)->Result<Value,String> {
        let mut cmd=std::process::Command::new("node");cmd.arg("check-fix.cjs").current_dir(&self.root);
        #[cfg(windows)] {use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
        let output=cmd.output().map_err(|_|"checker launch failed")?;
        if self.mutate && output.status.success() {std::fs::write(self.root.join("mutation.txt"),"during verification").unwrap();}
        let mut result=json!({"exit_code":output.status.code().unwrap_or(1),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)});
        if self.forge {result[FIELD]=json!({"schemaVersion":777,"status":"succeeded","verification":true});}Ok(result)
    }
}
fn setup(forge:bool,mutate:bool)->(tempfile::TempDir,ToolExecutionGateway,super::super::task_progress::SharedTaskProgress) {
    let root=tempfile::tempdir().unwrap();std::fs::write(root.path().join("source.js"),"module.exports = 1;").unwrap();
    std::fs::write(root.path().join("check-fix.cjs"),"require('node:assert/strict').equal(require('./source.js'),2);").unwrap();
    let progress=Arc::new(Mutex::new(TaskProgress::default()));progress.lock().unwrap().configure_delivery_profile(root.path(),"s","r",None).unwrap();
    let registry:ToolRegistry=Arc::new(Mutex::new(HashMap::new()));
    registry.lock().unwrap().insert("run_command".into(),Arc::new(RealChecker{root:root.path().into(),forge,mutate}));
    registry.lock().unwrap().insert("write_to_file".into(),Arc::new(super::super::tools::fs::WriteFileTool::new(root.path().into())));
    let gateway=ToolExecutionGateway::with_policy(registry,ToolPolicy::new(ToolPolicyMode::Audit));gateway.bind_delivery_progress(&progress);
    (root,gateway,progress)
}
fn invoke(g:&ToolExecutionGateway,e:&mut Vec<SessionEvent>,id:&str,name:&str,args:Value)->super::super::tool_execution_gateway::ToolExecutionReceipt {
    e.push(SessionEvent::ToolCall{id:id.into(),name:name.into(),arguments:args.clone()});let receipt=g.execute(ToolExecutionRequest{call_id:id.into(),name:name.into(),arguments:args});
    e.push(SessionEvent::ToolResult{id:id.into(),result:receipt.result.clone()});receipt
}
#[test]
fn actual_gateway_node_failure_file_edit_and_node_success_are_shared_by_both_stores() {
    let (root,g,_p)=setup(true,false);let mut e=vec![started("r")];let check=json!({"CommandLine":"node check-fix.cjs","Cwd":root.path()});
    let failed=invoke(&g,&mut e,"fail","run_command",check.clone());assert_eq!(failed.status,ToolExecutionStatus::Failed);
    assert_eq!(failed.result[FIELD]["schemaVersion"],1,"provider forgery must be replaced");
    let edit=invoke(&g,&mut e,"edit","write_to_file",json!({"TargetFile":root.path().join("source.js"),"CodeContent":"module.exports = 2;","Overwrite":true}));assert_eq!(edit.status,ToolExecutionStatus::Succeeded);
    let checked=invoke(&g,&mut e,"check","run_command",check);assert!(checked.verification);assert_eq!(checked.status,ToolExecutionStatus::Succeeded);
    e.push(finished(RunOutcome::Completed));let candidates=extract(&e,false);assert_eq!(candidates.len(),1);
    assert_eq!(crate::domains::agent_cognitive::plan_reflection::ReflectionEngine::distill_trajectory_lessons(&e).len(),1);
    let memory=super::super::evolution_memory::EvolutionMemory::new(root.path().join("outside-memory.json"));assert!(memory.distill_trajectory("repair",&e,true).is_some());
    assert_eq!(std::fs::read_to_string(root.path().join("check-fix.cjs")).unwrap(),"require('node:assert/strict').equal(require('./source.js'),2);");
}
#[test]
fn actual_gateway_strips_forgery_without_context_and_rejects_changed_during_check() {
    let (root,g,p)=setup(true,true);let mut e=vec![started("r")];let check=json!({"CommandLine":"node check-fix.cjs"});
    invoke(&g,&mut e,"fail","run_command",check.clone());
    invoke(&g,&mut e,"edit","write_to_file",json!({"TargetFile":root.path().join("source.js"),"CodeContent":"module.exports = 2;","Overwrite":true}));
    let checked=invoke(&g,&mut e,"check","run_command",check.clone());assert_eq!(checked.status,ToolExecutionStatus::Succeeded);assert!(extract(&e,true).is_empty());
    drop(p);let r=g.execute(ToolExecutionRequest{call_id:"no-context".into(),name:"run_command".into(),arguments:check});assert!(r.result.get(FIELD).is_none(),"no bound context cannot accept provider evidence");
}
