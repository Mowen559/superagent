use crate::services::brain::tool_call_identity::fingerprint;
use crate::services::brain::tool_policy::ToolRisk;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const DEFAULT_TTL_SECONDS: i64 = 15 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Consumed,
    Expired,
}

impl ApprovalStatus {
    fn directory(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Consumed => "consumed",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolApprovalRecord {
    pub id: String,
    pub request_hash: String,
    pub tool_name: String,
    pub risk: ToolRisk,
    pub created_at: String,
    pub expires_at: String,
    pub expires_at_ms: i64,
    pub status: ApprovalStatus,
}

/// File-backed approval control plane shared by the Hub HTTP process and
/// superagent subprocesses. Directory moves are the source of truth for state,
/// so consuming an approved record is atomic and single-use across processes.
#[derive(Debug, Clone)]
pub struct ToolApprovalStore {
    root: PathBuf,
}

impl ToolApprovalStore {
    pub fn default_location() -> Self {
        let paths = crate::config::app_paths::AppPaths::new();
        Self::new(paths.data_path(&["tool-approvals"]))
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn request(
        &self,
        tool_name: &str,
        arguments: &Value,
        risk: ToolRisk,
    ) -> io::Result<ToolApprovalRecord> {
        self.ensure_directories()?;
        let request_hash = fingerprint(tool_name, arguments);
        let now_ms = chrono::Utc::now().timestamp_millis();

        for record in self.records_in(ApprovalStatus::Pending)? {
            if record.request_hash == request_hash && record.expires_at_ms > now_ms {
                return Ok(record);
            }
        }

        let created = chrono::Utc::now();
        let expires = created + chrono::Duration::seconds(DEFAULT_TTL_SECONDS);
        let record = ToolApprovalRecord {
            id: Uuid::new_v4().simple().to_string(),
            request_hash,
            tool_name: tool_name.to_string(),
            risk,
            created_at: created.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
            expires_at_ms: expires.timestamp_millis(),
            status: ApprovalStatus::Pending,
        };
        self.write_new(&record)?;
        Ok(record)
    }

    pub fn approve(&self, id: &str) -> io::Result<ToolApprovalRecord> {
        self.transition(id, ApprovalStatus::Pending, ApprovalStatus::Approved)
    }

    pub fn deny(&self, id: &str) -> io::Result<ToolApprovalRecord> {
        self.transition(id, ApprovalStatus::Pending, ApprovalStatus::Denied)
    }

    pub fn consume_matching(
        &self,
        tool_name: &str,
        arguments: &Value,
    ) -> io::Result<Option<ToolApprovalRecord>> {
        self.ensure_directories()?;
        let expected_hash = fingerprint(tool_name, arguments);
        let now_ms = chrono::Utc::now().timestamp_millis();

        for record in self.records_in(ApprovalStatus::Approved)? {
            if record.expires_at_ms <= now_ms {
                let _ = self.transition(
                    &record.id,
                    ApprovalStatus::Approved,
                    ApprovalStatus::Expired,
                );
                continue;
            }
            if record.request_hash != expected_hash {
                continue;
            }

            match self.transition(
                &record.id,
                ApprovalStatus::Approved,
                ApprovalStatus::Consumed,
            ) {
                Ok(consumed) => return Ok(Some(consumed)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    pub fn get(&self, id: &str) -> io::Result<Option<ToolApprovalRecord>> {
        validate_id(id)?;
        self.ensure_directories()?;
        for status in all_statuses() {
            let path = self.record_path(status, id);
            if path.is_file() {
                return self.read_record(&path, status).map(Some);
            }
        }
        Ok(None)
    }

    pub fn list(&self) -> io::Result<Vec<ToolApprovalRecord>> {
        self.ensure_directories()?;
        let mut records = Vec::new();
        for status in all_statuses() {
            records.extend(self.records_in(status)?);
        }
        records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(records)
    }

    fn transition(
        &self,
        id: &str,
        from: ApprovalStatus,
        to: ApprovalStatus,
    ) -> io::Result<ToolApprovalRecord> {
        validate_id(id)?;
        self.ensure_directories()?;
        let source = self.record_path(from, id);
        let destination = self.record_path(to, id);
        fs::rename(&source, &destination)?;
        let mut record = self.read_record(&destination, to)?;
        record.status = to;
        // The directory move above is the atomic state transition. Rewriting
        // merely keeps the human-readable JSON status consistent.
        fs::write(
            &destination,
            serde_json::to_vec_pretty(&record).map_err(json_error)?,
        )?;
        Ok(record)
    }

    fn write_new(&self, record: &ToolApprovalRecord) -> io::Result<()> {
        let directory = self.root.join(record.status.directory());
        let destination = self.record_path(record.status, &record.id);
        let temporary = directory.join(format!(".{}.{}.tmp", record.id, Uuid::new_v4().simple()));
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(record).map_err(json_error)?,
        )?;
        fs::rename(temporary, destination)
    }

    fn records_in(&self, status: ApprovalStatus) -> io::Result<Vec<ToolApprovalRecord>> {
        let mut records = Vec::new();
        for entry in fs::read_dir(self.root.join(status.directory()))? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Ok(record) = self.read_record(&path, status) {
                records.push(record);
            }
        }
        Ok(records)
    }

    fn read_record(&self, path: &Path, status: ApprovalStatus) -> io::Result<ToolApprovalRecord> {
        let mut record: ToolApprovalRecord =
            serde_json::from_slice(&fs::read(path)?).map_err(json_error)?;
        record.status = status;
        Ok(record)
    }

    fn record_path(&self, status: ApprovalStatus, id: &str) -> PathBuf {
        self.root
            .join(status.directory())
            .join(format!("{id}.json"))
    }

    fn ensure_directories(&self) -> io::Result<()> {
        for status in all_statuses() {
            fs::create_dir_all(self.root.join(status.directory()))?;
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> io::Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid approval id",
        ))
    }
}

fn all_statuses() -> [ApprovalStatus; 5] {
    [
        ApprovalStatus::Pending,
        ApprovalStatus::Approved,
        ApprovalStatus::Denied,
        ApprovalStatus::Consumed,
        ApprovalStatus::Expired,
    ]
}

fn json_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, ToolApprovalStore) {
        let directory = tempdir().unwrap();
        let store = ToolApprovalStore::new(directory.path().to_path_buf());
        (directory, store)
    }

    #[test]
    fn approved_record_is_consumed_exactly_once() {
        let (_directory, store) = store();
        let arguments = json!({ "path": "notes.txt" });
        let pending = store
            .request("write_to_file", &arguments, ToolRisk::WorkspaceMutation)
            .unwrap();
        store.approve(&pending.id).unwrap();

        assert!(store
            .consume_matching("write_to_file", &arguments)
            .unwrap()
            .is_some());
        assert!(store
            .consume_matching("write_to_file", &arguments)
            .unwrap()
            .is_none());
        assert_eq!(
            store.get(&pending.id).unwrap().unwrap().status,
            ApprovalStatus::Consumed
        );
    }

    #[test]
    fn denied_or_different_arguments_cannot_be_consumed() {
        let (_directory, store) = store();
        let first = json!({ "path": "a.txt" });
        let pending = store
            .request("write_to_file", &first, ToolRisk::WorkspaceMutation)
            .unwrap();
        store.deny(&pending.id).unwrap();
        assert!(store
            .consume_matching("write_to_file", &first)
            .unwrap()
            .is_none());

        let second = store
            .request("write_to_file", &first, ToolRisk::WorkspaceMutation)
            .unwrap();
        store.approve(&second.id).unwrap();
        assert!(store
            .consume_matching("write_to_file", &json!({ "path": "b.txt" }))
            .unwrap()
            .is_none());
    }

    #[test]
    fn canonical_hash_ignores_object_key_order_and_reuses_pending_record() {
        let (_directory, store) = store();
        let first = store
            .request(
                "fixture",
                &json!({ "a": 1, "b": 2 }),
                ToolRisk::Unclassified,
            )
            .unwrap();
        let second = store
            .request(
                "fixture",
                &json!({ "b": 2, "a": 1 }),
                ToolRisk::Unclassified,
            )
            .unwrap();
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn rejects_path_traversal_ids() {
        let (_directory, store) = store();
        assert_eq!(
            store.get("../approved").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
