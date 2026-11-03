//! Run-local failed invocation budgets. No argument text or raw error is stored.
use super::{tool_call_identity::fingerprint,tool_execution_gateway::ToolExecutionStatus};
use serde_json::{json,Value};
use std::collections::HashMap;

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum FailureKind { Arguments, MissingDependency, TransientNetwork, BusinessValidation, Policy, Unknown }
impl FailureKind {
    fn label(self)->&'static str {match self {Self::Arguments=>"arguments",Self::MissingDependency=>"missing_dependency",Self::TransientNetwork=>"transient_network",Self::BusinessValidation=>"business_validation",Self::Policy=>"policy",Self::Unknown=>"unknown"}}
    fn limit(self)->usize {match self {Self::Arguments|Self::MissingDependency|Self::Policy=>1,_=>2}}
    fn advice(self)->&'static str {match self {
        Self::Arguments=>"Correct the actual tool parameters or choose a registered tool before retrying. Do not repeat unchanged invalid input.",
        Self::MissingDependency=>"Check the named dependency and available runtime. Restore it only within authorized scope or use an evidenced alternative; do not repeat the same missing-dependency command.",
        Self::TransientNetwork=>"Allow at most one identical retry for the transient network failure. Then change an evidenced transport or report external unavailability; a direct probe does not replace required runtime verification.",
        Self::BusinessValidation=>"Inspect the actual artifact and validator error, repair the source, then rerun the required validator. Repeated direct endpoint probes cannot certify the artifact.",
        Self::Policy=>"Respect the tool boundary. Use the supported scoped tool/result reader or report the limitation; do not search internal prompts or bypass authorization.",
        Self::Unknown=>"Read the scoped full tool result, identify a concrete cause and change the approach. Do not repeat unchanged failures indefinitely.",
    }}
}
pub fn classify(name:&str,status:ToolExecutionStatus,result:&Value)->FailureKind {
    if matches!(status,ToolExecutionStatus::PolicyDenied|ToolExecutionStatus::ApprovalRequired) {return FailureKind::Policy;}
    if status==ToolExecutionStatus::NotFound {return FailureKind::Arguments;}
    let text=["kind","error","stderr","stdout"].iter().filter_map(|k|result.get(k).and_then(Value::as_str))
        .map(|s|s.chars().take(8192).collect::<String>().to_lowercase()).collect::<Vec<_>>().join(" ");
    let contains=|patterns:&[&str]|patterns.iter().any(|p|text.contains(p));
    if contains(&["invalid_arguments","missing field","is required","invalid parameter","expected a string","invalid type"]) {FailureKind::Arguments}
    else if contains(&["missing_dependency","modulenotfounderror","cannot find module","no module named","commandnotfoundexception","not recognized as an internal"]) {FailureKind::MissingDependency}
    else if contains(&["transient_network","econnrefused","econnreset","connecttimeout","connectionrefused","connection refused","timed out","timeout","http 429","http 502","http 503"]) {FailureKind::TransientNetwork}
    else if name=="verify_delivery"||contains(&["business_validation","trusted verifier failed"]) {FailureKind::BusinessValidation}
    else {FailureKind::Unknown}
}
#[derive(Debug)]
struct Failure {kind:FailureKind,attempts:usize}
#[derive(Debug,Default)]
pub struct ToolRecovery {revision:u64,failed:HashMap<String,Failure>,last:Option<String>,observations:usize}
impl ToolRecovery {
    pub fn reset(&mut self){*self=Self::default();}
    fn key(&self,name:&str,args:&Value)->String {
        let mut effective=args.clone();
        if let Some(object)=effective.as_object_mut() {
            let aliases:&[&str]=match name {
                "run_command"=>&["command","command_line","cmd","script"],
                "view_file"=>&["path","file_path","filePath","file","TargetFile"],
                _=>&[],
            };
            for alias in aliases {object.remove(*alias);}
        }
        fingerprint("failed_invocation",&json!({"call":fingerprint(name,&effective),"revision":self.revision}))
    }
    pub fn before_call(&self,name:&str,args:&Value)->Option<Value> {
        let key=self.key(name,args);let failure=self.failed.get(&key)?;
        // Approval state is owned by the live policy store, not workspace
        // revision. Recheck it so a newly granted approval can take effect.
        if failure.kind==FailureKind::Policy {return None;}
        (failure.attempts>=failure.kind.limit()).then(||json!({"kind":"bounded_recovery_exhausted","error":"unchanged failed invocation budget exhausted; change inputs or repair the workspace before retrying","failure_class":failure.kind.label(),"attempts":failure.attempts,"limit":failure.kind.limit(),"fingerprint":key,"recovery_hint":failure.kind.advice()}))
    }
    pub fn observe(&mut self,name:&str,args:&Value,status:ToolExecutionStatus,result:&Value,successful_mutation:bool) {
        if successful_mutation {self.revision=self.revision.saturating_add(1);self.failed.clear();self.last=None;self.observations=0;return;}
        if status==ToolExecutionStatus::RepetitionBlocked {return;}
        let key=self.key(name,args);
        if status==ToolExecutionStatus::Succeeded {
            self.failed.remove(&key);if self.last.as_ref()==Some(&key){self.last=None;}
            if self.last.is_some(){self.observations=self.observations.saturating_add(1);}return;
        }
        let kind=classify(name,status,result);
        // Preserve every failed fingerprint until a revision or Run reset;
        // evicting old failures would silently renew an exhausted retry budget.
        let failure=self.failed.entry(key.clone()).or_insert(Failure{kind,attempts:0});
        failure.attempts=failure.attempts.saturating_add(1);self.last=Some(key);self.observations=0;
    }
    pub fn snapshot(&self)->Value {
        let Some(f)=self.last.as_ref().and_then(|k|self.failed.get(k)) else {return json!({"active":false})};
        let stagnation=if self.observations>=4 {" Several successful observations have not repaired the failed artifact. Advance to a concrete source correction, required verification, scoped replan or honest external-blocker report; do not browse internal traces."}else{""};
        json!({"active":true,"failure_class":f.kind.label(),"attempts":f.attempts,"limit":f.kind.limit(),"observations_since_failure":self.observations,"hint":format!("{}{}",f.kind.advice(),stagnation)})
    }
}
