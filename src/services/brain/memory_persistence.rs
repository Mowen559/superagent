//! One terminal automatic-memory decision per Run. Audit is session metadata,
//! never a recalled lesson or a model instruction. All reasons are fixed codes.
use super::{events::{RunOutcome,SessionEvent},evolution_memory::{DistilledLesson,EvolutionMemory,EvolutionReport},project_memory,recovery_candidate::{self,RecoveryCandidate}};
use crate::{config::runtime::RuntimeConfig,models::memory::SupermemoryRecord,services::supermemory_store::SupermemoryStore};
use serde::{Serialize,Deserialize};
use std::path::Path;

#[derive(Debug,Clone,Copy,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum PersistenceStatus {Stored,Skipped,Rejected,Failed}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct MemoryDecision {
    pub stage:String,pub status:PersistenceStatus,pub reason:String,
    pub record_ids:Vec<String>,pub readback_digests:Vec<String>,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct MemoryAudit {
    pub schema_version:u8,pub session_id:String,pub run_id:String,pub project_digest:String,
    pub decisions:Vec<MemoryDecision>,
}
#[derive(Debug,Clone,Copy,Default)]
pub struct MemoryPolicy {pub read_only_intent:bool,pub read_only_policy:bool,pub delegated:bool}
impl MemoryPolicy {
    pub fn skip_reason(&self)->Option<&'static str> {
        if self.read_only_intent {Some("read_only_task")}else if self.read_only_policy{Some("read_only_policy")}else if self.delegated{Some("delegated_run")}else{None}
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum PersistenceFailure {Sensitive,WriteFailed,ReadbackFailed,ReadbackMismatch,SerializationFailed,SynthesisFailed}
impl PersistenceFailure {
    pub fn reason(self)->&'static str {match self {Self::Sensitive=>"sensitive_candidate",Self::WriteFailed=>"write_failed",Self::ReadbackFailed=>"readback_failed",Self::ReadbackMismatch=>"readback_mismatch",Self::SerializationFailed=>"serialization_failed",Self::SynthesisFailed=>"checkpoint_model_failed"}}
}
pub type PendingCheckpoint=Option<Result<String,PersistenceFailure>>;
#[derive(Debug)]
pub struct StoredRecord {pub id:String,pub digest:String}
fn digest(value:&impl Serialize)->Result<String,PersistenceFailure> {
    serde_json::to_vec(value).map(|b|crate::shared_kernel::crypto::sha256_hex(&b)).map_err(|_|PersistenceFailure::SerializationFailed)
}
fn safe_id(id:&str)->String {
    if id.len()>128 || project_memory::reject_sensitive(&id).is_err() {format!("sha256:{}",crate::shared_kernel::crypto::sha256_hex(id.as_bytes()))}else{id.into()}
}
fn skipped(stage:&str,reason:&str)->MemoryDecision {MemoryDecision{stage:stage.into(),status:PersistenceStatus::Skipped,reason:reason.into(),record_ids:vec![],readback_digests:vec![]}}
fn failed(stage:&str,error:PersistenceFailure)->MemoryDecision {MemoryDecision{stage:stage.into(),status:if error==PersistenceFailure::Sensitive{PersistenceStatus::Rejected}else{PersistenceStatus::Failed},reason:error.reason().into(),record_ids:vec![],readback_digests:vec![]}}
fn confirmed(stage:&str,result:Result<Vec<StoredRecord>,PersistenceFailure>)->MemoryDecision {
    match result {Ok(records) if !records.is_empty()=>MemoryDecision{stage:stage.into(),status:PersistenceStatus::Stored,reason:"write_and_readback_confirmed".into(),record_ids:records.iter().map(|r|r.id.clone()).collect(),readback_digests:records.into_iter().map(|r|r.digest).collect()},Ok(_)=>failed(stage,PersistenceFailure::ReadbackMismatch),Err(e)=>failed(stage,e)}
}
pub fn skipped_audit(session:&str,run:&str,project:&str,reason:&str)->MemoryAudit {
    MemoryAudit{schema_version:1,session_id:safe_id(session),run_id:safe_id(run),project_digest:crate::shared_kernel::crypto::sha256_hex(project.as_bytes()),
        decisions:["strategy_observation","execution_candidate","reflection_candidate","run_checkpoint"].into_iter().map(|s|skipped(s,reason)).collect()}
}
/// A missing audit is unknown history, never a reason to replay persistence.
pub fn recover_missing_audits(events:&mut Vec<SessionEvent>,session:&str)->usize {
    let missing:Vec<_>=events.iter().filter_map(|e|match e {
        SessionEvent::RunFinished{run_id,outcome,..} if !events.iter().any(|e|matches!(e,SessionEvent::MemoryPersistenceRecorded{receipt} if receipt.run_id==safe_id(run_id)))=>Some((run_id.clone(),*outcome)),_=>None,
    }).collect();
    for (run,outcome) in &missing {
        events.push(SessionEvent::MemoryPersistenceRecorded{receipt:skipped_audit(session,run,"unknown_historical_project",if *outcome==RunOutcome::Interrupted{"interrupted_no_replay"}else{"prior_attempt_unknown_no_replay"})});
    }
    missing.len()
}

fn io_failure(error:std::io::Error)->PersistenceFailure {
    match error.to_string().as_str(){"sensitive memory rejected"=>PersistenceFailure::Sensitive,"readback missing"=>PersistenceFailure::ReadbackFailed,"readback mismatch"=>PersistenceFailure::ReadbackMismatch,_=>PersistenceFailure::WriteFailed}
}
/// Confirm the entire stored object and ownership context, not merely a count.
fn persist_record(config:&RuntimeConfig,workspace:&Path,record:SupermemoryRecord,source:&str,kind:&str)->Result<StoredRecord,PersistenceFailure> {
    project_memory::reject_sensitive(&record).map_err(|_|PersistenceFailure::Sensitive)?;
    let (project,path)=project_memory::managed_scope(config,workspace);
    let mut context=project_memory::new_context(&project,workspace,None,None).map_err(|_|PersistenceFailure::WriteFailed)?;
    context.source=source.into();context.kind=kind.into();
    project_memory::reject_sensitive(&context).map_err(|_|PersistenceFailure::Sensitive)?;
    let store=SupermemoryStore::from_path(path);
    store.write_context_memory(record.clone(),context.clone()).map_err(|_|PersistenceFailure::WriteFailed)?;
    let confirmed=confirm_record(&store,&record,&context)?;
    if source.starts_with("automatic_") {
        super::candidate_governance::Governance::new(config,workspace).propose_memory(&record).map_err(|_|PersistenceFailure::WriteFailed)?;
    }
    Ok(confirmed)
}
fn confirm_record(store:&SupermemoryStore,record:&SupermemoryRecord,context:&crate::models::memory::MemoryContext)->Result<StoredRecord,PersistenceFailure> {
    let saved=store.load_result().map_err(|_|PersistenceFailure::ReadbackFailed)?;
    let actual=saved.memories.iter().find(|r|r.id==record.id).ok_or(PersistenceFailure::ReadbackMismatch)?;
    let actual_context=saved.memory_context.get(&record.id).ok_or(PersistenceFailure::ReadbackMismatch)?;
    if digest(actual)?!=digest(&record)? || digest(actual_context)?!=digest(&context)? {return Err(PersistenceFailure::ReadbackMismatch);}
    Ok(StoredRecord{id:record.id.clone(),digest:digest(&(actual,actual_context))?})
}
pub fn persist_reflection(config:&RuntimeConfig,workspace:&Path,evidence:&RecoveryCandidate)->Result<StoredRecord,PersistenceFailure> {
    project_memory::reject_sensitive(evidence).map_err(|_|PersistenceFailure::Sensitive)?;
    let record=SupermemoryRecord {
        id:format!("distilled_{}",uuid::Uuid::new_v4().simple()),name:"Observed recovery candidate".into(),
        summary:format!("Pattern: {}\nObservation: {}\nEvidence: {}",evidence.failure_reason,evidence.summary(),serde_json::to_string(evidence).map_err(|_|PersistenceFailure::SerializationFailed)?),
        tags:vec!["distilled_rule".into(),"reflexion".into()],file_path:String::new(),complexity:"unverified_candidate".into(),node_type:"reflexion".into(),source_graph_path:String::new(),source_label:"PostMortemReflexion".into(),updated_at:chrono::Utc::now().to_rfc3339(),embedding:None,
    };
    persist_record(config,workspace,record,"automatic_reflection","experience_candidate")
}
pub fn persist_lesson_draft(config:&RuntimeConfig,workspace:&Path,record:SupermemoryRecord)->Result<StoredRecord,PersistenceFailure>{
    persist_record(config,workspace,record,"automatic_lesson_draft","experience_candidate")
}

fn persist_execution(memory:&EvolutionMemory,evidence:RecoveryCandidate)->Result<StoredRecord,PersistenceFailure> {
    let lesson=DistilledLesson {
        id:format!("lesson-{}",uuid::Uuid::new_v4()),domain:"observed_recovery_candidate".into(),
        trigger_scenario:format!("Same-check recovery {}",evidence.failure.action_digest),trap_pattern:evidence.failure_reason.clone(),golden_solution:evidence.summary(),confidence_score:0.0,usage_count:0,updated_at:chrono::Utc::now().to_rfc3339(),recovery_evidence:Some(evidence),
    };
    project_memory::reject_sensitive(&lesson).map_err(|_|PersistenceFailure::Sensitive)?;
    let actual=memory.record_lesson_confirmed(lesson).map_err(io_failure)?;
    Ok(StoredRecord{id:actual.id.clone(),digest:digest(&actual)?})
}
fn persist_checkpoint(config:&RuntimeConfig,workspace:&Path,content:&str)->Result<StoredRecord,PersistenceFailure> {
    project_memory::reject_sensitive(&content).map_err(|_|PersistenceFailure::Sensitive)?;
    let record=SupermemoryRecord{id:format!("checkpoint_{}",uuid::Uuid::new_v4().simple()),name:"Unverified run checkpoint".into(),summary:content.into(),tags:vec!["checkpoint".into()],file_path:String::new(),complexity:"unverified_candidate".into(),node_type:"synthesis".into(),source_graph_path:String::new(),source_label:"automatic_checkpoint".into(),updated_at:chrono::Utc::now().to_rfc3339(),embedding:None};
    persist_record(config,workspace,record,"automatic_checkpoint","run_checkpoint")
}

pub fn finalize(config:&RuntimeConfig,workspace:&Path,session:&str,run:&str,events:&[SessionEvent],report:Option<&EvolutionReport>,policy:MemoryPolicy,checkpoint:&PendingCheckpoint)->MemoryAudit {
    if let Some(existing)=events.iter().find_map(|e|match e {SessionEvent::MemoryPersistenceRecorded{receipt} if receipt.run_id==safe_id(run)&&receipt.session_id==safe_id(session)=>Some(receipt.clone()),_=>None}) {return existing;}
    let (project,_)=project_memory::managed_scope(config,workspace);
    let identity=project_memory::project_identity(&project);
    let mut audit=skipped_audit(session,run,&identity,"no_matched_recovery");
    if let Some(reason)=policy.skip_reason(){return skipped_audit(session,run,&identity,reason);}
    let terminal=events.iter().rev().find_map(|e|match e {SessionEvent::RunFinished{run_id,outcome,..} if run_id==run=>Some(*outcome),_=>None});
    if terminal.is_none(){return skipped_audit(session,run,&identity,"run_not_terminal");}
    let memory=EvolutionMemory::for_project(config,workspace);
    audit.decisions[0]=match report {
        None=>skipped("strategy_observation","no_execution_report"),
        Some(report)=>{
            let mut report=report.clone();report.completed=terminal==Some(RunOutcome::Completed);
            confirmed("strategy_observation",memory.observe_confirmed(&report).map_err(io_failure).and_then(|records|records.into_iter().map(|r|Ok(StoredRecord{id:r.id.clone(),digest:digest(&r)?})).collect()))
        }
    };
    if terminal!=Some(RunOutcome::Completed) {
        for d in audit.decisions.iter_mut().skip(1){d.reason="run_not_completed".into();}return audit;
    }
    let candidate=recovery_candidate::extract_raw(events,false).into_iter().next();
    if let Some(evidence)=candidate {
        if project_memory::reject_sensitive(&evidence).is_err() {
            audit.decisions[1]=failed("execution_candidate",PersistenceFailure::Sensitive);audit.decisions[2]=failed("reflection_candidate",PersistenceFailure::Sensitive);
        } else if evidence.failure.run_id!=run || evidence.failure.session_id!=session {
            audit.decisions[1].reason="evidence_scope_mismatch".into();audit.decisions[2].reason="evidence_scope_mismatch".into();
        } else if !recovery_candidate::is_current(&evidence,workspace) {
            audit.decisions[1].reason="stale_workspace".into();audit.decisions[2].reason="stale_workspace".into();
        } else {
            audit.decisions[1]=confirmed("execution_candidate",persist_execution(&memory,evidence.clone()).map(|r|vec![r]));
            audit.decisions[2]=confirmed("reflection_candidate",persist_reflection(config,workspace,&evidence).map(|r|vec![r]));
        }
    }
    audit.decisions[3]=match checkpoint {None=>skipped("run_checkpoint","not_requested"),Some(Err(e))=>failed("run_checkpoint",*e),Some(Ok(content))=>confirmed("run_checkpoint",persist_checkpoint(config,workspace,content).map(|r|vec![r]))};
    audit
}

#[cfg(test)]
#[path="memory_persistence_tests.rs"]
mod tests;
