use super::*;
use super::super::{run_plan::TaskKind,terminal_receipt::{start_run,finish_run,repair_incomplete_runs}};
use serde_json::json;
use std::fs;
fn config(root:&Path)->RuntimeConfig {RuntimeConfig{workspace_root:root.into(),understand_anything_root:root.join("understand"),shadow_git_root:root.join("cache"),project_root:root.into(),projects_root:root.join("projects"),codebase_memory_cache_root:root.join("graphs"),codebase_memory_bin:"missing".into(),rtk_bin:"unused".into(),sso_api_url:String::new()}}
fn setup()->(tempfile::TempDir,RuntimeConfig,std::path::PathBuf,Vec<SessionEvent>) {
    let root=tempfile::tempdir().unwrap();let workspace=root.path().join("workspace");fs::create_dir(&workspace).unwrap();
    let cfg=config(root.path());let events=super::super::recovery_candidate::tests::successful_workspace_fixture(&workspace);(root,cfg,workspace,events)
}
fn report()->EvolutionReport {EvolutionReport{task_kind:TaskKind::Execution,completed:true,plan_closed:true,replans:0}}
fn run(cfg:&RuntimeConfig,w:&Path,e:&[SessionEvent],policy:MemoryPolicy,checkpoint:PendingCheckpoint)->MemoryAudit {finalize(cfg,w,"s","r",e,Some(&report()),policy,&checkpoint)}
#[test]
fn actual_recovery_stores_matching_readback_ids_and_safe_receipt() {
    let (_root,cfg,w,e)=setup();let a=run(&cfg,&w,&e,MemoryPolicy::default(),None);
    assert_eq!(a.decisions.len(),4);assert!(a.decisions[..3].iter().all(|d|d.status==PersistenceStatus::Stored));
    assert_eq!(a.decisions[3].reason,"not_requested");
    let memory=EvolutionMemory::for_project(&cfg,&w);let lesson=memory.list_lessons().unwrap().pop().unwrap();
    assert_eq!(a.decisions[1].record_ids,vec![lesson.id.clone()]);assert_eq!(a.decisions[1].readback_digests,vec![digest(&lesson).unwrap()]);
    let notes=SupermemoryStore::from_path(project_memory::managed_scope(&cfg,&w).1).load_result().unwrap();
    let note=notes.memories.iter().find(|n|a.decisions[2].record_ids.contains(&n.id)).unwrap();
    assert_eq!(a.decisions[2].readback_digests,vec![digest(&(note,notes.memory_context.get(&note.id).unwrap())).unwrap()]);
    assert!(!w.join(".supermemory").exists());
}
#[test]
fn read_only_intent_policy_and_leaf_skip_all_without_touching_store() {
    for policy in [MemoryPolicy{read_only_intent:true,..Default::default()},MemoryPolicy{read_only_policy:true,..Default::default()},MemoryPolicy{delegated:true,..Default::default()}] {
        let (_root,cfg,w,e)=setup();let a=run(&cfg,&w,&e,policy,Some(Ok("summary".into())));
        assert!(a.decisions.iter().all(|d|d.status==PersistenceStatus::Skipped&&d.record_ids.is_empty()&&d.readback_digests.is_empty()));
        assert!(!cfg.projects_root.exists(),"a skipped run must not even create a memory lock file");
    }
}
#[test]
fn outer_failure_overrides_inner_completed_report_and_never_saves_recovery() {
    let (_root,cfg,w,mut e)=setup();let SessionEvent::RunFinished{outcome,..}=e.last_mut().unwrap() else{panic!()};*outcome=RunOutcome::Failed;
    let a=run(&cfg,&w,&e,MemoryPolicy::default(),Some(Ok("unverified summary".into())));
    assert_eq!(a.decisions[0].status,PersistenceStatus::Stored);assert!(a.decisions[1..].iter().all(|d|d.status==PersistenceStatus::Skipped&&d.reason=="run_not_completed"));
    let memory=EvolutionMemory::for_project(&cfg,&w);assert!(memory.list_lessons().unwrap().is_empty());let observed=memory.list().unwrap();assert_eq!(observed[0].failures,1);assert_eq!(observed[0].successes,0);
}
#[test]
fn valid_task_without_a_repair_records_a_specific_skip() {
    let (_root,cfg,w,_)=setup();let mut e=vec![];start_run(&mut e,"r".into()).unwrap();finish_run(&mut e,"r",RunOutcome::Completed,None).unwrap();
    let a=run(&cfg,&w,&e,MemoryPolicy::default(),None);assert_eq!(a.decisions[1].reason,"no_matched_recovery");assert_eq!(a.decisions[2].reason,"no_matched_recovery");
    let a=finalize(&cfg,&w,"s","r",&e,None,MemoryPolicy::default(),&None);assert_eq!(a.decisions[0].reason,"no_execution_report");
}
#[test]
fn real_write_failure_preserves_blocking_directories_and_never_reports_stored() {
    let (_root,cfg,w,e)=setup();let (_,notes)=project_memory::managed_scope(&cfg,&w);let evolution=notes.with_file_name("evolution.json");
    for target in [&notes,&evolution] {fs::create_dir_all(target).unwrap();fs::write(target.join("sentinel"),"unchanged").unwrap();}
    let a=run(&cfg,&w,&e,MemoryPolicy::default(),Some(Ok("summary".into())));
    assert!(a.decisions.iter().all(|d|d.status==PersistenceStatus::Failed&&d.record_ids.is_empty()));
    assert!(a.decisions.iter().all(|d|d.reason=="write_failed"));
    for target in [&notes,&evolution]{assert_eq!(fs::read_to_string(target.join("sentinel")).unwrap(),"unchanged");}
}
#[test]
fn sensitive_candidate_checkpoint_and_identifiers_are_rejected_without_echo() {
    let (_root,cfg,w,mut e)=setup();let canary="sk-memory-canary-12345678901234567890";
    for event in &mut e {if let SessionEvent::ToolResult{result,..}=event {if let Some(v)=result.get_mut(recovery_candidate::FIELD){v["sessionId"]=json!(canary);}}}
    let a=finalize(&cfg,&w,canary,"r",&e,None,MemoryPolicy::default(),&Some(Ok(canary.into())));
    assert_eq!(a.decisions[1].status,PersistenceStatus::Rejected);assert_eq!(a.decisions[2].status,PersistenceStatus::Rejected);assert_eq!(a.decisions[3].status,PersistenceStatus::Rejected);
    assert!(!serde_json::to_string(&a).unwrap().contains(canary));assert!(!cfg.projects_root.exists());
    let lesson=DistilledLesson{id:"manual".into(),domain:"manual".into(),trigger_scenario:"manual".into(),trap_pattern:canary.into(),golden_solution:"ignored".into(),confidence_score:0.0,usage_count:0,updated_at:String::new(),recovery_evidence:None};
    let memory=EvolutionMemory::for_project(&cfg,&w);assert!(memory.record_lesson(lesson).is_err());assert!(!cfg.projects_root.exists());
}
#[test]
fn stale_and_nonterminal_runs_are_skipped_without_claiming_a_fix() {
    let (_root,cfg,w,e)=setup();fs::write(w.join("source.js"),"module.exports=9;").unwrap();let a=run(&cfg,&w,&e,MemoryPolicy::default(),None);assert_eq!(a.decisions[1].reason,"stale_workspace");
    let a=run(&cfg,&w,&e[..e.len()-1],MemoryPolicy::default(),None);assert!(a.decisions.iter().all(|d|d.reason=="run_not_terminal"));
}
#[test]
fn audit_is_idempotent_and_not_projected_or_replayed_after_restart() {
    let (_root,cfg,w,mut e)=setup();let a=run(&cfg,&w,&e,MemoryPolicy::default(),None);e.push(SessionEvent::MemoryPersistenceRecorded{receipt:a.clone()});
    let before=fs::read(project_memory::managed_scope(&cfg,&w).1).unwrap();let a2=run(&cfg,&w,&e,MemoryPolicy::default(),None);
    assert_eq!(serde_json::to_value(a).unwrap(),serde_json::to_value(a2).unwrap());assert_eq!(before,fs::read(project_memory::managed_scope(&cfg,&w).1).unwrap());
    let projected=crate::domains::agent_cognitive::event_projection::project_events_with_micro_compaction(&e,3);
    assert!(!serde_json::to_string(&projected).unwrap().contains("memory_audit_internal"));
    start_run(&mut e,"interrupted".into()).unwrap();assert_eq!(repair_incomplete_runs(&mut e),1);assert_eq!(recover_missing_audits(&mut e,"s"),1);assert_eq!(recover_missing_audits(&mut e,"s"),0);
    let SessionEvent::MemoryPersistenceRecorded{receipt}=e.last().unwrap() else{panic!()};assert!(receipt.decisions.iter().all(|d|d.reason=="interrupted_no_replay"));
    assert_eq!(before,fs::read(project_memory::managed_scope(&cfg,&w).1).unwrap());
}
#[test]
fn actual_readback_corruption_and_scope_mismatch_fail_confirmation() {
    let (_root,cfg,w,_)=setup();let (_,path)=project_memory::managed_scope(&cfg,&w);let store=SupermemoryStore::from_path(&path);
    let record:SupermemoryRecord=serde_json::from_value(json!({"id":"test","name":"candidate","summary":"safe"})).unwrap();let context=project_memory::new_context(&w,&w,None,None).unwrap();
    store.write_context_memory(record.clone(),context.clone()).unwrap();assert!(confirm_record(&store,&record,&context).is_ok());
    let mut other=context.clone();other.source="other".into();assert_eq!(confirm_record(&store,&record,&other).unwrap_err(),PersistenceFailure::ReadbackMismatch);
    fs::write(&path,"not valid JSON").unwrap();assert_eq!(confirm_record(&store,&record,&context).unwrap_err(),PersistenceFailure::ReadbackFailed);
}
#[test]
fn concurrent_writers_preserve_all_confirmed_strategy_observations() {
    let root=tempfile::tempdir().unwrap();let cfg=config(root.path());let memory=EvolutionMemory::for_project(&cfg,root.path());
    std::thread::scope(|scope| {for _ in 0..4 {let memory=memory.clone();scope.spawn(move||{for _ in 0..5{memory.observe_confirmed(&report()).unwrap();}});}});
    let observed=memory.list().unwrap();assert_eq!(observed.len(),1);assert_eq!(observed[0].successes,20);
}

#[test]
fn compaction_retains_physical_audits_but_excludes_them_from_model_summary() {
    let mut e=vec![];start_run(&mut e,"past".into()).unwrap();
    e.push(SessionEvent::ToolCall{id:"read".into(),name:"view_file".into(),arguments:json!({"path":"data.txt"})});
    e.push(SessionEvent::ToolResult{id:"read".into(),result:json!({"stdout":"safe",recovery_candidate::FIELD:{"private":"PRIVATE_AUDIT_PROOF"}})});
    finish_run(&mut e,"past",RunOutcome::Completed,None).unwrap();
    e.push(SessionEvent::MemoryPersistenceRecorded{receipt:skipped_audit("s","past","project","read_only_task")});
    for _ in 0..25 {e.push(SessionEvent::ModelThought("historical observation ".repeat(100)));}
    start_run(&mut e,"new".into()).unwrap();e.push(SessionEvent::UserInput("Continue a new task".into()));
    let result=super::super::compaction_runtime::CompactionRuntime::default().compact_if_needed(&mut e,3000);
    assert!(matches!(result,super::super::compaction_runtime::CompactionOutcome::Compacted{..}));
    assert_eq!(e.iter().filter(|e|matches!(e,SessionEvent::MemoryPersistenceRecorded{..})).count(),1);
    assert!(e.iter().any(|e|matches!(e,SessionEvent::RunFinished{run_id,..} if run_id=="past")));
    let projected=crate::domains::agent_cognitive::event_projection::project_events_with_micro_compaction(&e,3);
    let serialized=serde_json::to_string(&projected).unwrap();assert!(!serialized.contains("PRIVATE_AUDIT_PROOF"));assert!(!serialized.contains("memory_audit_internal"));
}
