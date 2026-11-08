//! Owned current-content snapshots and explicit, fingerprinted collection.
use super::subagent_runtime::SubagentHandle;
use crate::domains::version_control::change_set::{self, BaselineManifest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIdentity {
    pub root: PathBuf,
    pub metadata_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    owner: SubagentHandle,
    baseline: BaselineManifest,
}

pub struct OwnedWorkspace {
    pub identity: WorkspaceIdentity,
    pub path: PathBuf,
    metadata: Metadata,
    applied: Mutex<Option<Value>>,
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map(|p| crate::infrastructure::fs_path::normalize_path(&p))
        .map_err(|e| e.to_string())
}
fn hash(bytes: &[u8]) -> String {
    crate::shared_kernel::crypto::sha256_hex(bytes)
}
fn owner_matches(a: &SubagentHandle, b: &SubagentHandle) -> bool {
    a.replaces_subagent_id == b.replaces_subagent_id && a.subagent_id == b.subagent_id
        && a.parent_run_id == b.parent_run_id
        && a.parent_session_id == b.parent_session_id
        && a.workspace_path == b.workspace_path
        && a.origin_project == b.origin_project
        && a.tool_policy == b.tool_policy
}

pub fn safe_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if canonical(root)? != root {
        return Err("bound workspace root changed or was replaced by a link".into());
    }
    use std::path::Component;
    let rel = Path::new(relative);
    if rel.as_os_str().is_empty()
        || rel.components().any(|c| !matches!(c, Component::Normal(_)))
        || relative.contains(['\\', ':'])
    {
        return Err("unsafe artifact relative path".into());
    }
    let mut current = root.to_path_buf();
    let count = rel.components().count();
    for (index, part) in rel.components().enumerate() {
        current.push(part);
        if let Ok(meta) = std::fs::symlink_metadata(&current) {
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let linked = meta.file_type().is_symlink();
            if linked || !canonical(&current)?.starts_with(root) {
                return Err("artifact path escapes owned workspace or uses a link".into());
            }
            if index + 1 < count && !meta.is_dir() {
                return Err("artifact parent path is a file, not a directory".into());
            }
        }
    }
    Ok(current)
}

fn reject_links(root: &Path) -> Result<(), String> {
    let mut dirs = vec![root.to_path_buf()];
    let mut count = 0usize;
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() == ".git" {
                continue;
            }
            count += 1;
            if count > 200_000 {
                return Err("artifact tree exceeds entry budget".into());
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let path = safe_file(root, &relative)?;
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                dirs.push(path);
            }
        }
    }
    Ok(())
}

async fn git(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("git");
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        cmd.env_remove(key);
    }
    cmd.args([
        "-c",
        "core.autocrlf=false",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "user.name=Hub",
        "-c",
        "user.email=hub@localhost",
    ])
    .args(args)
    .env("GIT_CONFIG_NOSYSTEM", "1")
    .env("GIT_CONFIG_GLOBAL", root.join("git-global-empty"))
    .current_dir(root.join("base"))
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let tree = match crate::services::agent::proc::RunProcessTree::attach(&child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(format!("cannot contain snapshot Git process: {error}"));
        }
    };
    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .map_err(|_| "owned snapshot Git operation timed out".to_string())?
        .map_err(|e| e.to_string())?;
    tree.terminate();
    if !output.status.success() {
        return Err(format!(
            "owned snapshot Git {} failed ({}): {}",
            args.first().copied().unwrap_or("operation"),
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1000)
                .collect::<String>()
        ));
    }
    Ok(())
}

impl OwnedWorkspace {
    pub fn applied_receipt(&self) -> Option<Value> {
        self.applied
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn restore_receipt(&self, receipt: Value) -> Result<(), String> {
        if receipt["parentRunId"] != self.metadata.owner.parent_run_id
            || receipt["subagentId"] != self.metadata.owner.subagent_id
        {
            return Err("collection receipt ownership mismatch".into());
        }
        *self.applied.lock().unwrap_or_else(|e| e.into_inner()) = Some(receipt);
        Ok(())
    }
    pub async fn prepare(owner: &SubagentHandle, managed_root: &Path) -> Result<Self, String> {
        uuid::Uuid::parse_str(&owner.subagent_id).map_err(|_| "invalid owned child ID")?;
        std::fs::create_dir_all(managed_root).map_err(|e| e.to_string())?;
        let root = canonical(managed_root)?.join(&owner.subagent_id);
        std::fs::create_dir(&root).map_err(|e| format!("cannot create owned snapshot: {e}"))?;
        let source = owner.workspace_path.clone();
        let copy_root = root.clone();
        let metadata_owner = owner.clone();
        let metadata=tokio::task::spawn_blocking(move||{
            let baseline=change_set::capture_baseline(&source,&copy_root.join("input-baseline.json"))?;
            let base=copy_root.join("base");std::fs::create_dir(&base).map_err(|e|e.to_string())?;
            for (relative,fingerprint) in &baseline.files{
                use std::io::Read;
                let input=safe_file(&source,relative)?;
                let file=std::fs::File::open(&input).map_err(|e|e.to_string())?;
                let mut bytes=Vec::new();file.take(fingerprint.bytes+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
                if bytes.len() as u64!=fingerprint.bytes||hash(&bytes)!=fingerprint.sha256{return Err("parent input changed during bounded snapshot copy".into());}
                let output=safe_file(&base,relative)?;
                if let Some(parent)=output.parent(){std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
                std::fs::write(&output,&bytes).map_err(|e|e.to_string())?;
                std::fs::set_permissions(&output,std::fs::metadata(&input).map_err(|e|e.to_string())?.permissions()).map_err(|e|e.to_string())?;
            }
            let source_after=change_set::capture_baseline(&source,&copy_root.join("input-after.json"))?;
            let copied=change_set::capture_baseline(&copy_root.join("base"),&copy_root.join("copied.json"))?;
            if baseline.files!=source_after.files||baseline.files!=copied.files{return Err("parent changed or snapshot omitted tracked input; shared workspace fallback forbidden".to_string());}
            let metadata=Metadata{owner:metadata_owner,baseline};
            std::fs::write(copy_root.join("owner.json"),serde_json::to_vec(&metadata).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            std::fs::write(copy_root.join("git-global-empty"),"").map_err(|e|e.to_string())?;
            Ok::<_,String>(metadata)
        }).await.map_err(|e|e.to_string())??;
        git(&root, &["init", "--quiet"]).await?;
        git(&root, &["add", "--force", "--", "."]).await?;
        git(
            &root,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "Owned current Run snapshot",
            ],
        )
        .await?;
        let destination = root.join("workspace");
        git(
            &root,
            &[
                "worktree",
                "add",
                "--detach",
                destination.to_str().ok_or("non UTF-8 snapshot path")?,
                "HEAD",
            ],
        )
        .await?;
        let output =
            change_set::capture_baseline(&destination, &root.join("worktree-baseline.json"))?;
        if metadata.baseline.files != output.files {
            return Err(
                "worktree differs from current parent input; shared workspace fallback forbidden"
                    .into(),
            );
        }
        // Knowledge origin is separate from the Git/code baseline.
        std::fs::write(root.join("baseline.json"),json!({"projectPath":owner.origin_project,"parentRunId":owner.parent_run_id,"subagentId":owner.subagent_id}).to_string()).map_err(|e|e.to_string())?;
        let identity = WorkspaceIdentity {
            root: root.clone(),
            metadata_sha256: hash(
                &std::fs::read(root.join("owner.json")).map_err(|e| e.to_string())?,
            ),
        };
        Ok(Self {
            identity,
            path: canonical(&destination)?,
            metadata,
            applied: Mutex::new(None),
        })
    }

    pub fn reopen(
        identity: WorkspaceIdentity,
        owner: &SubagentHandle,
        managed_root: &Path,
    ) -> Result<Self, String> {
        let expected = canonical(managed_root)?.join(&owner.subagent_id);
        if identity.root != expected || canonical(&identity.root)? != expected {
            return Err("workspace ownership root mismatch".into());
        }
        let bytes = std::fs::read(expected.join("owner.json")).map_err(|e| e.to_string())?;
        if hash(&bytes) != identity.metadata_sha256 {
            return Err("workspace ownership receipt changed".into());
        }
        let metadata: Metadata = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !owner_matches(&metadata.owner, owner) {
            return Err("workspace does not belong to this parent Run".into());
        }
        Ok(Self {
            path: canonical(&expected.join("workspace"))?,
            identity,
            metadata,
            applied: Mutex::new(None),
        })
    }

    fn verify(&self, owner: &SubagentHandle) -> Result<(), String> {
        if !owner_matches(&self.metadata.owner, owner)
            || canonical(&self.identity.root.join("workspace"))? != self.path
            || hash(
                &std::fs::read(self.identity.root.join("owner.json")).map_err(|e| e.to_string())?,
            ) != self.identity.metadata_sha256
        {
            return Err("owned artifact identity changed".into());
        }
        reject_links(&self.path)?;
        Ok(())
    }

    pub fn inspect(&self, owner: &SubagentHandle) -> Result<Value, String> {
        self.verify(owner)?;
        let mut receipt = change_set::inspect_changes(
            &self.metadata.baseline,
            &owner.workspace_path,
            &self.path,
        )?;
        for file in &receipt.files {
            let target = safe_file(&owner.workspace_path, &file.path)?;
            if target.is_dir() && !receipt.conflicts.contains(&file.path) {
                receipt.conflicts.push(file.path.clone());
            }
            safe_file(&self.path, &file.path)?;
        }
        let proposed = change_set::workspace_fingerprint(&self.path)?;
        let digest = hash(
            &serde_json::to_vec(&(
                owner.parent_run_id.clone(),
                owner.subagent_id.clone(),
                &self.identity.metadata_sha256,
                &proposed,
            ))
            .map_err(|e| e.to_string())?,
        );
        Ok(
            json!({"schemaVersion":1,"subagentId":owner.subagent_id,"parentRunId":owner.parent_run_id,"proposalDigest":digest,"changes":receipt,"businessVerified":false}),
        )
    }

    pub fn apply(
        &self,
        owner: &SubagentHandle,
        digest: &str,
        guard: impl Fn(&str) -> Result<(), String>,
    ) -> Result<Value, String> {
        let mut applied = self.applied.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(receipt) = applied.as_ref() {
            if receipt["proposalDigest"] == digest {
                return Ok(json!({"alreadyApplied":true,"receipt":receipt}));
            }
            return Err("owned child already collected with a different proposal".into());
        }
        // Never replay an incomplete write after restart, even if no final receipt exists.
        if self.identity.root.join("merge-pending.json").exists() {
            return Err(
                "collection was already attempted; explicit recovery review required, no replay"
                    .into(),
            );
        }
        let inspection = self.inspect(owner)?;
        if inspection["proposalDigest"] != digest {
            return Err("artifact fingerprint changed since inspection".into());
        }
        let receipt: change_set::ChangeSetReceipt =
            serde_json::from_value(inspection["changes"].clone()).map_err(|e| e.to_string())?;
        if !receipt.conflicts.is_empty() {
            return Err(format!(
                "parent file conflicts: {}",
                receipt.conflicts.join(", ")
            ));
        }
        for file in &receipt.files {
            guard(&file.path)?;
        }
        std::fs::write(
            self.identity.root.join("merge-pending.json"),
            inspection.to_string(),
        )
        .map_err(|e| e.to_string())?;
        let result =
            change_set::apply_changes(&receipt, &owner.workspace_path, &self.path, None, false)?;
        let result = json!({"proposalDigest":digest,"subagentId":owner.subagent_id,"parentRunId":owner.parent_run_id,"apply":result,"businessVerified":false});
        *applied = Some(result.clone());
        std::fs::write(
            self.identity.root.join("merge-receipt.json"),
            result.to_string(),
        )
        .map_err(|e| format!("files collected but receipt persistence failed; no replay: {e}"))?;
        Ok(json!({"alreadyApplied":false,"receipt":result}))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tool_policy::WorkMode;
    use super::*;
    fn owner(root: &Path) -> SubagentHandle {
        SubagentHandle {
            replaces_subagent_id:None,
            subagent_id: uuid::Uuid::new_v4().to_string(),
            parent_session_id: "owner".into(),
            parent_run_id: "run".into(),
            role: "coder".into(),
            correlation_id: None,
            depth: 1,
            created_at: 0,
            workspace_path: canonical(root).unwrap(),
            origin_project: canonical(root).unwrap(),
            tool_policy: WorkMode::Yolo,
        }
    }
    #[tokio::test]
    async fn snapshot_keeps_dirty_and_untracked_input_without_touching_source_git() {
        let source = tempfile::tempdir().unwrap();
        let managed = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("code.js"), "current uncommitted code").unwrap();
        std::fs::write(source.path().join("new.js"), "untracked input").unwrap();
        std::fs::File::create(source.path().join("large-resource.bin"))
            .unwrap()
            .set_len(11 * 1024 * 1024)
            .unwrap();
        let handle = owner(source.path());
        let owned = OwnedWorkspace::prepare(&handle, managed.path())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(owned.path.join("code.js")).unwrap(),
            "current uncommitted code"
        );
        assert_eq!(
            std::fs::read_to_string(owned.path.join("new.js")).unwrap(),
            "untracked input"
        );
        assert!(!source.path().join(".git").exists());
        assert!(!owned.path.join("large-resource.bin").exists());
        assert_eq!(
            crate::infrastructure::fs_path::resolve_origin_project_from_baseline(&owned.path, &[]),
            handle.origin_project
        );
    }
    #[tokio::test]
    async fn collection_binds_fingerprint_conflicts_ownership_and_one_time_application() {
        let source = tempfile::tempdir().unwrap();
        let managed = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("code.js"), "before").unwrap();
        let handle = owner(source.path());
        let owned = OwnedWorkspace::prepare(&handle, managed.path())
            .await
            .unwrap();
        std::fs::write(owned.path.join("code.js"), "child").unwrap();
        std::fs::write(owned.path.join("extra.js"), "new artifact").unwrap();
        let inspected = owned.inspect(&handle).unwrap();
        let digest = inspected["proposalDigest"].as_str().unwrap();
        std::fs::create_dir(source.path().join("extra.js")).unwrap();
        assert!(
            owned
                .apply(&handle, digest, |_| Ok(()))
                .unwrap_err()
                .contains("conflict")
        );
        assert_eq!(
            std::fs::read_to_string(source.path().join("code.js")).unwrap(),
            "before"
        );
        std::fs::remove_dir(source.path().join("extra.js")).unwrap();
        let mut foreign = handle.clone();
        foreign.parent_run_id = "other".into();
        assert!(owned.inspect(&foreign).is_err());
        std::fs::write(source.path().join("code.js"), "parent concurrent edit").unwrap();
        assert!(
            owned
                .apply(&handle, digest, |_| Ok(()))
                .unwrap_err()
                .contains("conflicts")
        );
        assert!(!source.path().join("extra.js").exists());
        assert_eq!(
            std::fs::read_to_string(source.path().join("code.js")).unwrap(),
            "parent concurrent edit"
        );
        std::fs::write(source.path().join("code.js"), "before").unwrap();
        std::fs::write(owned.path.join("extra.js"), "changed after inspection").unwrap();
        assert!(
            owned
                .apply(&handle, digest, |_| Ok(()))
                .unwrap_err()
                .contains("fingerprint")
        );
        let fresh = owned.inspect(&handle).unwrap();
        let fresh = fresh["proposalDigest"].as_str().unwrap();
        assert!(
            owned
                .apply(&handle, fresh, |f| if f == "extra.js" {
                    Err("protected receipt".into())
                } else {
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(source.path().join("code.js")).unwrap(),
            "before"
        );
        let result = owned.apply(&handle, fresh, |_| Ok(())).unwrap();
        assert_eq!(result["alreadyApplied"], false);
        assert_eq!(
            std::fs::read_to_string(source.path().join("code.js")).unwrap(),
            "child"
        );
        std::fs::write(source.path().join("code.js"), "user edit after collection").unwrap();
        assert_eq!(
            owned.apply(&handle, fresh, |_| Ok(())).unwrap()["alreadyApplied"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(source.path().join("code.js")).unwrap(),
            "user edit after collection"
        );
        let reopened =
            OwnedWorkspace::reopen(owned.identity.clone(), &handle, managed.path()).unwrap();
        assert!(
            reopened
                .apply(&handle, fresh, |_| Ok(()))
                .unwrap_err()
                .contains("no replay")
        );
        reopened.restore_receipt(result["receipt"].clone()).unwrap();
        assert_eq!(
            reopened.apply(&handle, fresh, |_| Ok(())).unwrap()["alreadyApplied"],
            true
        );
    }
    #[tokio::test]
    async fn forged_metadata_or_links_cannot_escape_collection() {
        let source = tempfile::tempdir().unwrap();
        let managed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("code.js"), "original").unwrap();
        let handle = owner(source.path());
        let owned = OwnedWorkspace::prepare(&handle, managed.path())
            .await
            .unwrap();
        assert!(safe_file(&handle.workspace_path, "../outside").is_err());
        #[cfg(windows)]
        {
            assert!(
                std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(owned.path.join("linked"))
                    .arg(outside.path())
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            assert!(owned.inspect(&handle).unwrap_err().contains("link"));
        }
        std::fs::write(owned.identity.root.join("owner.json"), "{}").unwrap();
        assert!(OwnedWorkspace::reopen(owned.identity.clone(), &handle, managed.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(source.path().join("code.js")).unwrap(),
            "original"
        );
    }
}
