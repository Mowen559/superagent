//! Project-scoped, immutable drafts. A trusted, pinned held-out oracle is
//! necessary for activation; counts, model claims and caller booleans are not.
use super::project_memory;
use crate::{config::runtime::RuntimeConfig,models::memory::SupermemoryRecord};
use serde::{Serialize,Deserialize};
use std::{fs,path::{Path,PathBuf},process::{Command,Stdio},sync::Mutex,time::{Instant,Duration}};
static WRITE_LOCK:Mutex<()>=Mutex::new(());
const MAX_PAYLOAD:u64=1024*1024;
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum CandidateState {Candidate,Active,Revoked}
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum CandidateKind {Skill,Memory}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Candidate {
 pub id:String,pub name:String,pub description:String,pub kind:CandidateKind,
 pub state:CandidateState,pub revision:u64,pub source_project:String,pub source_fingerprint:String,
 pub content_sha256:String,pub created_at:String,pub expires_at:String,
 pub validation:Option<ValidationReceipt>,pub last_result:String,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct ValidationReceipt {
 pub suite_id:String,pub profile_digest:String,pub candidate_digest:String,pub source_fingerprint:String,
 pub checked_at:String,pub expires_at:String,pub cases:Vec<CaseReceipt>,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct CaseReceipt {
 pub id:String,pub target_path:PathBuf,pub target_fingerprint:String,pub oracle_path:PathBuf,
 pub oracle_sha256:String,pub exit_code:i32,pub duration_ms:u64,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct HoldoutProfiles {pub schema_version:u8,pub suites:Vec<HoldoutSuite>}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct HoldoutSuite {pub id:String,pub timeout_seconds:u64,pub cases:Vec<HoldoutCase>}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct HoldoutCase {pub id:String,pub target_path:PathBuf,pub target_fingerprint:String,pub oracle_path:PathBuf,pub oracle_sha256:String}
#[derive(Default,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Registry {version:u8,source_project:String,entries:Vec<Candidate>}
#[derive(Clone)]
pub struct Governance {root:PathBuf,project:PathBuf,profiles:PathBuf}
fn hash(bytes:&[u8])->String {crate::shared_kernel::crypto::sha256_hex(bytes)}
fn bounded(path:&Path,limit:u64)->Result<Vec<u8>,String>{use std::io::Read;let f=fs::File::open(path).map_err(|_|"candidate_read_failed")?;if !f.metadata().map_err(|_|"candidate_read_failed")?.is_file(){return Err("candidate_not_file".into());}let mut bytes=vec![];f.take(limit+1).read_to_end(&mut bytes).map_err(|_|"candidate_read_failed")?;if bytes.len() as u64>limit{return Err("candidate_size_limit".into());}Ok(bytes)}
fn file_hash(path:&Path)->Result<String,String>{Ok(hash(&bounded(path,MAX_PAYLOAD)?))}
fn fingerprint(root:&Path)->Result<String,String>{crate::domains::version_control::change_set::workspace_fingerprint(root).map_err(|_|"source_fingerprint_unavailable".into())}
fn unexpired(at:&str)->bool{chrono::DateTime::parse_from_rfc3339(at).is_ok_and(|d|d>chrono::Utc::now())}
fn within(path:&Path,root:&Path)->bool{path.starts_with(root)}
fn valid_id(id:&str)->bool{!id.is_empty()&&id.len()<=200&&id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'||b==b'_')}
impl Governance {
 pub fn new(config:&RuntimeConfig,workspace:&Path)->Self {
  let (project,notes)=project_memory::managed_scope(config,workspace);
  let base=config.projects_root.parent().unwrap_or(&config.projects_root);
  Self{root:notes.with_file_name("candidate-governance"),project,profiles:base.join("evolution/holdout-profiles.json")}
 }
 pub fn root(&self)->&Path{&self.root}
 pub fn profile_path(&self)->&Path{&self.profiles}
 pub fn output_root(&self)->PathBuf{self.profiles.parent().unwrap_or(&self.root).join("validation-runs")}
 fn registry_path(&self)->PathBuf{self.root.join("governance.json")}
 pub fn payload_path(&self,c:&Candidate)->PathBuf{self.root.join("candidates").join(&c.id).join(if c.kind==CandidateKind::Skill{"SKILL.md"}else{"memory.json"})}
 fn payload_bytes(&self,c:&Candidate)->Result<Vec<u8>,String>{
  if !valid_id(&c.id){return Err("candidate_id_invalid".into());}
  let container=fs::canonicalize(self.root.join("candidates")).map_err(|_|"candidate_read_failed")?;
  let path=self.payload_path(c);let actual=fs::canonicalize(&path).map_err(|_|"candidate_read_failed")?;
  let expected=container.join(&c.id).join(path.file_name().ok_or("candidate_path_invalid")?);
  if actual!=expected{return Err("candidate_payload_boundary".into());}bounded(&actual,MAX_PAYLOAD)
 }
 fn load(&self)->Result<Registry,String>{
  let path=self.registry_path();if !path.exists(){return Ok(Registry{version:1,source_project:project_memory::project_identity(&self.project),entries:vec![]});}
  let data:Registry=serde_json::from_slice(&bounded(&path,8*MAX_PAYLOAD)?).map_err(|_|"candidate_registry_invalid")?;
  if data.version!=1||data.source_project!=project_memory::project_identity(&self.project){return Err("candidate_registry_scope_mismatch".into());}Ok(data)
 }
 fn save(&self,data:&Registry)->Result<(),String>{
  project_memory::reject_sensitive(data).map_err(|_|"sensitive_candidate")?;
  let path=self.registry_path();let temp=self.root.join(format!("{}.tmp",uuid::Uuid::new_v4()));let backup=path.with_extension("json.bak");
  fs::write(&temp,serde_json::to_vec_pretty(data).map_err(|_|"candidate_serialization_failed")?).map_err(|_|"candidate_write_failed")?;
  if path.exists(){if !path.is_file(){let _=fs::remove_file(&temp);return Err("candidate_registry_not_file".into());}let _=fs::remove_file(&backup);fs::rename(&path,&backup).map_err(|_|"candidate_write_failed")?;}
  if fs::rename(&temp,&path).is_err(){if backup.exists(){let _=fs::rename(&backup,&path);}let _=fs::remove_file(&temp);return Err("candidate_write_failed".into());}
  let _=fs::remove_file(backup);Ok(())
 }
 fn mutate<T>(&self,f:impl FnOnce(&mut Registry)->Result<T,String>)->Result<T,String>{
  let _g=WRITE_LOCK.lock().map_err(|_|"candidate_lock_failed")?;fs::create_dir_all(&self.root).map_err(|_|"candidate_write_failed")?;
  let file=fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(self.root.join("governance.lock")).map_err(|_|"candidate_lock_failed")?;
  let mut lock=fd_lock::RwLock::new(file);let _lock=lock.write().map_err(|_|"candidate_lock_failed")?;
  let mut data=self.load()?;let result=f(&mut data)?;self.save(&data)?;Ok(result)
 }
 pub fn list(&self)->Result<Vec<Candidate>,String>{Ok(self.load()?.entries)}
 pub fn get(&self,id:&str)->Result<Candidate,String>{if !valid_id(id){return Err("candidate_id_invalid".into());}self.list()?.into_iter().find(|c|c.id==id).ok_or("candidate_not_found".into())}
 fn propose(&self,id:String,name:&str,description:&str,kind:CandidateKind,bytes:Vec<u8>)->Result<Candidate,String>{
  if !valid_id(&id)||name.is_empty()||name.len()>256||description.len()>4000||bytes.len() as u64>MAX_PAYLOAD{return Err("candidate_payload_invalid".into());}
  project_memory::reject_sensitive(&(name,description,String::from_utf8_lossy(&bytes))).map_err(|_|"sensitive_candidate")?;
  let c=Candidate{id,name:name.into(),description:description.into(),kind,state:CandidateState::Candidate,revision:1,
   source_project:project_memory::project_identity(&self.project),source_fingerprint:fingerprint(&self.project)?,content_sha256:hash(&bytes),
   created_at:chrono::Utc::now().to_rfc3339(),expires_at:(chrono::Utc::now()+chrono::Duration::days(30)).to_rfc3339(),validation:None,last_result:"unvalidated_candidate".into()};
  self.mutate(|data|{
   if let Some(old)=data.entries.iter().find(|x|x.id==c.id){if old.content_sha256==c.content_sha256{return Ok(old.clone());}return Err("candidate_id_conflict".into());}
   let path=self.payload_path(&c);fs::create_dir_all(path.parent().ok_or("candidate_path_invalid")?).map_err(|_|"candidate_write_failed")?;
   fs::OpenOptions::new().write(true).create_new(true).open(&path).and_then(|mut f|{use std::io::Write;f.write_all(&bytes)}).map_err(|_|"candidate_write_failed")?;
   if file_hash(&path)?!=c.content_sha256{return Err("candidate_readback_mismatch".into());}data.entries.push(c.clone());Ok(c.clone())
  })
 }
 pub fn propose_skill(&self,name:&str,description:&str,markdown:&str)->Result<Candidate,String>{
  let id=format!("skill-{}",uuid::Uuid::new_v4());let safe_name=format!("learned-{}-{}",crate::domains::agent_cognitive::skill_crystallizer::AutoSkillCrystallizer::generate_skill_slug(name),&id[id.len()-8..]);
  self.propose(id,&safe_name,description,CandidateKind::Skill,markdown.as_bytes().to_vec())
 }
 pub fn propose_memory(&self,record:&SupermemoryRecord)->Result<Candidate,String>{self.propose(record.id.clone(),&record.name,"Unverified project experience candidate",CandidateKind::Memory,serde_json::to_vec(record).map_err(|_|"candidate_serialization_failed")?)}
 pub fn read_candidate(&self,id:&str)->Result<String,String>{let c=self.get(id)?;let bytes=self.payload_bytes(&c)?;if hash(&bytes)!=c.content_sha256{return Err("candidate_content_changed".into());}String::from_utf8(bytes).map_err(|_|"candidate_encoding_invalid".into())}
 fn current(&self,c:&Candidate)->Result<(),String>{
  if c.source_project!=project_memory::project_identity(&self.project)||!unexpired(&c.expires_at){return Err("candidate_scope_or_expiry".into());}
  if fingerprint(&self.project)?!=c.source_fingerprint{return Err("candidate_source_changed".into());}
  let payload=self.payload_bytes(c)?;
  if hash(&payload)!=c.content_sha256{return Err("candidate_content_changed".into());}
  if c.kind==CandidateKind::Memory {
   let record:SupermemoryRecord=serde_json::from_slice(&payload).map_err(|_|"candidate_memory_invalid")?;
   let notes=crate::services::supermemory_store::SupermemoryStore::from_path(self.root.with_file_name("supermemory.json")).load_result().map_err(|_|"candidate_memory_store_unavailable")?;
   if record.id!=c.id || !notes.memories.iter().any(|r|r.id==c.id&&serde_json::to_value(r).ok()==serde_json::to_value(&record).ok()){return Err("candidate_memory_record_changed_or_missing".into());}
   let mut context=notes.memory_context.get(&c.id).cloned().ok_or("candidate_memory_context_missing")?;
   if !context.source.starts_with("automatic_"){return Err("candidate_memory_source_changed".into());}
   context.source="governed_experience".into();
   if !project_memory::context_eligible(Some(&context),Some((&self.project,&self.project))){return Err("candidate_memory_withdrawn_or_stale".into());}
  }
  Ok(())
 }
 fn profile(&self,suite_id:&str)->Result<(HoldoutSuite,String),String>{
  let raw=bounded(&self.profiles,MAX_PAYLOAD).map_err(|_|"holdout_profile_missing")?;
  let profiles:HoldoutProfiles=serde_json::from_slice(&raw).map_err(|_|"holdout_profile_invalid")?;
  if profiles.schema_version!=1{return Err("holdout_profile_invalid".into());}
  let suite=profiles.suites.into_iter().find(|s|s.id==suite_id).ok_or("holdout_suite_missing")?;
  if suite.cases.is_empty()||suite.cases.len()>8||suite.timeout_seconds==0||suite.timeout_seconds>120{return Err("holdout_suite_invalid".into());}
  Ok((suite,hash(&raw)))
 }
 fn case_paths(&self,c:&Candidate,case:&HoldoutCase)->Result<(PathBuf,PathBuf),String>{
  let target=fs::canonicalize(&case.target_path).map_err(|_|"holdout_target_missing")?;
  let source=fs::canonicalize(&self.project).map_err(|_|"candidate_source_missing")?;
  let oracle=fs::canonicalize(&case.oracle_path).map_err(|_|"holdout_oracle_missing")?;
  let root=fs::canonicalize(&self.root).map_err(|_|"candidate_store_unavailable")?;
  if within(&target,&source)||within(&source,&target)||within(&target,&root)||within(&oracle,&source)||within(&oracle,&target)||within(&oracle,&root){return Err("holdout_not_independent".into());}
  let fp=fingerprint(&target)?;
  if fp==c.source_fingerprint||fp!=case.target_fingerprint{return Err("holdout_target_changed_or_reused".into());}
  if file_hash(&oracle)?!=case.oracle_sha256{return Err("holdout_oracle_changed".into());}Ok((target,oracle))
 }
 fn validation_current(&self,c:&Candidate)->Result<(),String>{
  self.current(c)?;let receipt=c.validation.as_ref().ok_or("holdout_pass_required")?;
  if !unexpired(&receipt.expires_at)||receipt.candidate_digest!=c.content_sha256||receipt.source_fingerprint!=c.source_fingerprint{return Err("holdout_receipt_stale".into());}
  let (suite,digest)=self.profile(&receipt.suite_id)?;
  if digest!=receipt.profile_digest||suite.cases.len()!=receipt.cases.len(){return Err("holdout_profile_changed".into());}
  for (case,pass) in suite.cases.iter().zip(&receipt.cases){let (target,oracle)=self.case_paths(c,case)?;
   if case.id!=pass.id||target!=pass.target_path||oracle!=pass.oracle_path||case.target_fingerprint!=pass.target_fingerprint||case.oracle_sha256!=pass.oracle_sha256||pass.exit_code!=0{return Err("holdout_receipt_mismatch".into());}}
  Ok(())
 }
 pub fn active(&self)->Vec<Candidate>{self.list().unwrap_or_default().into_iter().filter(|c|c.state==CandidateState::Active&&self.validation_current(c).is_ok()).collect()}
 pub fn active_memories(&self)->Vec<SupermemoryRecord>{self.active().into_iter().filter(|c|c.kind==CandidateKind::Memory).filter_map(|c|{let r:SupermemoryRecord=serde_json::from_slice(&self.payload_bytes(&c).ok()?).ok()?;(r.id==c.id).then_some(r)}).collect()}
 pub fn promote(&self,id:&str,revision:u64)->Result<Candidate,String>{
  let observed=self.get(id)?;if observed.state==CandidateState::Revoked{return Err("candidate_revoked".into());}self.validation_current(&observed)?;
  self.mutate(|data|{let c=data.entries.iter_mut().find(|c|c.id==id).ok_or("candidate_not_found")?;
   if c.revision!=revision{return Err("candidate_revision_conflict".into());}if c.state==CandidateState::Revoked{return Err("candidate_revoked".into());}
   self.validation_current(c)?;c.state=CandidateState::Active;c.revision+=1;c.last_result="activated_after_heldout_pass".into();Ok(c.clone())})
 }
 pub fn revoke(&self,id:&str,revision:u64)->Result<Candidate,String>{self.mutate(|data|{let c=data.entries.iter_mut().find(|c|c.id==id).ok_or("candidate_not_found")?;if c.revision!=revision{return Err("candidate_revision_conflict".into());}c.state=CandidateState::Revoked;c.revision+=1;c.last_result="revoked".into();Ok(c.clone())})}
 pub fn validate(&self,id:&str,revision:u64,suite_id:&str)->Result<Candidate,String>{
  let c=self.get(id)?;if c.revision!=revision{return Err("candidate_revision_conflict".into());}if c.state==CandidateState::Revoked{return Err("candidate_revoked".into());}self.current(&c)?;
  let (suite,profile_digest)=self.profile(suite_id)?;let mut cases=vec![];let mut failure=None;
  for case in &suite.cases {
   let (target,oracle)=self.case_paths(&c,case)?;let output=self.output_root().join(uuid::Uuid::new_v4().to_string());fs::create_dir_all(&output).map_err(|_|"holdout_output_failed")?;
   let clean=crate::infrastructure::fs_path::canonicalize_clean;
   let mut command=Command::new("node");command.arg(clean(&oracle)).arg(clean(&self.payload_path(&c))).arg(clean(&target)).arg(clean(&output)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).current_dir(clean(&output));
   #[cfg(windows)]{use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
   let start=Instant::now();let mut child=command.spawn().map_err(|e|format!("holdout_launch_failed_{:?}_{:?}",e.kind(),e.raw_os_error()))?;
   let exit=loop{match child.try_wait(){Ok(Some(s))=>break s.code().unwrap_or(-1),Ok(None) if start.elapsed()<Duration::from_secs(suite.timeout_seconds)=>std::thread::sleep(Duration::from_millis(25)),_=>{let _=child.kill();let _=child.wait();failure=Some("holdout_timed_out_or_wait_failed");break -1;}}};
   self.current(&c)?;self.case_paths(&c,case)?;
   cases.push(CaseReceipt{id:case.id.clone(),target_path:target,target_fingerprint:case.target_fingerprint.clone(),oracle_path:oracle,oracle_sha256:case.oracle_sha256.clone(),exit_code:exit,duration_ms:start.elapsed().as_millis() as u64});
   if exit!=0{failure.get_or_insert("holdout_check_failed");break;}
  }
  let checked_at=chrono::Utc::now().to_rfc3339();
  self.mutate(|data|{let entry=data.entries.iter_mut().find(|e|e.id==id).ok_or("candidate_not_found")?;
   if entry.revision!=revision||entry.state==CandidateState::Revoked{return Err("candidate_revision_conflict".into());}
   entry.revision+=1;entry.state=CandidateState::Candidate;
   if let Some(reason)=failure {entry.validation=None;entry.last_result=reason.into();}else{
    let (_,now_profile)=self.profile(suite_id)?;if now_profile!=profile_digest{return Err("holdout_profile_changed".into());}
    entry.validation=Some(ValidationReceipt{suite_id:suite_id.into(),profile_digest,candidate_digest:c.content_sha256.clone(),source_fingerprint:c.source_fingerprint.clone(),checked_at,expires_at:(chrono::Utc::now()+chrono::Duration::hours(24)).to_rfc3339(),cases});entry.last_result="heldout_profile_passed_not_general_certification".into();
   }Ok(entry.clone())
  })
 }
}
#[cfg(test)]
#[path="candidate_governance_tests.rs"]
mod tests;
