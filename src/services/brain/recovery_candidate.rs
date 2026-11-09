//! Conservative, shared extraction of observed recovery sequences. Never infers
//! a root cause or a general repair from model prose or tool output strings.
use super::events::{RunOutcome, SessionEvent};
use super::tool_execution_gateway::{canonicalize_tool_name, normalize_tool_arguments};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const FIELD: &str = "_execution_evidence";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionEvidence {
    pub schema_version: u8,
    pub session_id: String,
    pub run_id: String,
    pub call_id: String,
    pub tool: String,
    pub action_digest: String,
    pub before_fingerprint: String,
    pub after_fingerprint: String,
    pub status: String,
    pub check_eligible: bool,
    pub verification: bool,
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub file_revision: Option<FileRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileRevision {
    pub path_digest: String,
    pub before_sha256: Option<String>,
    pub after_sha256: String,
}

/// Only small, in-workspace target files can attest an actual edit. Paths and
/// file contents never enter the persisted candidate.
pub(crate) fn file_before(root:&std::path::Path,args:&Value)->Option<(std::path::PathBuf,String,Option<String>)> {
    let target=args.get("TargetFile").or_else(||args.get("path")).and_then(Value::as_str)?;
    let path=std::path::Path::new(target);
    let path=if path.is_absolute(){path.to_path_buf()}else{root.join(path)};
    let path=if path.exists(){std::fs::canonicalize(&path).ok()?}else{std::fs::canonicalize(path.parent()?).ok()?.join(path.file_name()?)};
    let relative=path.strip_prefix(root).ok()?;
    let digest=crate::shared_kernel::crypto::sha256_hex(relative.to_string_lossy().as_bytes());
    let before=if path.exists(){Some(file_hash(&path)?)}else{None};
    Some((path,digest,before))
}
pub(crate) fn file_hash(path:&std::path::Path)->Option<String> {
    use std::io::Read;
    let file=std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file(){return None;}
    let mut bytes=Vec::new();file.take(16*1024*1024+1).read_to_end(&mut bytes).ok()?;
    if bytes.len()>16*1024*1024{return None;}
    Some(crate::shared_kernel::crypto::sha256_hex(&bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCandidate {
    pub state: String,
    pub failure_reason: String,
    pub failure: ExecutionEvidence,
    pub revisions: Vec<ExecutionEvidence>,
    pub validation: ExecutionEvidence,
    pub applicability: Vec<String>,
    pub unverified: Vec<String>,
}
impl RecoveryCandidate {
    pub fn summary(&self) -> String {
        format!("Observed candidate only: check {} exited nonzero; {} physical file edit(s); the same check exited 0 on fingerprint {}. Root cause, causal fix, checker integrity and transfer remain unverified.", self.failure.action_digest, self.revisions.len(), self.validation.after_fingerprint)
    }
}

pub fn observes(tool: &str) -> bool {
    matches!(tool, "run_command" | "verify_task_tool" | "write_to_file" | "replace_in_file" | "multi_replace_file_content" | "collect_subagent_changes")
}
fn file_edit(tool: &str) -> bool {
    matches!(tool, "write_to_file" | "replace_in_file" | "multi_replace_file_content")
}
fn metadata_or_read(tool: &str) -> bool {
    matches!(tool, "view_file" | "view_outline" | "list_dir" | "find_by_name" | "grep_search" | "ast_grep" | "search_graph" | "query_graph" | "trace_path" | "get_code_snippet" | "get_architecture" | "manage_task" | "update_task_progress" | "super_memory" | "check_worker_status")
}
pub fn action_digest(name: &str, arguments: &Value) -> String {
    let canonical = canonicalize_tool_name(name);
    let tool = if canonical.is_empty() { name } else { canonical };
    let args = normalize_tool_arguments(tool, arguments);
    crate::shared_kernel::crypto::sha256_hex(serde_json::to_vec(&(tool, args)).unwrap_or_default().as_slice())
}
fn hash(value: &str) -> bool { value.len()==64 && value.bytes().all(|b|b.is_ascii_hexdigit()) }

pub fn is_current(candidate:&RecoveryCandidate,root:&std::path::Path)->bool {
    crate::domains::version_control::change_set::workspace_fingerprint(root).ok().as_ref()==Some(&candidate.validation.after_fingerprint)
}

/// Only the latest Run is eligible. The live executor calls before RunFinished;
/// postmortem reflection requires an explicit completed terminal event.
pub fn extract(events: &[SessionEvent], allow_open_run: bool) -> Vec<RecoveryCandidate> {
    extract_raw(events,allow_open_run).into_iter().filter(|c|super::project_memory::reject_sensitive(c).is_ok()).collect()
}

pub(crate) fn extract_raw(events: &[SessionEvent], allow_open_run: bool) -> Vec<RecoveryCandidate> {
    let mut run = None;
    let mut session = None;
    let mut pending = HashMap::new();
    let mut seen = HashSet::new();
    let mut chain: Option<(ExecutionEvidence, Vec<ExecutionEvidence>, String)> = None;
    let mut candidate: Option<RecoveryCandidate> = None;
    let mut finished = false;
    let mut invalid = false;
    for event in events {
        match event {
            SessionEvent::RunStarted { run_id, .. } => {
                run=Some(run_id.clone()); session=None; pending.clear(); seen.clear();
                chain=None; candidate=None; finished=false; invalid=false;
            }
            SessionEvent::RunFinished { run_id, outcome, .. } => {
                if run.as_ref()!=Some(run_id) || *outcome!=RunOutcome::Completed || !pending.is_empty() { invalid=true; }
                finished=true;
            }
            SessionEvent::ToolCall { id, name, arguments } if run.is_some() => {
                if finished || !seen.insert(id.clone()) { invalid=true; }
                pending.insert(id.clone(), (name.clone(), action_digest(name, arguments)));
            }
            SessionEvent::ToolResult { id, result } if run.is_some() => {
                let Some((name,digest))=pending.remove(id) else { invalid=true; continue; };
                if finished { invalid=true; continue; }
                let canonical=canonicalize_tool_name(&name);
                let tool=if canonical.is_empty() {name.as_str()} else {canonical};
                let evidence=result.get(FIELD).cloned().and_then(|v|serde_json::from_value::<ExecutionEvidence>(v).ok());
                let Some(e)=evidence else {
                    if !metadata_or_read(tool) {chain=None; candidate=None;}
                    continue;
                };
                if e.schema_version!=1 || Some(&e.run_id)!=run.as_ref() || e.call_id!=*id || e.tool!=tool || e.action_digest!=digest
                    || e.session_id.is_empty() || session.as_ref().is_some_and(|s|s!=&e.session_id)
                    || !hash(&e.before_fingerprint) || !hash(&e.after_fingerprint) || !observes(tool) {
                    invalid=true; continue;
                }
                session=Some(e.session_id.clone());
                if chain.as_ref().is_some_and(|(_,_,current)|current!=&e.before_fingerprint) {chain=None; candidate=None;}
                let changed=e.before_fingerprint!=e.after_fingerprint;
                // Every later failure or content change withdraws an earlier success.
                if changed || e.status!="succeeded" {candidate=None;}
                if e.check_eligible && e.status=="failed" && e.exit_code.is_some_and(|code|code!=0) && !changed {
                    chain=Some((e.clone(),vec![],e.after_fingerprint.clone()));
                } else if file_edit(tool) && changed && e.status=="succeeded"
                    && e.file_revision.as_ref().is_some_and(|f|hash(&f.path_digest)&&hash(&f.after_sha256)&&f.before_sha256.as_ref()!=Some(&f.after_sha256)&&f.before_sha256.as_ref().is_none_or(|h|hash(h))) {
                    if let Some((_,edits,current))=chain.as_mut() {edits.push(e.clone());*current=e.after_fingerprint.clone();}
                } else if e.check_eligible && e.status=="succeeded" && e.verification && e.exit_code==Some(0) && !changed {
                    if let Some((failure,edits,current))=chain.as_ref() {
                        if !edits.is_empty() && failure.action_digest==e.action_digest && *current==e.before_fingerprint {
                            candidate=Some(RecoveryCandidate {
                                state:"candidate".into(), failure_reason:format!("process_exit_nonzero:{}",failure.exit_code.unwrap()),
                                failure:failure.clone(), revisions:edits.clone(), validation:e,
                                applicability:vec!["Same source project, check arguments and observed content revisions only".into(),"Foreground check; workspace scan obeys repository ignore rules and size limits".into()],
                                unverified:vec!["Semantic root cause and edit-to-fix causality".into(),"Checker integrity, assertion coverage and independent business acceptance".into(),"Untracked/excluded files, changes after last observation, different inputs and held-out transfer".into()],
                            });
                        }
                    }
                } else if changed || e.status!="succeeded" {chain=None;}
            }
            _=>{}
        }
    }
    if invalid || !pending.is_empty() || (!allow_open_run && !finished) {return vec![];}
    candidate.into_iter().collect()
}

#[cfg(test)]
#[path = "recovery_candidate_tests.rs"]
pub(crate) mod tests;
