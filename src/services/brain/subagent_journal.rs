//! Single-writer child lifecycle ledger. No automatic process/action replay.
use super::subagent_runtime::{SubagentSnapshot, SubagentState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub version: u8,
    pub sequence: u64,
    pub kind: String,
    pub request_identity: String,
    pub snapshot: SubagentSnapshot,
}
pub struct ChildJournal {
    _lease: File,
    file: Mutex<File>,
    entries: Mutex<BTreeMap<String, Entry>>,
    sequence: Mutex<u64>,
}

impl ChildJournal {
    pub fn open(root: &Path, session: &str) -> Result<Self, String> {
        if session.is_empty()
            || session.len() > 120
            || !session
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Err("invalid child journal session".into());
        }
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(format!("{session}.lock")))
            .map_err(|e| e.to_string())?;
        lease
            .try_lock()
            .map_err(|_| "subagent session already has an active owner".to_string())?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(format!("{session}.jsonl")))
            .map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
            return Err("child journal exceeds 64 MiB budget".into());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let mut entries = BTreeMap::new();
        let mut sequence = 0;
        let mut valid = 0;
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            if line.iter().all(|b| b.is_ascii_whitespace()) {
                valid += line.len();
                continue;
            }
            match serde_json::from_slice::<Entry>(line) {
                Ok(entry) => {
                    if entry.version != 1
                        || entry.snapshot.handle.parent_session_id != session
                        || entry.sequence <= sequence
                    {
                        return Err("child journal identity/order mismatch".into());
                    }
                    sequence = entry.sequence;
                    entries.insert(entry.snapshot.handle.subagent_id.clone(), entry);
                    valid += line.len();
                }
                Err(_) if valid + line.len() == bytes.len() && !line.ends_with(b"\n") => break,
                Err(e) => return Err(format!("child journal is corrupt: {e}")),
            }
        }
        if valid < bytes.len() {
            file.set_len(valid as u64).map_err(|e| e.to_string())?;
        } else if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            file.write_all(b"\n")
                .and_then(|_| file.sync_data())
                .map_err(|e| e.to_string())?;
        }
        Ok(Self {
            _lease: lease,
            file: Mutex::new(file),
            entries: Mutex::new(entries),
            sequence: Mutex::new(sequence),
        })
    }
    pub fn entries(&self) -> Vec<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }
    pub fn append(
        &self,
        kind: &str,
        identity: &str,
        snapshot: &SubagentSnapshot,
    ) -> Result<Entry, String> {
        let mut sequence = self.sequence.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = entries.get(&snapshot.handle.subagent_id) {
            if serde_json::to_value(&old.snapshot).map_err(|e| e.to_string())?
                == serde_json::to_value(snapshot).map_err(|e| e.to_string())?
            {
                return Ok(old.clone());
            }
            if old.snapshot.state.is_terminal() && !snapshot.state.is_terminal() {
                return Err("child journal rejects terminal regression".into());
            }
            if old.snapshot.state.is_terminal() && old.snapshot.state != snapshot.state {
                return Err("child terminal outcome is immutable".into());
            }
        }
        let entry = Entry {
            version: 1,
            sequence: *sequence + 1,
            kind: kind.into(),
            request_identity: identity.into(),
            snapshot: snapshot.clone(),
        };
        let mut bytes = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if file.metadata().map_err(|e| e.to_string())?.len() + bytes.len() as u64 > 64 * 1024 * 1024
        {
            return Err("child journal exceeds 64 MiB budget".into());
        }
        file.seek(SeekFrom::End(0))
            .and_then(|_| file.write_all(&bytes))
            .and_then(|_| file.sync_data())
            .map_err(|e| e.to_string())?;
        *sequence += 1;
        entries.insert(snapshot.handle.subagent_id.clone(), entry.clone());
        Ok(entry)
    }
    pub fn reconcile(&self, reason: &str) -> Result<Vec<Value>, String> {
        let mut frames = Vec::new();
        for entry in self.entries() {
            let mut snapshot = entry.snapshot;
            if !snapshot.state.is_terminal() {
                snapshot.state = match reason {
                    "cancelled" => SubagentState::Cancelled,
                    "timeout" => SubagentState::TimedOut,
                    _ => SubagentState::Failed,
                };
                snapshot.updated_at = chrono::Utc::now().timestamp_millis();
                snapshot.error = Some(format!(
                    "parent {reason}; no child terminal receipt, execution will not be replayed"
                ));
                snapshot.process_reaped =
                    process_is_gone(snapshot.process_id, snapshot.process_birth.as_deref());
                self.append("reconciled_parent_exit", &entry.request_identity, &snapshot)?;
            } else if !snapshot.process_reaped
                && process_is_gone(snapshot.process_id, snapshot.process_birth.as_deref())
            {
                snapshot.process_reaped = true;
                self.append("cleanup_confirmed", &entry.request_identity, &snapshot)?;
            }
            frames.push(json!({"type":"subagent_runtime","schemaVersion":1,"event":"snapshot","reconciled":true,"snapshot":snapshot}));
        }
        Ok(frames)
    }
}

pub fn root() -> PathBuf {
    crate::config::app_paths::AppPaths::new().data_path(&["agent-subagents"])
}
pub fn after_parent_exit(session: &str, reason: &str) -> Result<Vec<Value>, String> {
    ChildJournal::open(&root(), session)?.reconcile(reason)
}

/// Creation identity avoids confusing a reused PID with the old worker. This
/// function never terminates a PID read from disk.
pub fn process_birth(pid: u32) -> Option<String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, FILETIME},
            System::Threading::*,
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut birth: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut birth, &mut exit, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        return ok.then(|| format!("{:08x}{:08x}", birth.dwHighDateTime, birth.dwLowDateTime));
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}
pub fn process_is_gone(pid: Option<u32>, birth: Option<&str>) -> bool {
    let Some(pid) = pid else {
        return true;
    };
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError},
            System::Threading::*,
        };
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZATION_SYNCHRONIZE,
            0,
            pid,
        );
        if handle.is_null() {
            return GetLastError() == ERROR_INVALID_PARAMETER;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        if ok && code != 259 {
            return true;
        }
        return birth
            .is_some_and(|expected| process_birth(pid).is_some_and(|actual| actual != expected));
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, birth);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> SubagentSnapshot {
        serde_json::from_value(json!({"handle":{"subagentId":"11111111-1111-4111-8111-111111111111","parentSessionId":"session","parentRunId":"run","correlationId":"once","role":"coder","depth":1,"createdAt":0,"workspacePath":std::env::temp_dir(),"originProject":std::env::temp_dir(),"toolPolicy":"yolo"},"state":"running","updatedAt":0,"result":null,"error":null,"recentEvents":[],"processId":null,"executionWorkspace":null,"lastHeartbeatAt":null,"processReaped":false,"businessVerified":false})).unwrap()
    }
    #[test]
    fn lease_recovery_and_terminal_dedup_never_replay_execution() {
        let root = tempfile::tempdir().unwrap();
        let journal = ChildJournal::open(root.path(), "session").unwrap();
        assert!(ChildJournal::open(root.path(), "session").is_err());
        journal.append("started", "identity", &snapshot()).unwrap();
        drop(journal);
        let journal = ChildJournal::open(root.path(), "session").unwrap();
        let frames = journal.reconcile("timeout").unwrap();
        assert_eq!(frames[0]["snapshot"]["state"], "timed_out");
        assert_eq!(frames[0]["snapshot"]["processReaped"], true);
        let len = std::fs::metadata(root.path().join("session.jsonl"))
            .unwrap()
            .len();
        assert_eq!(journal.reconcile("interrupted").unwrap(), frames);
        assert_eq!(
            std::fs::metadata(root.path().join("session.jsonl"))
                .unwrap()
                .len(),
            len
        );
        let mut stale = snapshot();
        stale.state = SubagentState::Succeeded;
        assert!(journal.append("late_success", "identity", &stale).is_err());
    }
    #[test]
    fn torn_tail_recovers_but_corrupt_history_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let journal = ChildJournal::open(root.path(), "session").unwrap();
        journal.append("start", "identity", &snapshot()).unwrap();
        drop(journal);
        let path = root.path().join("session.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"version\":")
            .unwrap();
        let journal = ChildJournal::open(root.path(), "session").unwrap();
        assert_eq!(journal.entries().len(), 1);
        journal.reconcile("interrupted").unwrap();
        drop(journal);
        assert_eq!(
            ChildJournal::open(root.path(), "session")
                .unwrap()
                .entries()[0]
                .snapshot
                .state,
            SubagentState::Failed
        );
        std::fs::write(&path, b"not-json\n").unwrap();
        assert!(ChildJournal::open(root.path(), "session").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn process_creation_identity_distinguishes_live_dead_and_reused_pids() {
        let pid = std::process::id();
        let actual = process_birth(pid).unwrap();
        assert!(!process_is_gone(Some(pid), Some(&actual)));
        assert!(process_is_gone(Some(pid), Some("different-creation-time")));
        assert!(process_is_gone(Some(u32::MAX), Some("old")));
    }
}
