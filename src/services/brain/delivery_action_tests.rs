#![cfg(windows)]
use super::*;

fn fixture() -> (tempfile::TempDir,PathBuf,PathBuf) {
    let dir=tempfile::tempdir().unwrap();let work=dir.path().join("工作区");fs::create_dir(&work).unwrap();
    let python=Command::new("python").args(["-X","utf8","-c","import sys; print(sys.executable)"]).output().unwrap();
    let program=PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
    let oracle=dir.path().join("oracle.py");
    fs::write(&oracle,"import json,sys\nok=json.load(open(sys.argv[1])).get('canary')=='expected'\nprint(json.dumps({'passed':ok,'target':'original-app','checks':['business-canary'],'error':'local canary mismatch'}))\nsys.exit(0 if ok else 2)\n").unwrap();
    let action=dir.path().join("action.py");
    fs::write(&action,"import hashlib,json,sys,pathlib\nassert hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest()==sys.argv[2]\npathlib.Path(sys.argv[3],'action.marker').write_text('executed')\nprint(json.dumps({'passed':True,'target':'original-app','checks':['action-canary'],'source_sha256':sys.argv[2]}))\n").unwrap();
    let verifier=|id:&str,script:&Path,arguments:Vec<&str>|json!({"id":id,"program":program,"program_sha256":file_sha256(&program).unwrap(),"script":script,"script_sha256":file_sha256(script).unwrap(),"args":arguments,"timeout_seconds":10});
    let profile=dir.path().join("host.json");
    fs::write(&profile,serde_json::to_vec(&json!({"delivery":{"version":1,"scope":"implementation","target":"original-app","artifacts":[
        {"id":"rule","path":"rule.json","required":true,"validator":"runtime"},{"id":"cloud","path":"publication.json","required":true}]},
        "validators":[verifier("runtime",&oracle,vec!["{artifact}"]),verifier("publisher",&action,vec!["{artifact}","{sha256}","{workspace}"])],
        "actions":[{"id":"publish","source":"rule","executor":"publisher"}]})).unwrap()).unwrap();
    (dir,work,profile)
}

#[test]
fn publication_requires_current_real_business_check_and_is_one_shot() {
    let (dir,work,profile)=fixture();let mut state=DeliveryEvidence::new(&work,"s","r").unwrap();state.load_profile(&profile).unwrap();
    assert!(state.perform_action("publish").is_err());assert!(!work.join("action.marker").exists());
    fs::write(work.join("rule.json"),r#"{"canary":"wrong"}"#).unwrap();
    assert!(state.verify(&["rule".into()]).is_err());assert!(state.action_guidance().contains("LOCAL BUSINESS FAILURE"));assert!(state.perform_action("publish").is_err());
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();
    let result=state.verify(&["rule".into()]).unwrap();assert_eq!(result["completion_ready"],false);assert!(state.completion_gap().is_some());
    fs::write(work.join("rule.json"),"changed").unwrap();assert!(state.perform_action("publish").unwrap_err().contains("stale"));assert!(!work.join("action.marker").exists());
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();state.verify(&["rule".into()]).unwrap();
    let oracle=dir.path().join("oracle.py");let before=fs::read(&oracle).unwrap();fs::write(&oracle,"changed verifier").unwrap();
    assert!(state.perform_action("publish").is_err());assert!(!work.join("action.marker").exists());fs::write(&oracle,before).unwrap();
    assert_eq!(state.perform_action("publish").unwrap()["status"],"success");assert!(work.join("action.marker").is_file());
    assert!(state.perform_action("publish").unwrap_err().contains("already attempted"));assert!(state.completion_gap().is_some(),"an action is not whole-delivery certification");
}

#[test]
fn action_executor_cannot_be_smuggled_into_verify_delivery() {
    let (_dir,work,profile)=fixture();let mut state=DeliveryEvidence::new(&work,"s","r").unwrap();state.load_profile(&profile).unwrap();
    let mut plan=state.contract.clone().unwrap();plan.artifacts.push(ArtifactRequirement{id:"bypass".into(),path:"rule.json".into(),required:true,validator:Some("publisher".into())});
    assert!(state.validate_declaration(Some(&plan)).unwrap_err().contains("cannot be used"));assert!(!work.join("action.marker").exists());
}

#[test]
fn attempted_action_guidance_prioritizes_missing_required_files_without_retry() {
    let (_dir,work,profile)=fixture();
    let mut config:Value=serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
    config["delivery"]["artifacts"].as_array_mut().unwrap().push(json!({"id":"readme","path":"README.md","required":true}));
    fs::write(&profile,serde_json::to_vec(&config).unwrap()).unwrap();
    let mut state=DeliveryEvidence::new(&work,"s","r").unwrap();state.load_profile(&profile).unwrap();
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();state.verify(&["rule".into()]).unwrap();state.perform_action("publish").unwrap();
    let guidance=state.action_guidance();assert!(guidance.contains("ACTION ALREADY ATTEMPTED"));assert!(guidance.contains("readme (README.md)"));
    assert!(guidance.contains("publication.json"));assert!(guidance.contains("ALL required artifacts"));assert!(!guidance.contains("use the managed action once"));
    fs::write(work.join("README.md"),"actual documentation").unwrap();fs::write(work.join("publication.json"),"{}").unwrap();
    let ready=state.action_guidance();assert!(!ready.contains("FINISH MISSING"));assert!(ready.contains("verify the complete delivery"));assert!(state.completion_gap().is_some());
    assert!(state.perform_action("publish").unwrap_err().contains("already attempted"));
}

#[test]
fn changed_host_configuration_never_starts_action_or_retries() {
    let (_dir,work,profile)=fixture();let mut state=DeliveryEvidence::new(&work,"s","r").unwrap();state.load_profile(&profile).unwrap();
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();state.verify(&["rule".into()]).unwrap();
    let original=fs::read(&profile).unwrap();let mut changed=original.clone();changed.push(b' ');fs::write(&profile,changed).unwrap();
    assert!(state.perform_action("publish").unwrap_err().contains("profile changed"));assert!(!work.join("action.marker").exists());
    fs::write(&profile,original).unwrap();assert!(state.perform_action("publish").unwrap_err().contains("already attempted"));
}

#[test]
fn gateway_blocks_observed_cloud_probes_and_workspace_scripts_before_execution() {
    use crate::services::brain::tool_execution_gateway::{ToolExecutionGateway,ToolExecutionRequest,ToolExecutionStatus};
    use std::sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}};
    struct Counter(Arc<AtomicUsize>);
    impl crate::domains::agent_cognitive::executor::ToolExecutor for Counter {
        fn name(&self)->&'static str {"run_command"}fn description(&self)->&'static str {"test"}
        fn parameters_schema(&self)->Value {json!({})}
        fn execute(&self,_:Value)->Result<Value,String>{self.0.fetch_add(1,Ordering::SeqCst);Ok(json!({"status":"completed","exit_code":0}))}
    }
    let (_dir,work,profile)=fixture();let progress=Arc::new(Mutex::new(super::super::task_progress::TaskProgress::default()));
    progress.lock().unwrap().configure_delivery_profile(&work,"s","r",Some(&profile)).unwrap();
    let count=Arc::new(AtomicUsize::new(0));let registry=Arc::new(Mutex::new(std::collections::HashMap::new()));
    registry.lock().unwrap().insert("run_command".into(),Arc::new(Counter(count.clone())) as Arc<dyn crate::domains::agent_cognitive::executor::ToolExecutor>);
    let gateway=ToolExecutionGateway::new(registry);gateway.bind_delivery_progress(&progress);
    fs::write(work.join("cloud probe.cjs"),"fetch('https://pasteme.tyrantg.com/api/create',{method:'POST',body:'update'})").unwrap();
    for (i,(name,command)) in [("run_command","node -e \"fetch('https://pasteme.tyrantg.com/api/create',{method:'POST'})\""),("bash","curl -X PUT https://cmd.im/a"),("verify_task_tool","node cloud-probe.js https://pasteme.tyrantg.com/"),("run_command","node \"cloud probe.cjs\"")].iter().enumerate(){
        let receipt=gateway.execute(ToolExecutionRequest{call_id:format!("deny{i}"),name:(*name).into(),arguments:json!({"CommandLine":command,"command":command})});assert_eq!(receipt.status,ToolExecutionStatus::PolicyDenied);
    }
    assert_eq!(count.load(Ordering::SeqCst),0);
    let receipt=gateway.execute(ToolExecutionRequest{call_id:"allowed".into(),name:"run_command".into(),arguments:json!({"CommandLine":"node -e console.log('local')"})});assert_eq!(receipt.status,ToolExecutionStatus::Succeeded);assert_eq!(count.load(Ordering::SeqCst),1);
    gateway.begin_run();let receipt=gateway.execute(ToolExecutionRequest{call_id:"legacy".into(),name:"run_command".into(),arguments:json!({"CommandLine":"curl https://cmd.im/"})});assert_eq!(receipt.status,ToolExecutionStatus::Succeeded,"managed scope must not leak into another Run");
}

#[test]
fn successful_managed_action_is_external_mutation_not_readonly_observation() {
    use crate::services::brain::{task_progress::TaskProgress,tool_execution_gateway::{ToolExecutionReceipt,ToolExecutionStatus},tool_policy::ToolRisk};
    let mut progress=TaskProgress::default();progress.observe(&ToolExecutionReceipt{call_id:"action-receipt".into(),name:"perform_delivery_action".into(),status:ToolExecutionStatus::Succeeded,duration_ms:1,result:json!({"status":"success","exit_code":0}),original_result_chars:0,truncated:false,stored_result_reference:None,risk:ToolRisk::Network,verification:false});
    let evidence=&progress.snapshot()["recent_evidence"][0];assert_eq!(evidence["kind"],"mutation");assert_eq!(evidence["external"],true);
}

#[test]
fn managed_action_is_exposed_to_model_only_when_host_configured() {
    use std::sync::{Arc,Mutex};
    use crate::domains::agent_cognitive::executor::ToolExecutor;
    use crate::services::brain::task_progress::TaskProgress;
    let progress=Arc::new(Mutex::new(TaskProgress::default()));let tool=PerformDeliveryActionTool(progress.clone());
    assert!(tool.prompt_projection().is_none());
    let (_dir,work,profile)=fixture();progress.lock().unwrap().configure_delivery_profile(&work,"s","r",Some(&profile)).unwrap();
    assert!(tool.prompt_projection().unwrap().contains("Host-managed"),"executor filters non-core tools without a projection");
}

#[test]
fn closeout_strategy_advances_only_with_fresh_source_and_never_retries_action() {
    let (_dir,work,profile)=fixture();
    let mut state=DeliveryEvidence::new(&work,"s","r").unwrap();state.load_profile(&profile).unwrap();
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();
    state.verify(&["rule".into()]).unwrap();
    assert!(state.managed_closeout_strategy().is_none(),"unattempted action still requires its normal workflow");
    state.perform_action("publish").unwrap();
    let missing=state.managed_closeout_strategy().unwrap();
    assert!(missing.contains("publication.json")&&missing.contains("Write the missing"));
    assert!(!work.join("publication.json").exists(),"guidance must not manufacture files");
    assert!(state.completion_gap().is_some());
    fs::write(work.join("publication.json"),"{}").unwrap();
    assert!(state.managed_closeout_strategy().unwrap().contains("Call verify_delivery for ALL"));
    state.verify(&[]).unwrap();
    assert!(state.managed_closeout_strategy().unwrap().contains("give the final answer now"));
    fs::write(work.join("rule.json"),"changed").unwrap();
    assert!(state.managed_closeout_strategy().is_none(),"changed source must restore normal recovery, not closeout");
    assert!(state.completion_gap().is_some());
    assert!(state.perform_action("publish").unwrap_err().contains("already attempted"));
}

#[test]
fn closeout_projection_rejects_changed_profile_and_business_verifier() {
    let (dir,work,profile)=fixture();
    let mut state=DeliveryEvidence::new(&work,"s","r").unwrap();state.load_profile(&profile).unwrap();
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();
    state.verify(&["rule".into()]).unwrap();state.perform_action("publish").unwrap();
    assert!(state.managed_closeout_strategy().is_some());
    let before=fs::read(&profile).unwrap();fs::write(&profile,b"changed profile").unwrap();
    assert!(state.managed_closeout_strategy().is_none());
    fs::write(&profile,before).unwrap();fs::write(dir.path().join("oracle.py"),"changed verifier").unwrap();
    assert!(state.managed_closeout_strategy().is_none());
    assert!(state.completion_gap().is_some());
}

#[test]
fn invalid_plan_shape_preserves_run_authority_and_fresh_evidence() {
    use crate::domains::agent_cognitive::executor::ToolExecutor;
    use std::sync::{Arc,Mutex};
    let (_dir,work,profile)=fixture();
    let progress=Arc::new(Mutex::new(crate::services::brain::task_progress::TaskProgress::default()));
    progress.lock().unwrap().configure_delivery_profile(&work,"s","r",Some(&profile)).unwrap();
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();
    progress.lock().unwrap().verify_delivery(&["rule".into()]).unwrap();
    let before=progress.lock().unwrap().snapshot();
    let mut args=crate::services::brain::task_understanding::tests::sample();
    args["delivery"]=json!({"version":"1","scope":"implementation","target":"wrong-target","artifacts":[]});
    let tool=crate::services::brain::tools::plan_task::RunBoundPlanTaskTool(progress.clone());
    assert!(tool.execute(args).unwrap_err().contains("delivery.version"));
    assert_eq!(progress.lock().unwrap().snapshot(),before);
}

fn receipt_fixture() -> (tempfile::TempDir,PathBuf,PathBuf) {
    let (dir,work,profile)=fixture();
    let action=dir.path().join("action.py");
    let script=fs::read_to_string(&action).unwrap().replace("print(json.dumps(",
        "pathlib.Path(sys.argv[3],'publication.json').write_text(json.dumps({'provider':'issued','public_id':'KEEP'}))\nprint(json.dumps(");
    fs::write(&action,script).unwrap();
    let mut value:Value=serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
    value["validators"][1]["script_sha256"]=json!(file_sha256(&action).unwrap());
    value["actions"][0]["receipt_artifacts"]=json!(["cloud"]);
    fs::write(&profile,serde_json::to_vec(&value).unwrap()).unwrap();
    (dir,work,profile)
}

#[test]
fn issued_receipt_blocks_native_alias_edit_before_executor_and_detects_shell_tampering() {
    use crate::services::brain::tool_execution_gateway::{ToolExecutionGateway,ToolExecutionRequest,ToolExecutionStatus};
    use std::sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}};
    struct Counter(Arc<AtomicUsize>);
    impl crate::domains::agent_cognitive::executor::ToolExecutor for Counter {
        fn name(&self)->&'static str {"write_to_file"} fn description(&self)->&'static str {"test"}
        fn parameters_schema(&self)->Value {json!({})}
        fn execute(&self,_:Value)->Result<Value,String>{self.0.fetch_add(1,Ordering::SeqCst);Ok(json!({"status":"success"}))}
    }
    let (_dir,work,profile)=receipt_fixture();
    let progress=Arc::new(Mutex::new(crate::services::brain::task_progress::TaskProgress::default()));
    progress.lock().unwrap().configure_delivery_profile(&work,"s","r",Some(&profile)).unwrap();
    fs::write(work.join("rule.json"),r#"{"canary":"expected"}"#).unwrap();
    progress.lock().unwrap().verify_delivery(&["rule".into()]).unwrap();
    progress.lock().unwrap().perform_delivery_action("publish").unwrap();
    let count=Arc::new(AtomicUsize::new(0));let registry=Arc::new(Mutex::new(HashMap::new()));
    registry.lock().unwrap().insert("write_to_file".into(),Arc::new(Counter(count.clone())) as Arc<dyn crate::domains::agent_cognitive::executor::ToolExecutor>);
    let gateway=ToolExecutionGateway::new(registry);gateway.bind_delivery_progress(&progress);
    let result=gateway.execute(ToolExecutionRequest{call_id:"overwrite".into(),name:"write_file".into(),arguments:json!({"path":work.join("publication.json"),"content":"{}"})});
    assert_eq!(result.status,ToolExecutionStatus::PolicyDenied);assert_eq!(result.result["kind"],"managed_receipt_immutable");assert_eq!(count.load(Ordering::SeqCst),0);
    let result=gateway.execute(ToolExecutionRequest{call_id:"readme".into(),name:"write_to_file".into(),arguments:json!({"TargetFile":work.join("README.md"),"CodeContent":"explanation"})});
    assert_eq!(result.status,ToolExecutionStatus::Succeeded);assert_eq!(count.load(Ordering::SeqCst),1);
    progress.lock().unwrap().verify_delivery(&[]).unwrap();
    fs::write(work.join("publication.json"),"{}").unwrap();
    assert!(progress.lock().unwrap().verify_delivery(&[]).unwrap_err().contains("modified after issuance"));
    assert!(progress.lock().unwrap().completion_gap().is_some());
}

#[test]
fn deferred_stage_denies_writes_but_new_approval_run_allows_execution() {
    use crate::services::brain::{cognitive_runtime::CognitiveRuntime,tool_policy::ToolRisk};
    let current=CognitiveRuntime::begin("当前文件报错，请先调查并给出修复方案，等我确认后再修改。");
    assert!(current.task_progress().lock().unwrap().rejects_mutation("write_to_file",ToolRisk::WorkspaceMutation));
    assert!(!current.task_progress().lock().unwrap().rejects_mutation("view_file",ToolRisk::ReadOnly));
    let approved=CognitiveRuntime::begin("方案已经确认，现在修改代码并验证。");
    assert!(!approved.task_progress().lock().unwrap().is_read_only_scope());
}
