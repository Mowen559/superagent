use super::*;
use crate::services::brain::{cognitive_runtime::{CognitiveRuntime,ReasoningMode,CompletionDecision},delivery_evidence::{VerifyDeliveryTool,DeliveryContract},task_progress::TaskProgress,tools::fs::{ReadFileTool,WriteFileTool}};
use std::{fs,path::Path,sync::atomic::AtomicUsize};

struct Counter {name:&'static str,error:Option<&'static str>,count:Arc<AtomicUsize>}
impl ToolExecutor for Counter {
    fn name(&self)->&'static str {self.name}
    fn description(&self)->&'static str {"fixture"}
    fn parameters_schema(&self)->Value {json!({})}
    fn execute(&self,_:Value)->Result<Value,String>{self.count.fetch_add(1,Ordering::SeqCst);match self.error {Some(e)=>Ok(json!({"exit_code":1,"stderr":e})),None=>Ok(json!({"files":[]}))}}
}
fn registry(tools:Vec<Arc<dyn ToolExecutor>>)->ToolRegistry {
    Arc::new(Mutex::new(tools.into_iter().map(|t|(t.name().into(),t)).collect()))
}
fn invoke(g:&ToolExecutionGateway,name:&str,args:Value)->ToolExecutionReceipt {
    g.execute(ToolExecutionRequest{call_id:uuid::Uuid::new_v4().to_string(),name:name.into(),arguments:args})
}

#[test]
fn four_failure_classes_are_bounded_before_execution_and_reads_do_not_reset() {
    for (error,class,limit) in [("invalid_arguments: missing field command","arguments",1),("ModuleNotFoundError: no module named fixture","missing_dependency",1),("ECONNREFUSED transient_network","transient_network",2),("trusted verifier failed: business_validation canary","business_validation",2)] {
        let tmp=tempfile::tempdir().unwrap();let count=Arc::new(AtomicUsize::new(0));
        let g=ToolExecutionGateway::with_policy(registry(vec![Arc::new(Counter{name:"run_command",error:Some(error),count:count.clone()}),Arc::new(Counter{name:"list_dir",error:None,count:Arc::new(AtomicUsize::new(0))}),Arc::new(WriteFileTool::new(tmp.path().into()))]),ToolPolicy::new(ToolPolicyMode::Audit));
        for _ in 0..limit {
            let failed=invoke(&g,"bash",json!({"command":"same invocation secret-canary"}));
            assert_eq!(failed.status,ToolExecutionStatus::Failed);assert_eq!(failed.result["recovery_state"]["failure_class"],class);
            for n in 0..6 {assert_eq!(invoke(&g,"list_dir",json!({"path":format!("observation-{n}")})).status,ToolExecutionStatus::Succeeded);}
        }
        let blocked=invoke(&g,"run_command",json!({"CommandLine":"same invocation secret-canary"}));
        assert_eq!(blocked.status,ToolExecutionStatus::RepetitionBlocked,"changing only argument aliases cannot renew the budget");
        let blocked=invoke(&g,"bash",json!({"command":"same invocation secret-canary"}));
        assert_eq!(blocked.status,ToolExecutionStatus::RepetitionBlocked);assert_eq!(blocked.result["kind"],"bounded_recovery_exhausted");
        assert!(!blocked.result["recovery_state"].to_string().contains("secret-canary"));
        let before=count.load(Ordering::SeqCst);assert_eq!(before,limit);
        let different=invoke(&g,"run_command",json!({"command":"changed method"}));assert_eq!(different.status,ToolExecutionStatus::Failed);assert_eq!(count.load(Ordering::SeqCst),before+1);
        let changed=invoke(&g,"write_file",json!({"path":tmp.path().join("repair.txt"),"content":"real repair"}));assert_eq!(changed.status,ToolExecutionStatus::Succeeded);
        assert_eq!(invoke(&g,"bash",json!({"command":"same invocation secret-canary"})).status,ToolExecutionStatus::Failed);
        g.begin_run();assert_eq!(invoke(&g,"bash",json!({"command":"same invocation secret-canary"})).status,ToolExecutionStatus::Failed);
    }
}

#[test]
fn internal_trace_paths_and_explicit_shell_reads_are_denied_before_execution() {
    let tmp=tempfile::tempdir().unwrap();let count=Arc::new(AtomicUsize::new(0));let progress=Arc::new(Mutex::new(TaskProgress::default()));
    progress.lock().unwrap().configure_delivery_profile(tmp.path(),"s","r",None).unwrap();
    let g=ToolExecutionGateway::with_policy(registry(vec![Arc::new(Counter{name:"view_file",error:None,count:count.clone()}),Arc::new(Counter{name:"run_command",error:None,count:count.clone()}),Arc::new(Counter{name:"list_dir",error:None,count:count.clone()})]),ToolPolicy::new(ToolPolicyMode::Audit));g.bind_delivery_progress(&progress);
    for (name,args) in [("cat",json!({"path":tmp.path().join(".superagent/traces/current.json")})),("list_dir",json!({"DirectoryPath":".superagent/traces"})),("bash",json!({"command":"node -e \"fs.readFileSync('.superagent/traces/current.json')\""}))] {
        let denied=invoke(&g,name,args);assert_eq!(denied.status,ToolExecutionStatus::PolicyDenied);assert_eq!(denied.result["kind"],"internal_trace_not_task_evidence");
    }
    assert_eq!(count.load(Ordering::SeqCst),0);
    assert_eq!(invoke(&g,"view_file",json!({"AbsolutePath":tmp.path().join("business/traces.txt")})).status,ToolExecutionStatus::Succeeded);
    assert_eq!(count.load(Ordering::SeqCst),1);
}

fn profile(root:&Path,physical:bool,diagnostics:bool)->std::path::PathBuf {
    let file=root.join("profile.json");fs::write(&file,serde_json::to_vec(&json!({"delivery":{"version":1,"scope":"implementation","target":"documentation","artifacts":[{"id":"readme","path":"README.md","required":true}]},"validators":[],"completion_policy":if physical{"physical_artifacts"}else{"quality_checks"},"allow_internal_diagnostics":diagnostics})).unwrap()).unwrap();file
}

#[test]
fn trusted_diagnostic_opt_in_does_not_leak_across_run_binding() {
    let tmp=tempfile::tempdir().unwrap();let work=tmp.path().join("work");fs::create_dir(&work).unwrap();let config=profile(tmp.path(),false,true);
    let progress=Arc::new(Mutex::new(TaskProgress::default()));progress.lock().unwrap().configure_delivery_profile(&work,"s","r",Some(&config)).unwrap();
    let count=Arc::new(AtomicUsize::new(0));let g=ToolExecutionGateway::with_policy(registry(vec![Arc::new(Counter{name:"view_file",error:None,count:count.clone()})]),ToolPolicy::new(ToolPolicyMode::Audit));g.bind_delivery_progress(&progress);
    let args=json!({"AbsolutePath":work.join(".superagent/traces/x")});assert_eq!(invoke(&g,"view_file",args.clone()).status,ToolExecutionStatus::Succeeded);
    g.begin_run();progress.lock().unwrap().configure_delivery_profile(&work,"s","new",None).unwrap();g.bind_delivery_progress(&progress);
    assert_eq!(invoke(&g,"view_file",args).status,ToolExecutionStatus::PolicyDenied);assert_eq!(count.load(Ordering::SeqCst),1);
}

#[test]
fn physical_document_completion_is_explicit_fresh_and_not_domain_verification() {
    for physical in [false,true] {
        let tmp=tempfile::tempdir().unwrap();let work=tmp.path().join("work");fs::create_dir(&work).unwrap();fs::write(work.join("README.md"),"before").unwrap();let config=profile(tmp.path(),physical,false);
        let mut runtime=CognitiveRuntime::with_mode("Fix the existing README documentation bug in this workspace",ReasoningMode::Adaptive);let progress=runtime.task_progress();
        progress.lock().unwrap().configure_delivery_profile(&work,"s","r",Some(&config)).unwrap();
        let g=ToolExecutionGateway::with_policy(registry(vec![Arc::new(ReadFileTool::new(work.clone())),Arc::new(WriteFileTool::new(work.clone())),Arc::new(VerifyDeliveryTool(progress.clone()))]),ToolPolicy::new(ToolPolicyMode::Audit));g.bind_delivery_progress(&progress);
        for (name,args) in [("view_file",json!({"AbsolutePath":work.join("README.md")})),("write_to_file",json!({"TargetFile":work.join("README.md"),"CodeContent":"Updated documentation with actual import instructions."}))] {let r=invoke(&g,name,args);assert_eq!(r.status,ToolExecutionStatus::Succeeded);runtime.observe_receipt(&r);}
        let verified=invoke(&g,"verify_delivery",json!({}));assert_eq!(verified.status,ToolExecutionStatus::Succeeded);assert_eq!(verified.result["domain_verified"],false);assert_eq!(verified.result["completion_ready"],true);assert!(!verified.verification);runtime.observe_receipt(&verified);
        let decision=runtime.review_completion("The README has been updated with the actual import instructions, and the required documentation file has a fresh physical delivery receipt. No runtime business certification is claimed.");
        assert_eq!(decision==CompletionDecision::Accept,physical,"{decision:?}");
        if physical {fs::write(work.join("README.md"),"changed after validation").unwrap();assert!(!progress.lock().unwrap().host_physical_delivery_complete());assert!(!matches!(runtime.review_completion("Updated and verified documentation is ready for delivery."),CompletionDecision::Accept));}
    }
}

#[test]
fn physical_policy_cannot_be_enabled_by_model_or_cover_business_validators() {
    let tmp=tempfile::tempdir().unwrap();let work=tmp.path().join("work");fs::create_dir(&work).unwrap();let config=profile(tmp.path(),true,false);
    let mut json:Value=serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();json["delivery"]["artifacts"][0]["validator"]=json!("business");fs::write(&config,serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(TaskProgress::default().configure_delivery_profile(&work,"s","r",Some(&config)).unwrap_err().contains("physical_artifacts"));
    let mut declaration=json["delivery"].clone();declaration["completion_policy"]=json!("physical_artifacts");assert!(serde_json::from_value::<DeliveryContract>(declaration).is_err());
}
