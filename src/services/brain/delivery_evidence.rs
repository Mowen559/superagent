//! Run-local physical delivery facts. Declarations are never verification.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, fs, path::{Component, Path, PathBuf},
    process::{Command, Stdio}, time::{Duration, Instant}};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn required_default() -> bool { true }

fn same_target(expected: &str, actual: &str) -> bool {
    expected == actual || (Path::new(expected).is_absolute() && Path::new(actual).is_absolute()
        && fs::canonicalize(expected).ok().zip(fs::canonicalize(actual).ok())
            .is_some_and(|(expected, actual)| expected == actual))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryContract {
    pub version: u32,
    pub scope: DeliveryScope,
    pub target: String,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRequirement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryScope { ReadOnly, Implementation }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRequirement {
    pub id: String,
    pub path: String,
    #[serde(default = "required_default")]
    pub required: bool,
    #[serde(default)]
    pub validator: Option<String>,
}

impl DeliveryContract {
    /// Give the model actionable feedback for observed shape errors, without
    /// coercing its plan or changing the Run's host-owned declaration.
    pub fn validate_model_value(value: &Value) -> Result<(), String> {
        let mut issues = Vec::new();
        if let Some(version) = value.get("version") {
            if version.as_u64() != Some(1) {
                let received = if version.is_string() { "string" } else { "non-version-1 value" };
                issues.push(format!("delivery.version: expected integer 1 (unquoted), received {received}; use \"version\": 1"));
            }
        }
        if let Some(artifacts) = value.get("artifacts").and_then(Value::as_array) {
            for (index, artifact) in artifacts.iter().enumerate() {
                if let Some(validator) = artifact.get("validator") {
                    if !validator.is_null() && !validator.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 80) {
                        issues.push(format!("delivery.artifacts[{index}].validator: expected null, omission, or a registered non-empty validator ID (maximum 80 bytes); an empty string is invalid"));
                    }
                }
            }
        }
        if issues.is_empty() { Ok(()) } else {
            Err(json!({"kind":"invalid_task_plan","issues":issues,
                "recovery":"Correct all listed fields in one submission. Preserve the host delivery requirements; do not guess a validator or change the target."}).to_string())
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.target.trim().is_empty() || self.target.len() > 2000 || self.artifacts.len() > 12 {
            return Err("delivery requires version=1, a bounded target and at most 12 artifacts".into());
        }
        if self.scope == DeliveryScope::ReadOnly && !self.artifacts.is_empty() {
            return Err("read_only delivery cannot require implementation artifacts".into());
        }
        let mut ids = HashSet::new();
        for item in &self.artifacts {
            if item.id.is_empty() || item.id.len() > 64 || !ids.insert(&item.id)
                || item.path.is_empty() || item.path.len() > 500
                || !Path::new(&item.path).components().all(|c| matches!(c, Component::Normal(_)))
                || item.path.contains(['\\', ':'])
                || item.validator.as_ref().is_some_and(|s| s.is_empty() || s.len() > 80) {
                return Err("artifact IDs must be unique and paths must be workspace-relative without traversal".into());
            }
        }
        Ok(())
    }

    pub fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,"required":["version","scope","target","artifacts"],
          "properties":{"version":{"type":"integer","enum":[1],"description":"Use the integer 1, never the string \"1\"."},"scope":{"type":"string","enum":["read_only","implementation"]},
          "target":{"type":"string","minLength":1,"maxLength":2000},"artifacts":{"type":"array","maxItems":12,"items":{
            "type":"object","additionalProperties":false,"required":["id","path"],"properties":{
              "id":{"type":"string","maxLength":64},"path":{"type":"string","maxLength":500},
              "required":{"type":"boolean","default":true},"validator":{"type":["string","null"],"minLength":1,"maxLength":80,"description":"Use the exact host-registered validator ID, or omit/use null for a physical-only artifact. Never use an empty string."}}}}}})
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierProfile {
    pub id: String,
    pub program: PathBuf,
    pub program_sha256: String,
    pub script: PathBuf,
    pub script_sha256: String,
    pub args: Vec<String>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostProfile {
    delivery: DeliveryContract,
    validators: Vec<VerifierProfile>,
    #[serde(default)]
    actions: Vec<DeliveryAction>,
    #[serde(default)]
    completion_policy: CompletionPolicy,
    #[serde(default)]
    allow_internal_diagnostics: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all="snake_case")]
enum CompletionPolicy { #[default] QualityChecks, PhysicalArtifacts }

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeliveryAction {
    id: String, source: String, executor: String,
    #[serde(default)]
    receipt_artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactRecord {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub target: String,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub source: String,
    pub observed_at: String,
    pub validator: Option<String>,
    pub verifier_sha256: Option<String>,
    pub domain_verified: bool,
    pub checks: Value,
}

#[derive(Debug)]
pub struct DeliveryEvidence {
    root: PathBuf,
    session_id: String,
    run_id: String,
    contract: Option<DeliveryContract>,
    imposed: Option<DeliveryContract>,
    validators: HashMap<String, VerifierProfile>,
    profile_file: Option<(PathBuf, String)>,
    records: HashMap<String, ArtifactRecord>,
    errors: HashMap<String, String>,
    actions: HashMap<String, DeliveryAction>,
    attempted_actions: HashSet<String>,
    issued_receipts: HashMap<String, ArtifactRecord>,
    receipt_protection_error: Option<String>,
    declaration_error: Option<String>,
    completion_policy: CompletionPolicy,
    allow_internal_diagnostics: bool,
}

pub fn file_sha256(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = fs::File::open(path).map_err(|_| format!("cannot read {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let count = file.read(&mut block).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        hash.update(&block[..count]);
    }
    Ok(hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

impl DeliveryEvidence {
    pub fn new(root: &Path, session_id: &str, run_id: &str) -> Result<Self, String> {
        Ok(Self { root: fs::canonicalize(root).map_err(|e| e.to_string())?, session_id: session_id.into(), run_id: run_id.into(),
            contract: None, imposed: None, validators: HashMap::new(), profile_file: None,
            records: HashMap::new(), errors: HashMap::new(), declaration_error: None,
            actions: HashMap::new(), attempted_actions: HashSet::new(),
            issued_receipts: HashMap::new(), receipt_protection_error: None,
            completion_policy:CompletionPolicy::default(), allow_internal_diagnostics:false })
    }

    pub fn load_profile(&mut self, path: &Path) -> Result<(), String> {
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        if path.starts_with(&self.root) || fs::metadata(&path).map_err(|e| e.to_string())?.len() > 64_000 {
            return Err("host delivery profile must be bounded and outside the execution workspace".into());
        }
        let profile_hash = file_sha256(&path)?;
        let profile: HostProfile = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        profile.delivery.validate()?;
        if profile.completion_policy==CompletionPolicy::PhysicalArtifacts &&
            (!profile.delivery.artifacts.iter().any(|a|a.required) || profile.delivery.artifacts.iter().any(|a|a.validator.is_some()) || !profile.actions.is_empty()) {
            return Err("physical_artifacts requires nonempty physical delivery without business validators or actions".into());
        }
        let mut validators = HashMap::new();
        for verifier in profile.validators {
            self.check_verifier(&verifier)?;
            if validators.insert(verifier.id.clone(), verifier).is_some() { return Err("duplicate verifier ID".into()); }
        }
        for artifact in &profile.delivery.artifacts {
            if artifact.validator.as_ref().is_some_and(|id| !validators.contains_key(id)) { return Err("missing host verifier".into()); }
        }
        let mut actions = HashMap::new();
        let mut receipt_owners = HashSet::new();
        for action in profile.actions {
            if action.receipt_artifacts.len() > 12 || action.receipt_artifacts.iter().any(|id|
                id == &action.source || !receipt_owners.insert(id.clone())
                    || !profile.delivery.artifacts.iter().any(|a| &a.id == id && a.required)) {
                return Err("action receipt_artifacts must uniquely reference required output IDs, never its source".into());
            }
            if action.id.is_empty() || action.id.len()>64 || !validators.contains_key(&action.executor)
                || profile.delivery.artifacts.iter().any(|a|a.validator.as_ref()==Some(&action.executor))
                || !profile.delivery.artifacts.iter().any(|a| a.id==action.source && a.required && a.validator.is_some() && a.validator.as_ref()!=Some(&action.executor))
                || actions.insert(action.id.clone(), action).is_some() {
                return Err("managed action requires a unique ID, pinned executor and required business-verified source".into());
            }
        }
        self.actions = actions;
        self.contract = Some(profile.delivery.clone());
        self.imposed = Some(profile.delivery);
        self.validators = validators;
        self.profile_file = Some((path, profile_hash));
        self.completion_policy=profile.completion_policy;
        self.allow_internal_diagnostics=profile.allow_internal_diagnostics;
        Ok(())
    }

    pub fn host_physical_delivery_complete(&self)->bool {
        self.profile_file.is_some() && self.completion_policy==CompletionPolicy::PhysicalArtifacts && self.completion_gap().is_none()
    }
    pub fn uses_host_physical_acceptance(&self)->bool {
        self.profile_file.is_some() && self.completion_policy==CompletionPolicy::PhysicalArtifacts
    }

    pub fn internal_trace_guard(&self,name:&str,args:&Value)->Option<String> {
        if self.allow_internal_diagnostics {return None;}
        let normalize=|s:&str|s.replace('\\',"/").trim_start_matches("//?/").to_ascii_lowercase();
        let internal=format!("{}/.superagent/traces",normalize(&self.root.to_string_lossy()));
        let direct=["AbsolutePath","path","file_path","DirectoryPath","SearchPath","SearchDirectory","directory","dir"];
        let file_tool=matches!(name,"view_file"|"view_outline"|"list_dir"|"find_by_name"|"grep_search"|"write_to_file"|"replace_in_file");
        let denied=file_tool && direct.iter().filter_map(|k|args.get(*k).and_then(Value::as_str)).any(|p|{
            let path=Path::new(p);let full=if path.is_absolute(){path.to_path_buf()}else{self.root.join(path)};
            let real=fs::canonicalize(&full).unwrap_or(full);let value=normalize(&real.to_string_lossy());
            value==internal||value.starts_with(&(internal.clone()+"/"))
        });
        let shell=matches!(name,"run_command"|"verify_task_tool") && args.get("CommandLine").or_else(||args.get("command")).and_then(Value::as_str)
            .is_some_and(|c|{let text=normalize(c);text.contains(&internal)||text.split([' ','\t','\n','\r','\'','"','(',')',';']).any(|s|{
                let s=s.strip_prefix("./").unwrap_or(s);s==".superagent/traces"||s.starts_with(".superagent/traces/")
            })});
        (denied||shell).then(||"Internal live traces are not task evidence. Use read_tool_result for current-Run full outputs, inspect actual business files or read Skill resources; finish missing required artifacts instead of rereading internal prompts.".into())
    }

    fn check_verifier(&self, verifier: &VerifierProfile) -> Result<(), String> {
        if !(1..=150).contains(&verifier.timeout_seconds) || verifier.args.len() > 24 { return Err("invalid verifier budget".into()); }
        for (path, expected) in [(&verifier.program, &verifier.program_sha256), (&verifier.script, &verifier.script_sha256)] {
            if !path.is_absolute() { return Err("verifier paths must be absolute".into()); }
            let actual = fs::canonicalize(path).map_err(|e| e.to_string())?;
            if actual.starts_with(&self.root) || &file_sha256(&actual)? != expected { return Err("verifier is mutable or its fingerprint changed".into()); }
        }
        Ok(())
    }

    pub fn validate_declaration(&self, declaration: Option<&DeliveryContract>) -> Result<(), String> {
        let Some(declaration) = declaration else { return Ok(()); };
        declaration.validate().and_then(|()| {
            if declaration.artifacts.iter().any(|a|self.actions.values().any(|action|a.validator.as_ref()==Some(&action.executor))) {
                return Err("managed action executor cannot be used as an artifact validator".into());
            }
            if let Some(imposed) = &self.imposed {
                let prefix = "plan cannot remove or weaken host delivery requirements";
                if !same_target(&imposed.target, &declaration.target) {
                    return Err(format!("{prefix}: delivery.target mismatch; expected {:?}, received {:?}. Copy the host target exactly without adding path escapes.", imposed.target, declaration.target));
                }
                if declaration.scope != imposed.scope {
                    return Err(format!("{prefix}: delivery.scope mismatch; expected {:?}, received {:?}", imposed.scope, declaration.scope));
                }
                if let Some(item) = imposed.artifacts.iter().find(|item| !declaration.artifacts.contains(item)) {
                    return Err(format!("{prefix}: artifact {:?} must retain path {:?}, required={}, validator={:?}; received {:?}",
                        item.id, item.path, item.required, item.validator, declaration.artifacts.iter().find(|actual| actual.id == item.id)));
                }
            }
            if let Some(previous) = &self.contract {
                if previous.artifacts.iter().filter(|a| a.required).any(|item| !declaration.artifacts.contains(item)) {
                    return Err("replacement plan cannot remove required delivery artifacts".into());
                }
            }
            Ok(())
        })
    }

    pub fn normalize_declaration(&self, declaration: &DeliveryContract) -> Result<DeliveryContract, String> {
        self.validate_declaration(Some(declaration))?;
        let mut normalized = declaration.clone();
        if let Some(imposed) = &self.imposed {
            normalized.target = imposed.target.clone();
        }
        Ok(normalized)
    }

    pub fn declare(&mut self, declaration: Option<&DeliveryContract>) {
        let Some(declaration) = declaration else { return; };
        let valid = self.normalize_declaration(declaration);
        match valid {
            Ok(normalized) => { self.contract = Some(normalized); self.declaration_error = None; }
            Err(error) => self.declaration_error = Some(error),
        }
    }

    fn inspect(&self, item: &ArtifactRequirement) -> Result<ArtifactRecord, String> {
        let contract = self.contract.as_ref().ok_or("no delivery contract")?;
        let path = fs::canonicalize(self.root.join(&item.path)).map_err(|_| format!("missing artifact {}", item.path))?;
        if !path.starts_with(&self.root) || !path.is_file() { return Err("artifact escapes workspace or is not a file".into()); }
        Ok(ArtifactRecord { id: item.id.clone(), session_id: self.session_id.clone(), run_id: self.run_id.clone(),
            target: contract.target.clone(), sha256: file_sha256(&path)?, bytes: fs::metadata(&path).map_err(|e| e.to_string())?.len(), path,
            source: "runtime_file_observation; authorship requires matching tool provenance".into(),
            observed_at: chrono::Utc::now().to_rfc3339(), validator: item.validator.clone(), verifier_sha256: None,
            domain_verified: false, checks: json!([]) })
    }

    fn run_verifier(&self, item: &ArtifactRequirement, record: &mut ArtifactRecord) -> Result<Value, String> {
        let Some(id) = &item.validator else { return Ok(Value::Null); };
        let verifier = self.validators.get(id).ok_or("declared verifier is not registered by the host")?;
        self.check_verifier(verifier)?;
        if let Some((path, expected)) = &self.profile_file {
            if &file_sha256(path)? != expected { return Err("host delivery profile changed".into()); }
        }
        let log = tempfile::tempfile().map_err(|e| e.to_string())?;
        let mut command = Command::new(&verifier.program);
        command.arg(&verifier.script).args(verifier.args.iter().map(|arg| arg.replace("{workspace}", &self.root.to_string_lossy())
            .replace("{artifact}", &record.path.to_string_lossy()).replace("{target}", &record.target).replace("{sha256}", &record.sha256)))
            .current_dir(&self.root).stdin(Stdio::null()).stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(Stdio::null());
        #[cfg(windows)] {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? { break status; }
            if started.elapsed() >= Duration::from_secs(verifier.timeout_seconds) {
                #[cfg(windows)] { let _ = Command::new("taskkill").args(["/PID", &child.id().to_string(), "/T", "/F"]).creation_flags(0x08000000).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
                let _ = child.kill(); let _ = child.wait();
                return Err("trusted verifier timed out".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if log.metadata().map_err(|e| e.to_string())?.len() > 2_000_000 { return Err("verifier output exceeds budget".into()); }
        use std::io::{Read, Seek, SeekFrom};
        let mut log = log; log.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut text = String::new(); log.read_to_string(&mut text).map_err(|e| e.to_string())?;
        self.check_verifier(verifier)?;
        let value: Value = serde_json::from_str(text.lines().rev().find(|s| !s.trim().is_empty()).ok_or("missing verifier result")?).map_err(|_| "invalid verifier JSON")?;
        if !status.success() {
            let detail = value.get("error").and_then(Value::as_str).unwrap_or("no domain diagnostic");
            return Err(format!("trusted verifier failed: exit {:?}; {}", status.code(), detail.chars().take(600).collect::<String>()));
        }
        if value["passed"] != true || value["target"] != record.target || !value["checks"].as_array().is_some_and(|a| !a.is_empty()) {
            return Err("business verifier did not confirm this target and its checks".into());
        }
        self.check_verifier(verifier)?;
        if file_sha256(&record.path)? != record.sha256 { return Err("artifact changed during verification".into()); }
        record.domain_verified = true;
        record.verifier_sha256 = Some(verifier.script_sha256.clone());
        record.checks = value["checks"].clone();
        Ok(value)
    }

    pub fn verify(&mut self, ids: &[String]) -> Result<Value, String> {
        if let Some(error) = self.receipt_integrity_gap() { return Err(error); }
        let items = self.contract.as_ref().ok_or("no delivery declaration; call plan_task with delivery first")?.artifacts.clone();
        if items.iter().any(|a|self.actions.values().any(|action|a.validator.as_ref()==Some(&action.executor))) {return Err("managed action executor cannot be used as an artifact validator".into());}
        if ids.iter().any(|id| !items.iter().any(|i| &i.id == id)) { return Err("unknown artifact ID".into()); }
        for item in items.iter().filter(|i| ids.is_empty() || ids.contains(&i.id)) {
            self.records.remove(&item.id);
            match self.inspect(item).and_then(|mut record| { self.run_verifier(item, &mut record)?; Ok(record) }) {
                Ok(record) => { self.records.insert(item.id.clone(), record); self.errors.remove(&item.id); }
                Err(error) => { self.errors.insert(item.id.clone(), error); }
            }
        }
        if ids.is_empty() {
            if let Some(gap) = self.completion_gap() { return Err(gap); }
        } else {
            for id in ids { if let Some(error)=self.errors.get(id) { return Err(format!("delivery '{id}' needs verify_delivery: {error}")); } }
        }
        Ok(json!({"status":"success","exit_code":0,"domain_verified":self.records.values().any(|r| r.domain_verified),"completion_ready":self.completion_gap().is_none(),"delivery_evidence":self.snapshot()}))
    }

    pub fn perform_action(&mut self, id: &str) -> Result<Value,String> {
        let action=self.actions.get(id).cloned().ok_or("unknown host-managed action")?;
        if self.attempted_actions.contains(id) {return Err("managed action already attempted; preserve its ID and use read-only verification, never repeat creation".into());}
        let source=self.contract.as_ref().and_then(|c|c.artifacts.iter().find(|a|a.id==action.source)).cloned().ok_or("missing source requirement")?;
        let mut record=self.records.get(&source.id).cloned().ok_or("source needs local verify_delivery before any publication")?;
        let fresh=self.inspect(&source)?;
        if !record.domain_verified || fresh.sha256!=record.sha256 || fresh.path!=record.path || fresh.target!=record.target {
            return Err("source verification is missing or stale; repair locally and verify_delivery before publishing".into());
        }
        self.check_verifier(self.validators.get(source.validator.as_ref().ok_or("source has no business validator")?).ok_or("missing source verifier")?)?;
        // A timed-out process may already have committed. Never blindly retry it.
        self.attempted_actions.insert(id.into());
        let invocation=ArtifactRequirement {validator:Some(action.executor),..source};
        let outcome=self.run_verifier(&invocation,&mut record);
        // Even a failed/timed-out action may already have issued a public ID.
        for id in &action.receipt_artifacts {
            let requirement=self.contract.as_ref().and_then(|c|c.artifacts.iter().find(|a| &a.id == id)).cloned();
            let captured=requirement.as_ref().ok_or_else(|| "receipt requirement missing".to_string()).and_then(|a|self.inspect(a));
            match captured {
                Ok(receipt) => { self.issued_receipts.insert(id.clone(),receipt); }
                Err(_) => { self.receipt_protection_error=Some(format!("managed receipt '{id}' could not be captured after action; preserve existing outputs and report the failure, never fabricate or recreate it")); }
            }
        }
        let result=outcome?;
        if let Some(error)=&self.receipt_protection_error { return Err(error.clone()); }
        Ok(json!({"status":"success","exit_code":0,"action_id":id,"source_sha256":record.sha256,"result":result}))
    }

    fn receipt_integrity_gap(&self) -> Option<String> {
        if let Some(error)=&self.receipt_protection_error { return Some(error.clone()); }
        for (id, issued) in &self.issued_receipts {
            let current=fs::canonicalize(&issued.path).ok();
            if current.as_ref()!=Some(&issued.path) || file_sha256(&issued.path).ok().as_ref()!=Some(&issued.sha256) {
                return Some(format!("managed receipt '{id}' was removed or modified after issuance; original fingerprint remains authoritative"));
            }
        }
        None
    }

    pub fn receipt_edit_guard(&self,name:&str,args:&Value)->Option<String> {
        if !matches!(name,"write_to_file"|"replace_in_file"|"multi_replace_file_content") { return None; }
        let paths:Vec<_>=["TargetFile","path","file_path","filePath","file","AbsolutePath"].iter()
            .filter_map(|key|args.get(*key).and_then(Value::as_str)).map(|s|self.root.join(s)).collect();
        let resolve=|p:&Path| fs::canonicalize(p).ok().or_else(|| {
            Some(fs::canonicalize(p.parent()?).ok()?.join(p.file_name()?))
        });
        for action in self.actions.values().filter(|a|self.attempted_actions.contains(&a.id)) {
            for id in &action.receipt_artifacts {
                let requirement=self.contract.as_ref()?.artifacts.iter().find(|a|&a.id==id)?;
                let protected=resolve(&self.root.join(&requirement.path));
                if protected.is_some() && paths.iter().any(|p|resolve(p)==protected) {
                    return Some(format!("Managed receipt '{id}' is immutable after action '{}'. Read the issued receipt and write explanations to a separate requested document; do not replace provider, ID or readback evidence.",action.id));
                }
            }
        }
        None
    }

    pub fn managed_command_guard(&self,name:&str,args:&Value)->Option<String> {
        if self.actions.is_empty() || !matches!(name,"run_command"|"verify_task_tool") {return None;}
        let text=args.get("CommandLine").or_else(||args.get("command")).and_then(Value::as_str).unwrap_or("");
        let mentions=|s:&str| {let s=s.to_lowercase();["pasteme.tyrantg.com","cmd.im","export-command.cjs","publish_verified_rule.cjs","exporttocloudpaste","exportruletocloudpaste","sharepaste("].iter().any(|x|s.contains(x))};
        let mut blocked=mentions(text);
        // Directly named workspace scripts are inspected too. This is not an OS sandbox.
        let tokens=regex::Regex::new(r#"\"([^\"]+)\"|'([^']+)'|([^\s]+)"#).ok()?;
        for capture in tokens.captures_iter(text) {
            let token=(1..=3).find_map(|i|capture.get(i)).map(|m|m.as_str()).unwrap_or("");
            if let Ok(p)=fs::canonicalize(self.root.join(token)) {
                if p.starts_with(&self.root) && p.is_file() && fs::metadata(&p).ok().is_some_and(|m|m.len()<256_000) {
                    blocked|=fs::read_to_string(&p).ok().is_some_and(|s|mentions(&s));
                }
            }
        }
        blocked.then(||"This Run uses managed cloud publication. Do not probe cloud endpoints or invoke export CLI through shell. Fix and verify the local source, then call perform_delivery_action with the registered action_id; use verify_delivery for readback.".into())
    }

    pub fn action_guidance(&self)->String {
        if self.contract.is_none() {return "\n[Delivery tool applicability]\nNo physical delivery contract is registered. verify_delivery is not applicable yet and is not mandatory for ordinary scoped code work. Do not create a plan solely to use it after a coherent change and successful quality check; retain any real explicit requirements.\n".into();}
        if self.actions.is_empty() {
            return if self.completion_policy==CompletionPolicy::PhysicalArtifacts {
                if self.host_physical_delivery_complete() {
                    "\n[Host physical-artifact acceptance]\nAll declared physical artifacts have fresh receipts. Complete any remaining explicit TaskContract steps; otherwise deliver the final answer now. Do not repeat hashes or checks merely to fill a generic phase. These receipts do not certify business behavior or validate historical claims in document text. Report only what this Run actually did.\n".into()
                } else {
                    "\n[Host physical-artifact acceptance]\nFinish the declared files and call verify_delivery for all required artifacts; the tool computes the declared file hashes. Fresh physical receipts satisfy this host's document acceptance; they do not certify business behavior. No unrelated code test or internal trace investigation is required. Distinguish supplied scope, historical document assertions and actual current-Run receipts.\n".into()
                }
            } else {String::new()};
        }
        let mut actions:Vec<_>=self.actions.values().collect();actions.sort_by_key(|a|&a.id);
        let missing:Vec<_>=self.contract.as_ref().map(|c|c.artifacts.iter().filter(|i|i.required&&!self.root.join(&i.path).is_file())
            .map(|i|format!("{} ({})",i.id,i.path)).collect()).unwrap_or_default();
        let rows:Vec<_>=actions.into_iter().map(|a|{
            let status=if let Some(error)=self.errors.get(&a.source) {format!("LOCAL BUSINESS FAILURE: {}",error.chars().take(500).collect::<String>())}
            else if self.contract.as_ref().and_then(|c|c.artifacts.iter().find(|i|i.id==a.source)).is_some_and(|i|!self.root.join(&i.path).is_file()) {"LOCAL SOURCE MISSING: gather only necessary facts and create the actual source artifact; do not repeat probes or planning".into()}
            else if self.records.get(&a.source).is_none_or(|r|!r.domain_verified || file_sha256(&r.path).ok().as_ref()!=Some(&r.sha256)) {"LOCAL SOURCE NEEDS VERIFICATION: call verify_delivery with the source ID; partial verification does not require cloud artifacts".into()}
            else if self.attempted_actions.contains(&a.id) {
                format!("ACTION ALREADY ATTEMPTED: never invoke it again or create another ID. Preserve real action outputs. {} Then call verify_delivery for ALL required artifacts; do not investigate internal traces or invent action outputs.",
                    if missing.is_empty(){"Required files are present; verify the complete delivery.".into()}
                    else{format!("FINISH MISSING REQUIRED ARTIFACTS: {}.",missing.join(", "))})
            }
            else {"Local source verified; use the managed action once. Preserve any returned ID; no guessed update APIs or dummy POSTs".into()};
            format!("action_id={}; source={}; attempted={}; {}",a.id,a.source,self.attempted_actions.contains(&a.id),status)
        }).collect();
        format!("\n[Local-first managed delivery]\n{}\nRepair local business failures before investigating remote publication. Never modify the frozen verifier.\n",rows.join("\n"))
    }

    /// A focused advisory, not a permission change or completion decision.
    pub fn managed_closeout_strategy(&self) -> Option<String> {
        if self.actions.is_empty() || self.declaration_error.is_some() || self.receipt_integrity_gap().is_some()
            || self.actions.keys().any(|id| !self.attempted_actions.contains(id)) {
            return None;
        }
        if let Some((path, expected)) = &self.profile_file {
            if file_sha256(path).as_ref() != Ok(expected) { return None; }
        }
        let contract = self.contract.as_ref()?;
        for action in self.actions.values() {
            let source = contract.artifacts.iter().find(|i| i.id == action.source)?;
            let record = self.records.get(&source.id)?;
            let current = self.inspect(source).ok()?;
            if self.errors.contains_key(&source.id) || !record.domain_verified
                || current.sha256 != record.sha256 || current.path != record.path || current.target != record.target {
                return None;
            }
            self.check_verifier(self.validators.get(source.validator.as_ref()?)?).ok()?;
        }
        let missing: Vec<_> = contract.artifacts.iter()
            .filter(|i| i.required && !self.root.join(&i.path).is_file())
            .map(|i| json!({"id":i.id,"path":i.path})).collect();
        let next = if !missing.is_empty() {
            format!("Write the missing required files in the current execution workspace {}: {}. Reuse existing action outputs; do not browse historical preparation directories or verifier implementations merely to write these files. Then call verify_delivery for ALL required artifacts.", self.root.display(), json!(missing))
        } else if self.completion_gap().is_none() {
            "All required delivery receipts are current. Complete any real explicit TaskContract gap; otherwise give the final answer now using the existing files and receipts. No additional investigation, hashing or publication is needed.".into()
        } else {
            "Required files are present. Call verify_delivery for ALL required artifacts and address only its actual remaining errors before the final answer.".into()
        };
        Some(format!("[Managed delivery closeout] {next} Every registered action was already attempted: never invoke it again or create another external ID. An attempt is not proof of success; preserve real receipts and disclose failures rather than invent outputs. Existing permissions and verification gates remain in force."))
    }

    pub fn completion_gap(&self) -> Option<String> {
        if let Some(error)=self.receipt_integrity_gap() { return Some(error); }
        if let Some(error) = &self.declaration_error { return Some(error.clone()); }
        if let Some((path, expected)) = &self.profile_file {
            if file_sha256(path).as_ref() != Ok(expected) { return Some("host delivery profile changed".into()); }
        }
        let contract = self.contract.as_ref()?;
        for item in contract.artifacts.iter().filter(|i| i.required) {
            let Some(record) = self.records.get(&item.id) else { return Some(format!("delivery '{}' needs verify_delivery: {}", item.id, self.errors.get(&item.id).map(String::as_str).unwrap_or("no physical receipt"))); };
            let fresh = self.inspect(item);
            if !fresh.is_ok_and(|r| r.sha256 == record.sha256 && r.path == record.path && r.target == record.target)
                || (item.validator.is_some() && !record.domain_verified) {
                return Some(format!("delivery '{}' has missing or stale verification", item.id));
            }
            if let Some(id) = &item.validator {
                if self.validators.get(id).is_none_or(|v| self.check_verifier(v).is_err()) { return Some("trusted verifier fingerprint changed".into()); }
            }
        }
        None
    }

    pub fn invalidate(&mut self) { self.records.clear(); }

    pub fn snapshot(&self) -> Value {
        json!({"schema_version":1,"run_id":self.run_id,"session_id":self.session_id,"contract":self.contract,
            "artifacts":self.records,"errors":self.errors,"managed_actions":self.actions,"attempted_actions":self.attempted_actions,
            "issued_receipts":self.issued_receipts,"completion_gap":self.completion_gap()})
    }
}

pub struct PerformDeliveryActionTool(pub super::task_progress::SharedTaskProgress);
impl crate::domains::agent_cognitive::executor::ToolExecutor for PerformDeliveryActionTool {
    fn name(&self)->&'static str {"perform_delivery_action"}
    fn prompt_projection(&self)->Option<String> {
        let guidance=self.0.lock().unwrap_or_else(|e|e.into_inner()).delivery_guidance();
        (!guidance.is_empty()).then(||"Host-managed delivery actions are available in this Run. Use the listed action_id only after verify_delivery confirms the current source. Do not use raw shell publication or dummy network writes. Partial source validation can succeed while completion_ready remains false.".into())
    }
    fn description(&self)->&'static str {"Execute a host-managed action once using a current business-verified artifact. No model-supplied commands, payloads or URLs. Call verify_delivery for the source first; raw cloud shell probes are disallowed in managed Runs."}
    fn parameters_schema(&self)->Value {json!({"type":"object","additionalProperties":false,"required":["action_id"],"properties":{"action_id":{"type":"string"}}})}
    fn execute(&self,args:Value)->Result<Value,String> {
        if args.as_object().is_none_or(|o|o.len()!=1) {return Err("only action_id is accepted".into());}
        let id=args.get("action_id").and_then(Value::as_str).ok_or("action_id is required")?;
        self.0.lock().unwrap_or_else(|e|e.into_inner()).perform_delivery_action(id)
    }
}

pub struct VerifyDeliveryTool(pub super::task_progress::SharedTaskProgress);
impl crate::domains::agent_cognitive::executor::ToolExecutor for VerifyDeliveryTool {
    fn name(&self) -> &'static str { "verify_delivery" }
    fn description(&self) -> &'static str { "Observe declared physical artifacts and run host-registered business verifiers. No model-supplied commands or success claims. Changed files invalidate prior receipts. For rules, static checks cannot replace actual runtime/media verification." }
    fn prompt_projection(&self) -> Option<String> {
        Some("When this Run has declared or host-required delivery artifacts, call verify_delivery after writing them. It reads real files and invokes host-pinned business verifiers. Ordinary successful commands or a probe cannot satisfy required rule delivery; fix reported errors without changing the frozen verifier.".into())
    }
    fn parameters_schema(&self) -> Value { json!({"type":"object","properties":{"artifact_ids":{"type":"array","maxItems":12,"items":{"type":"string"}}}}) }
    fn execute(&self, args: Value) -> Result<Value, String> {
        let ids: Vec<String> = serde_json::from_value(args.get("artifact_ids").cloned().unwrap_or(json!([]))).map_err(|e| e.to_string())?;
        self.0.lock().unwrap_or_else(|e| e.into_inner()).verify_delivery(&ids)
    }
}

#[cfg(test)]
#[path="delivery_action_tests.rs"]
mod managed_action_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn contract(path: &str) -> DeliveryContract {
        DeliveryContract { version: 1, scope: DeliveryScope::Implementation, target: "original-app".into(),
            artifacts: vec![ArtifactRequirement { id: "rule".into(), path: path.into(), required: true, validator: None }] }
    }

    #[test]
    fn invalid_host_plan_reports_the_field_without_mutating_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = DeliveryEvidence::new(dir.path(), "s", "r").unwrap();
        let required = contract("rule.json");
        state.imposed = Some(required.clone());
        state.contract = Some(required.clone());
        fs::write(dir.path().join("rule.json"), "{}").unwrap();
        state.verify(&[]).unwrap();
        let before = state.snapshot();
        let mut candidate = required.clone();
        candidate.target = "wrong-target".into();
        let error = state.validate_declaration(Some(&candidate)).unwrap_err();
        for text in ["delivery.target", "original-app", "wrong-target", "expected", "received"] {
            assert!(error.contains(text), "{error}");
        }
        assert_eq!(state.snapshot(), before);
        candidate = required.clone();
        candidate.artifacts[0].required = false;
        let error = state.validate_declaration(Some(&candidate)).unwrap_err();
        assert!(error.contains("artifact") && error.contains("rule.json") && error.contains("required=true"), "{error}");
        assert_eq!(state.snapshot(), before);
        assert!(state.validate_declaration(Some(&required)).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_aliases_keep_the_host_target_and_fresh_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut state = DeliveryEvidence::new(dir.path(), "s", "r").unwrap();
        let mut required = contract("rule.json");
        required.target = dir.path().to_string_lossy().into_owned();
        state.imposed = Some(required.clone()); state.contract = Some(required.clone());
        fs::write(dir.path().join("rule.json"), "{}").unwrap();
        state.verify(&[]).unwrap();
        for alias in [required.target.replace('\\', "\\\\"), required.target.replace('\\', "/")] {
            let mut declared = required.clone(); declared.target = alias;
            assert_eq!(state.normalize_declaration(&declared).unwrap().target, required.target);
            state.declare(Some(&declared));
            assert_eq!(state.snapshot()["contract"]["target"], required.target);
            assert!(state.completion_gap().is_none(), "equivalent spelling must retain the physical receipt");
        }
        let mut substitute = required.clone(); substitute.target = other.path().to_string_lossy().into_owned();
        assert!(state.normalize_declaration(&substitute).is_err(), "another existing directory is not this target");
        substitute.target = dir.path().join("missing").to_string_lossy().into_owned();
        assert!(state.normalize_declaration(&substitute).is_err());
    }

    #[test]
    fn logical_and_url_targets_do_not_use_filesystem_alias_rules() {
        assert!(!same_target("https://example.invalid/api", "https://example.invalid//api"));
        assert!(!same_target("logical-target", "./logical-target"));
        assert!(same_target("logical-target", "logical-target"));
    }

    #[test]
    fn missing_changed_and_cross_run_artifacts_cannot_complete() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = DeliveryEvidence::new(dir.path(), "session", "run-a").unwrap();
        first.declare(Some(&contract("rule.json")));
        assert!(first.verify(&[]).is_err());
        fs::write(dir.path().join("rule.json"), "{\"target\":\"original-app\"}").unwrap();
        let verified = first.verify(&[]).unwrap();
        assert_eq!(verified["domain_verified"], false);
        assert_eq!(verified["delivery_evidence"]["artifacts"]["rule"]["run_id"], "run-a");
        let mut second = DeliveryEvidence::new(dir.path(), "session", "run-b").unwrap();
        second.declare(Some(&contract("rule.json")));
        assert!(second.completion_gap().is_some(), "a physical file is not a current Run receipt");
        fs::write(dir.path().join("rule.json"), "changed after check").unwrap();
        assert!(first.completion_gap().unwrap().contains("stale"));
        first.verify(&[]).unwrap();
        assert!(first.completion_gap().is_none());
    }

    #[test]
    fn readonly_optional_and_host_requirements_have_distinct_completion_rules() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = DeliveryEvidence::new(dir.path(), "s", "r").unwrap();
        let readonly = DeliveryContract { version: 1, scope: DeliveryScope::ReadOnly, target: "report".into(), artifacts: vec![] };
        state.declare(Some(&readonly));
        assert!(state.completion_gap().is_none());
        let mut optional = contract("missing-optional.txt");
        optional.artifacts[0].required = false;
        state.declare(Some(&optional));
        state.verify(&[]).unwrap();
        assert!(state.snapshot()["errors"]["rule"].is_string());
        let imposed = contract("required-rule.json");
        state.imposed = Some(imposed.clone()); state.contract = Some(imposed);
        state.declare(Some(&readonly));
        assert!(state.completion_gap().unwrap().contains("weaken"));
        state.declare(None);
        assert!(state.completion_gap().is_some(), "omission cannot erase requirements");
    }

    #[test]
    fn target_switch_and_unregistered_validator_are_not_verification() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("rule.json"), "{}").unwrap();
        let mut state = DeliveryEvidence::new(dir.path(), "s", "r").unwrap();
        let mut declaration = contract("rule.json"); declaration.artifacts[0].validator = Some("validate.cjs".into());
        state.imposed = Some(declaration.clone()); state.contract = Some(declaration.clone());
        assert!(state.verify(&[]).unwrap_err().contains("not registered"));
        declaration.target = "substitute-site".into();
        state.declare(Some(&declaration));
        assert!(state.completion_gap().unwrap().contains("weaken"));
        for path in ["../elsewhere", "/outside", "C:/outside", "nested/../../outside"] {
            assert!(contract(path).validate().is_err(), "{path}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn pinned_business_verifier_rejects_false_success_and_mutation() {
        // Exercise the actual process boundary, not a mocked return value.
        let dir = tempfile::tempdir().unwrap(); let work = dir.path().join("work"); fs::create_dir(&work).unwrap();
        fs::write(work.join("rule.json"), "{\"canary\":\"expected\"}").unwrap();
        let python = Command::new("python").args(["-X", "utf8", "-c", "import sys; print(sys.executable)"]).output().unwrap();
        let program = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
        let script = dir.path().join("oracle.py");
        fs::write(&script, "import json,sys\nok=json.load(open(sys.argv[1])).get('canary')=='expected'\nprint(json.dumps({'passed':ok,'target':'original-app','checks':['independent-canary'],'error':'independent business canary mismatch' if not ok else None}))\nsys.exit(0 if ok else 2)\n").unwrap();
        let verifier = VerifierProfile { id: "domain".into(), program: program.clone(), program_sha256: file_sha256(&program).unwrap(),
            script: script.clone(), script_sha256: file_sha256(&script).unwrap(), args: vec!["{artifact}".into()], timeout_seconds: 10 };
        let mut state = DeliveryEvidence::new(&work, "s", "r").unwrap();
        let mut declaration = contract("rule.json"); declaration.artifacts[0].validator = Some("domain".into());
        state.declare(Some(&declaration)); state.validators.insert("domain".into(), verifier.clone());
        let result = state.verify(&[]).unwrap(); assert_eq!(result["domain_verified"], true);
        fs::write(work.join("rule.json"), "{}").unwrap();
        let failure = state.verify(&[]).unwrap_err();
        assert!(failure.contains("independent business canary mismatch"), "the model needs the actual domain diagnostic to repair: {failure}");
        fs::write(&verifier.script, "print('PASS')\n").unwrap();
        assert!(state.completion_gap().is_some());
        assert!(state.verify(&[]).is_err());
        let mut unpinned = verifier; unpinned.script_sha256 = file_sha256(&unpinned.script).unwrap();
        state.validators.insert("domain".into(), unpinned);
        assert!(state.verify(&[]).is_err(), "exit zero and PASS without business JSON cannot certify");
    }
}
