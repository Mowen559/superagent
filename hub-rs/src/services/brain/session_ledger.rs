use crate::config::app_paths::AppPaths;
use crate::services::brain::events::SessionEvent;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;
use xxhash_rust::xxh3::xxh3_64;

#[derive(Debug, Clone, Copy)]
struct PersistedState {
    initialized: bool,
    event_count: usize,
    fingerprint: u64,
}

pub struct SessionLedger {
    session_id: String,
    path: PathBuf,
    state: Mutex<PersistedState>,
}

impl SessionLedger {
    pub fn default_location() -> Self {
        let session_id = std::env::var("HUB_AGENT_SESSION_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "default".to_string());
        let root = AppPaths::new().data_path(&["agent-sessions"]);
        Self::new(root, &session_id).unwrap_or_else(|_| {
            Self::new(AppPaths::new().data_path(&["agent-sessions"]), "default")
                .expect("default session id is valid")
        })
    }

    pub fn new(root: PathBuf, session_id: &str) -> Result<Self, String> {
        validate_session_id(session_id)?;
        Ok(Self {
            session_id: session_id.to_string(),
            path: root.join(format!("{session_id}.events.jsonl")),
            state: Mutex::new(PersistedState {
                initialized: false,
                event_count: 0,
                fingerprint: fingerprint(&[]),
            }),
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Vec<SessionEvent>, String> {
        self.recover_backup_if_needed()?;
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)
            .map_err(|error| format!("failed to read session ledger: {error}"))?;
        let lines = content.lines().collect::<Vec<_>>();
        let mut events = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SessionEvent>(line) {
                Ok(event) => events.push(event),
                Err(_) if index + 1 == lines.len() => break,
                Err(error) => {
                    return Err(format!(
                        "session ledger is corrupt at line {}: {}",
                        index + 1,
                        error
                    ));
                }
            }
        }
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = PersistedState {
            initialized: true,
            event_count: events.len(),
            fingerprint: fingerprint(&events),
        };
        Ok(events)
    }

    pub fn sync(&self, events: &[SessionEvent]) -> Result<(), String> {
        let needs_load = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            !state.initialized && self.path.exists()
        };
        if needs_load {
            self.load()?;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let append_only = events.len() >= state.event_count
            && fingerprint(&events[..state.event_count]) == state.fingerprint;

        if append_only {
            self.append(&events[state.event_count..])?;
        } else {
            self.checkpoint(events)?;
        }
        *state = PersistedState {
            initialized: true,
            event_count: events.len(),
            fingerprint: fingerprint(events),
        };
        Ok(())
    }

    fn append(&self, events: &[SessionEvent]) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create session ledger directory: {error}"))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("failed to open session ledger: {error}"))?;
        for event in events {
            serde_json::to_writer(&mut file, event)
                .map_err(|error| format!("failed to serialize session event: {error}"))?;
            file.write_all(b"\n")
                .map_err(|error| format!("failed to append session event: {error}"))?;
        }
        file.sync_data()
            .map_err(|error| format!("failed to flush session ledger: {error}"))
    }

    fn checkpoint(&self, events: &[SessionEvent]) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "session ledger has no parent directory".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create session ledger directory: {error}"))?;
        let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4()));
        let backup = self.path.with_extension("jsonl.bak");
        {
            let mut file = fs::File::create(&temporary)
                .map_err(|error| format!("failed to create session checkpoint: {error}"))?;
            for event in events {
                serde_json::to_writer(&mut file, event)
                    .map_err(|error| format!("failed to serialize session checkpoint: {error}"))?;
                file.write_all(b"\n")
                    .map_err(|error| format!("failed to write session checkpoint: {error}"))?;
            }
            file.sync_data()
                .map_err(|error| format!("failed to flush session checkpoint: {error}"))?;
        }

        if self.path.exists() {
            if backup.exists() {
                fs::remove_file(&backup)
                    .map_err(|error| format!("failed to clear stale session backup: {error}"))?;
            }
            fs::rename(&self.path, &backup)
                .map_err(|error| format!("failed to stage session ledger: {error}"))?;
        }
        if let Err(error) = fs::rename(&temporary, &self.path) {
            if backup.exists() {
                let _ = fs::rename(&backup, &self.path);
            }
            return Err(format!("failed to commit session checkpoint: {error}"));
        }
        if backup.exists() {
            let _ = fs::remove_file(backup);
        }
        Ok(())
    }

    fn recover_backup_if_needed(&self) -> Result<(), String> {
        let backup = self.path.with_extension("jsonl.bak");
        if !self.path.exists() && backup.exists() {
            fs::rename(&backup, &self.path)
                .map_err(|error| format!("failed to recover session ledger backup: {error}"))?;
        }
        Ok(())
    }
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("session id must contain only ASCII letters, digits, '-' or '_'".to_string());
    }
    Ok(())
}

fn fingerprint(events: &[SessionEvent]) -> u64 {
    let bytes = serde_json::to_vec(events).unwrap_or_default();
    xxh3_64(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn appends_new_events_and_reloads_them() {
        let dir = tempdir().unwrap();
        let ledger = SessionLedger::new(dir.path().to_path_buf(), "session_1").unwrap();
        ledger
            .sync(&[SessionEvent::UserInput("one".into())])
            .unwrap();
        ledger
            .sync(&[
                SessionEvent::UserInput("one".into()),
                SessionEvent::ModelThought("two".into()),
            ])
            .unwrap();

        let reloaded = SessionLedger::new(dir.path().to_path_buf(), "session_1").unwrap();
        assert_eq!(reloaded.load().unwrap().len(), 2);
        assert_eq!(
            fs::read_to_string(ledger.path()).unwrap().lines().count(),
            2
        );
    }

    #[test]
    fn rewrites_checkpoint_when_history_changes() {
        let dir = tempdir().unwrap();
        let ledger = SessionLedger::new(dir.path().to_path_buf(), "session_1").unwrap();
        ledger
            .sync(&[
                SessionEvent::UserInput("one".into()),
                SessionEvent::ToolResult {
                    id: "1".into(),
                    result: json!({ "large": true }),
                },
            ])
            .unwrap();
        ledger
            .sync(&[SessionEvent::ContextCompacted {
                summary: "checkpoint".into(),
                original_event_count: 2,
            }])
            .unwrap();

        let events = ledger.load().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SessionEvent::ContextCompacted { .. }));
    }

    #[test]
    fn ignores_a_partial_trailing_record_after_crash() {
        let dir = tempdir().unwrap();
        let ledger = SessionLedger::new(dir.path().to_path_buf(), "session_1").unwrap();
        ledger
            .sync(&[SessionEvent::UserInput("safe".into())])
            .unwrap();
        let mut file = OpenOptions::new().append(true).open(ledger.path()).unwrap();
        file.write_all(b"{\"type\":\"ToolCall\"").unwrap();
        drop(file);

        assert_eq!(ledger.load().unwrap().len(), 1);
    }

    #[test]
    fn isolates_sessions_and_rejects_unsafe_ids() {
        let dir = tempdir().unwrap();
        let first = SessionLedger::new(dir.path().to_path_buf(), "first").unwrap();
        let second = SessionLedger::new(dir.path().to_path_buf(), "second").unwrap();
        first
            .sync(&[SessionEvent::UserInput("one".into())])
            .unwrap();
        second
            .sync(&[SessionEvent::UserInput("two".into())])
            .unwrap();

        assert_ne!(first.path(), second.path());
        assert!(SessionLedger::new(dir.path().to_path_buf(), "../escape").is_err());
    }

    #[test]
    fn sync_initializes_from_existing_ledger_without_duplicate_append() {
        let dir = tempdir().unwrap();
        let first = SessionLedger::new(dir.path().to_path_buf(), "session_1").unwrap();
        let events = vec![SessionEvent::UserInput("one".into())];
        first.sync(&events).unwrap();

        let restarted = SessionLedger::new(dir.path().to_path_buf(), "session_1").unwrap();
        restarted.sync(&events).unwrap();

        assert_eq!(fs::read_to_string(first.path()).unwrap().lines().count(), 1);
    }
}
