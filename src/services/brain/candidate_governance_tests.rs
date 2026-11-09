use super::*;
use serde_json::json;
fn config(root:&Path)->RuntimeConfig{RuntimeConfig{workspace_root:root.into(),understand_anything_root:root.join("understand"),shadow_git_root:root.join("cache"),project_root:root.into(),projects_root:root.join("projects"),codebase_memory_cache_root:root.join("graphs"),codebase_memory_bin:"missing".into(),rtk_bin:"unused".into(),sso_api_url:String::new()}}
const GOOD:&str="module.exports = input => Array.isArray(input) ? input.filter(x=>typeof x==='string').map(x=>x.trim()).filter(Boolean) : [];";
fn markdown(code:&str)->String{format!("---\nname: normalize-heldout\ndescription: normalization draft\n---\n\nUnverified normalization proposal.\n```js\n{code}\n```\n")}
struct Fixture{_root:tempfile::TempDir,cfg:RuntimeConfig,source:PathBuf,target:PathBuf,oracle:PathBuf,store:Governance}
fn fixture()->Fixture{
 let root=tempfile::tempdir().unwrap();let cfg=config(root.path());let source=root.path().join("source");let target=root.path().join("different-target");let oracle=root.path().join("trusted/check.cjs");
 fs::create_dir_all(&source).unwrap();fs::create_dir_all(&target).unwrap();fs::create_dir_all(oracle.parent().unwrap()).unwrap();
 fs::write(source.join("training.json"),r#"{"training":["one"]}"#).unwrap();
 fs::write(target.join("cases.json"),r#"[[null,[]],[[" A ",7,null,"B"," "],["A","B"]],[[""," z "],["z"]]]"#).unwrap();
 fs::write(&oracle,r#"const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');const [file,target,out]=process.argv.slice(2);let text=fs.readFileSync(file,'utf8');if(file.endsWith('.json'))text=JSON.parse(text).summary;const match=text.match(/```js\n([\s\S]*?)\n```/);assert.ok(match,'missing actual proposal');const sandbox={module:{exports:null}};vm.runInNewContext(match[1],sandbox,{timeout:500});for(const [input,expected] of JSON.parse(fs.readFileSync(target+'/cases.json','utf8'))){assert.deepEqual(JSON.parse(JSON.stringify(sandbox.module.exports(input))),expected);}fs.writeFileSync(out+'/oracle-proof.json',JSON.stringify({assertionsPassed:true,cases:3}));"#).unwrap();
 let store=Governance::new(&cfg,&source);let f=Fixture{_root:root,cfg,source,target,oracle,store};profiles(&f,30);f
}
fn profiles(f:&Fixture,timeout:u64){let p=HoldoutProfiles{schema_version:1,suites:vec![HoldoutSuite{id:"normalization".into(),timeout_seconds:timeout,cases:vec![HoldoutCase{id:"different-inputs".into(),target_path:f.target.clone(),target_fingerprint:fingerprint(&f.target).unwrap(),oracle_path:f.oracle.clone(),oracle_sha256:file_hash(&f.oracle).unwrap()}]}]};fs::create_dir_all(f.store.profile_path().parent().unwrap()).unwrap();fs::write(f.store.profile_path(),serde_json::to_vec_pretty(&p).unwrap()).unwrap();}
fn draft(f:&Fixture,code:&str)->Candidate{f.store.propose_skill("normalize","normalization draft",&markdown(code)).unwrap()}
fn activate(f:&Fixture,c:&Candidate)->Candidate{let checked=f.store.validate(&c.id,c.revision,"normalization").unwrap();assert_eq!(checked.last_result,"heldout_profile_passed_not_general_certification");f.store.promote(&c.id,checked.revision).unwrap()}
#[test]
fn unvalidated_draft_is_outside_active_roots_and_promotion_requires_real_receipt(){
 let f=fixture();let c=draft(&f,GOOD);assert!(f.store.active().is_empty());assert!(f.store.promote(&c.id,1).is_err());assert!(!f.source.join(".agents").exists());
 let discovered=crate::domains::agent_cognitive::capability_scout::CapabilityScout::discover_skills_with_config(&f.source,&f.cfg);assert!(!discovered.iter().any(|s|s.name==c.name));
 let c=activate(&f,&c);assert_eq!(c.state,CandidateState::Active);let discovered=crate::domains::agent_cognitive::capability_scout::CapabilityScout::discover_skills_with_config(&f.source,&f.cfg);assert!(discovered.iter().any(|s|s.name==c.name));
 assert_eq!(c.validation.as_ref().unwrap().cases[0].exit_code,0);assert!(f.store.output_root().is_dir());
}
#[test]
fn incorrect_implementation_fails_actual_different_target_assertions(){let f=fixture();let c=draft(&f,"module.exports = input => input;");let checked=f.store.validate(&c.id,1,"normalization").unwrap();assert_eq!(checked.last_result,"holdout_check_failed");assert!(checked.validation.is_none());assert!(f.store.promote(&c.id,checked.revision).is_err());assert!(f.store.active().is_empty());}
#[test]
fn same_source_or_reused_content_is_not_a_holdout(){let f=fixture();let c=draft(&f,GOOD);let mut p:HoldoutProfiles=serde_json::from_slice(&fs::read(f.store.profile_path()).unwrap()).unwrap();p.suites[0].cases[0].target_path=f.source.clone();p.suites[0].cases[0].target_fingerprint=fingerprint(&f.source).unwrap();fs::write(f.store.profile_path(),serde_json::to_vec(&p).unwrap()).unwrap();assert_eq!(f.store.validate(&c.id,1,"normalization").unwrap_err(),"holdout_not_independent");}
#[test]
fn revocation_prevents_future_reads_without_removing_user_files(){let f=fixture();let c=activate(&f,&draft(&f,GOOD));let user=f.source.join(".agents/skills/user/SKILL.md");fs::create_dir_all(user.parent().unwrap()).unwrap();fs::write(&user,"# User owned skill").unwrap();let before=fs::read(&user).unwrap();let revoked=f.store.revoke(&c.id,c.revision).unwrap();assert_eq!(revoked.state,CandidateState::Revoked);assert!(f.store.active().is_empty());assert!(f.store.promote(&c.id,revoked.revision).is_err());assert_eq!(before,fs::read(user).unwrap());assert!(f.store.payload_path(&c).is_file());}
#[test]
fn source_content_oracle_and_input_changes_invalidate_activation(){
 for case in 0..4 {let f=fixture();let c=activate(&f,&draft(&f,GOOD));match case{0=>fs::write(f.source.join("training.json"),"changed source").unwrap(),1=>fs::write(f.store.payload_path(&c),"changed candidate").unwrap(),2=>fs::write(&f.oracle,"process.exit(0)").unwrap(),_=>fs::write(f.target.join("cases.json"),"[]").unwrap()};assert!(f.store.active().is_empty(),"case {case}");assert!(f.store.promote(&c.id,c.revision).is_err());}
}
#[test]
fn expired_receipt_or_candidate_and_changed_profile_stay_inactive(){
 for case in 0..3 {let f=fixture();let c=activate(&f,&draft(&f,GOOD));if case<2{f.store.mutate(|r|{let e=r.entries.iter_mut().find(|e|e.id==c.id).unwrap();if case==0{e.expires_at="2000-01-01T00:00:00Z".into();}else{e.validation.as_mut().unwrap().expires_at="2000-01-01T00:00:00Z".into();}Ok(())}).unwrap();}else{let mut raw=fs::read(f.store.profile_path()).unwrap();raw.push(b' ');fs::write(f.store.profile_path(),raw).unwrap();}assert!(f.store.active().is_empty());}
}
#[test]
fn stale_revision_cannot_promote_or_revoke_another_version(){let f=fixture();let c=draft(&f,GOOD);let checked=f.store.validate(&c.id,1,"normalization").unwrap();assert_eq!(f.store.promote(&c.id,1).unwrap_err(),"candidate_revision_conflict");assert_eq!(f.store.revoke(&c.id,1).unwrap_err(),"candidate_revision_conflict");let next=draft(&f,"module.exports = x=>x;");assert_ne!(checked.id,next.id);assert_eq!(checked.state,CandidateState::Candidate);}
#[test]
fn concurrent_revoke_wins_over_a_running_validation(){
 let f=fixture();let c=draft(&f,GOOD);let original=fs::read_to_string(&f.oracle).unwrap();fs::write(&f.oracle,format!("const s=require('node:fs');s.writeFileSync(process.argv[4]+'/started','1');Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,750);{original}")).unwrap();profiles(&f,5);
 let store=f.store.clone();let id=c.id.clone();let thread=std::thread::spawn(move||store.validate(&id,1,"normalization"));
 let deadline=Instant::now()+Duration::from_secs(5);while Instant::now()<deadline {let started=fs::read_dir(f.store.output_root()).ok().is_some_and(|v|v.filter_map(Result::ok).any(|e|e.path().join("started").exists()));if started{break;}std::thread::sleep(Duration::from_millis(20));}
 f.store.revoke(&c.id,1).unwrap();assert_eq!(thread.join().unwrap().unwrap_err(),"candidate_revision_conflict");assert_eq!(f.store.get(&c.id).unwrap().state,CandidateState::Revoked);
}
#[test]
fn timed_out_oracle_is_reaped_and_cannot_activate(){let f=fixture();let c=draft(&f,GOOD);fs::write(&f.oracle,"setTimeout(()=>{},10000)").unwrap();profiles(&f,1);let checked=f.store.validate(&c.id,1,"normalization").unwrap();assert!(checked.last_result.starts_with("holdout_timed_out"));assert!(f.store.promote(&c.id,checked.revision).is_err());}
#[test]
fn automatic_memory_requires_governance_and_native_tombstones_remain_effective(){
 let f=fixture();let record:SupermemoryRecord=serde_json::from_value(json!({"id":"lesson-test","name":"normalize experience","summary":markdown(GOOD),"sourceLabel":"PostMortemReflexion","nodeType":"reflexion"})).unwrap();
 let (_,path)=project_memory::managed_scope(&f.cfg,&f.source);let store=crate::services::supermemory_store::SupermemoryStore::from_path(path);let mut context=project_memory::new_context(&f.source,&f.source,None,None).unwrap();context.source="automatic_reflection".into();context.kind="experience_candidate".into();store.write_context_memory(record.clone(),context).unwrap();
 let c=f.store.propose_memory(&record).unwrap();assert!(project_memory::recall(&f.cfg,&f.source,Some("normalize"),10).hits.is_empty());activate(&f,&c);assert_eq!(project_memory::recall(&f.cfg,&f.source,Some("normalize"),10).hits.len(),1);
 use crate::domains::agent_cognitive::executor::ToolExecutor;
 let hermes=super::super::tools::hermes::HermesTool::new(super::super::evolution_memory::EvolutionMemory::for_project(&f.cfg,&f.source));
 let queried=hermes.execute(json!({"Action":"query_lessons","Query":"normalize"})).unwrap();assert_eq!(queried["matched_count"],1);assert_eq!(queried["lessons"][0]["id"],c.id);
 let mut changed=record.clone();changed.summary="changed underlying record".into();let mut automatic=project_memory::new_context(&f.source,&f.source,None,None).unwrap();automatic.source="automatic_reflection".into();automatic.kind="experience_candidate".into();
 store.write_context_memory(changed,automatic.clone()).unwrap();assert!(f.store.active().is_empty());assert!(project_memory::recall(&f.cfg,&f.source,Some("normalize"),10).hits.is_empty());
 store.write_context_memory(record.clone(),automatic).unwrap();assert_eq!(f.store.active().len(),1);
 let explicit=project_memory::new_context(&f.source,&f.source,None,None).unwrap();store.write_context_memory(record.clone(),explicit).unwrap();assert!(f.store.active().is_empty(),"an explicit overwrite must not resurrect governed history");
 store.revoke_context_memory(&c.id).unwrap();assert!(project_memory::recall(&f.cfg,&f.source,Some("normalize"),10).hits.is_empty());
}
#[test]
fn source_bound_explicit_notes_keep_expiry_change_and_revoke_guards(){
 let f=fixture();let record:SupermemoryRecord=serde_json::from_value(json!({"id":"explicit","name":"explicit marker","summary":"explicit marker"})).unwrap();let (_,path)=project_memory::managed_scope(&f.cfg,&f.source);let store=crate::services::supermemory_store::SupermemoryStore::from_path(path);
 let context=project_memory::new_context(&f.source,&f.source,Some("training.json"),None).unwrap();store.write_context_memory(record.clone(),context).unwrap();assert_eq!(project_memory::recall(&f.cfg,&f.source,Some("marker"),10).hits.len(),1);fs::write(f.source.join("training.json"),"new source").unwrap();assert!(project_memory::recall(&f.cfg,&f.source,Some("marker"),10).hits.is_empty());
 let mut context=project_memory::new_context(&f.source,&f.source,None,None).unwrap();context.expires_at=Some("2000-01-01T00:00:00Z".into());assert!(store.write_context_memory(record.clone(),context).is_err());
 store.revoke_context_memory("explicit").unwrap();assert!(project_memory::recall(&f.cfg,&f.source,Some("marker"),10).hits.is_empty());
}
#[test]
fn forged_scope_sensitive_payload_and_legacy_autoskills_do_not_activate(){
 let f=fixture();assert!(f.store.propose_skill("secret","draft","sk-candidate-canary-12345678901234567890").is_err());assert!(!f.source.join(".agents").exists());
 let c=draft(&f,GOOD);f.store.mutate(|r|{r.entries[0].source_project="different-project".into();r.entries[0].state=CandidateState::Active;Ok(())}).unwrap();assert!(f.store.active().is_empty());
 let old=f.source.join(".agents/skills/old/SKILL.md");fs::create_dir_all(old.parent().unwrap()).unwrap();fs::write(&old,"---\nname: old\n---\n> 💎 **实战结晶资产**：old auto output").unwrap();let found=crate::domains::agent_cognitive::capability_scout::CapabilityScout::discover_skills_with_config(&f.source,&f.cfg);assert!(!found.iter().any(|s|s.name=="old"||s.name==c.name));assert!(old.exists());
}

#[test]
fn legacy_tool_generated_skills_and_checkpoints_are_inspectable_but_not_auto_loaded(){
 let f=fixture();let old=f.source.join(".agents/skills/old-tool/SKILL.md");let user=f.source.join(".agents/skills/user-kept/SKILL.md");
 for (p,name) in [(&old,"old-tool"),(&user,"user-kept")] {fs::create_dir_all(p.parent().unwrap()).unwrap();fs::write(p,format!("---\nname: {name}\ndescription: legacy example\n---\nlegacy body")).unwrap();}
 let store=crate::services::supermemory_store::SupermemoryStore::new(&f.cfg,Some(&f.source.to_string_lossy()));let mut data=store.load_result().unwrap();
 for raw in [json!({"id":"skill:old-tool","name":"legacy tool skill","summary":"legacy body","filePath":".agents/skills/old-tool/SKILL.md","nodeType":"skill","tags":["superagent"]}),json!({"id":"checkpoint-old","name":"legacy checkpoint","summary":"legacy checkpoint","sourceLabel":"checkpoint","nodeType":"synthesis"}),json!({"id":"explicit-old","name":"legacy explicit note","summary":"legacy explicit note","sourceLabel":"fixture"})] {data.memories.push(serde_json::from_value(raw).unwrap());}
 store.save_result(&data).unwrap();
 let found=crate::domains::agent_cognitive::capability_scout::CapabilityScout::discover_skills_with_config(&f.source,&f.cfg);
 assert!(!found.iter().any(|s|s.name=="old-tool"));assert!(found.iter().any(|s|s.name=="user-kept"));
 let recall=project_memory::recall(&f.cfg,&f.source,Some("legacy"),10);assert_eq!(recall.hits.len(),1);assert_eq!(recall.hits[0].record.id,"explicit-old");assert!(old.exists());
}
#[test]
fn candidate_inspection_is_read_only_but_save_and_revoke_remain_mutations(){
 use super::super::tool_policy::{ToolPolicy,ToolPolicyMode,ToolPolicyDecision};let policy=ToolPolicy::new(ToolPolicyMode::ReadOnly);
 for action in ["list","read","read_candidate"] {assert_eq!(policy.evaluate("skill_tool",&json!({"action":action})).1,ToolPolicyDecision::Allow);}
 assert_eq!(policy.evaluate("hermes_workflow",&json!({"Action":"query_lessons"})).1,ToolPolicyDecision::Allow);
 assert_eq!(policy.evaluate("hermes_workflow",&json!({"Action":"record_lesson"})).1,ToolPolicyDecision::Deny);
 for action in ["save","update","delete"] {assert_eq!(policy.evaluate("skill_tool",&json!({"action":action})).1,ToolPolicyDecision::Deny);}
}

#[test]
fn a_failed_new_version_never_overwrites_the_existing_active_skill(){
 let f=fixture();let old=activate(&f,&draft(&f,GOOD));let bytes=fs::read(f.store.payload_path(&old)).unwrap();
 let new=draft(&f,"module.exports = input => input;");let failed=f.store.validate(&new.id,1,"normalization").unwrap();assert_eq!(failed.last_result,"holdout_check_failed");
 let active=f.store.active();assert_eq!(active.len(),1);assert_eq!(active[0].id,old.id);assert_eq!(bytes,fs::read(f.store.payload_path(&old)).unwrap());assert_ne!(old.id,new.id);
}
