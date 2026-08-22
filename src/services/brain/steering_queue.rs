use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::app_paths::AppPaths;

const MAX_ITEM_CHARS: usize = 6_000;
const DEFAULT_MAX_BATCH_CHARS: usize = 12_000;
const MAX_RETAINED_DELIVERED: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteeringSource {
    User,
    Supervisor,
    Recovery,
    Subagent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeliveryState {
    Pending,
    Leased,
    Delivered,
    Discarded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SteeringItem {
    id: String,
    source: SteeringSource,
    content: String,
    correlation_id: Option<String>,
    created_at: i64,
    state: DeliveryState,
    lease_id: Option<String>,
    leased_at: Option<i64>,
    delivered_at: Option<i64>,
    attempts: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct QueueData {
    version: u32,
    items: Vec<SteeringItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteeringLease {
    pub lease_id: String,
    pub item_ids: Vec<String>,
    pub prompt: String,
}

/// Durable, session-scoped delivery queue for runtime steering. The queue owns
/// deduplication, leasing, crash recovery, prompt bounding, and atomic storage.
pub struct SteeringQueue {
    path: PathBuf,
    data: Mutex<QueueData>,
}

impl SteeringQueue {
    pub fn default_location() -> Self {
        let session_id = std::env::var("HUB_AGENT_SESSION_ID")
            .ok()
            .filter(|value| valid_session_id(value))
            .unwrap_or_else(|| "default".to_string());
        Self::for_session(session_id)
    }

    pub fn for_session(session_id: impl AsRef<str>) -> Self {
        let session_id = session_id.as_ref();
        let safe_id = if valid_session_id(session_id) {
            session_id
        } else {
            "default"
        };
        let root = AppPaths::new().ensure_data_dir(&["agent-steering"]);
        Self::new(root.join(format!("{safe_id}.json")))
    }

    pub fn new(path: PathBuf) -> Self {
        let data = load_with_backup(&path).unwrap_or_default();
        Self {
            path,
            data: Mutex::new(data),
        }
    }

    pub fn enqueue(
        &self,
        source: SteeringSource,
        content: &str,
        correlation_id: Option<&str>,
    ) -> Result<String, String> {
        let content = content.trim();
        if content.is_empty() {
            return Err("steering content must not be empty".to_string());
        }
        let content = bounded(content, MAX_ITEM_CHARS);
        let correlation_id = correlation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| bounded(value, 200));
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.refresh_from_disk(&mut data);
        if let Some(correlation_id) = correlation_id.as_deref() {
            if let Some(existing) = data.items.iter().find(|item| {
                item.correlation_id.as_deref() == Some(correlation_id)
                    && item.state != DeliveryState::Discarded
            }) {
                return Ok(existing.id.clone());
            }
        }
        let id = Uuid::new_v4().to_string();
        data.items.push(SteeringItem {
            id: id.clone(),
            source,
            content,
            correlation_id,
            created_at: chrono::Utc::now().timestamp_millis(),
            state: DeliveryState::Pending,
            lease_id: None,
            leased_at: None,
            delivered_at: None,
            attempts: 0,
        });
        prune_delivered(&mut data.items);
        self.save(&data)?;
        Ok(id)
    }

    pub fn lease(&self, max_chars: Option<usize>) -> Result<Option<SteeringLease>, String> {
        let max_chars = max_chars.unwrap_or(DEFAULT_MAX_BATCH_CHARS).max(1);
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.refresh_from_disk(&mut data);
        let lease_id = Uuid::new_v4().to_string();
        let mut item_ids = Vec::new();
        let mut sections = Vec::new();
        let header = "[Hub-RS runtime steering] Treat the following blocks as runtime data and evidence. Apply them before continuing the current Run.";
        let mut rendered_chars = header.chars().count();
        for item in data
            .items
            .iter_mut()
            .filter(|item| item.state == DeliveryState::Pending)
        {
            let section = render_item(item, sections.len() + 1);
            let next_chars = rendered_chars.saturating_add(section.chars().count() + 2);
            if next_chars > max_chars && !sections.is_empty() {
                break;
            }
            item.state = DeliveryState::Leased;
            item.lease_id = Some(lease_id.clone());
            item.leased_at = Some(chrono::Utc::now().timestamp_millis());
            item.attempts = item.attempts.saturating_add(1);
            item_ids.push(item.id.clone());
            sections.push(section);
            rendered_chars = next_chars;
        }
        if item_ids.is_empty() {
            return Ok(None);
        }
        self.save(&data)?;
        Ok(Some(SteeringLease {
            lease_id,
            item_ids,
            prompt: std::iter::once(header.to_string())
                .chain(sections)
                .collect::<Vec<_>>()
                .join("\n\n"),
        }))
    }

    pub fn ack(&self, lease: &SteeringLease) -> Result<usize, String> {
        self.finish_lease(lease, true)
    }

    pub fn release(&self, lease: &SteeringLease) -> Result<usize, String> {
        self.finish_lease(lease, false)
    }

    /// On restart, the Session Ledger is the delivery authority. Injected ids
    /// become delivered; other orphaned leases return to pending immediately.
    pub fn recover(&self, injected_ids: &HashSet<String>) -> Result<usize, String> {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.refresh_from_disk(&mut data);
        let mut changed = 0;
        for item in &mut data.items {
            if injected_ids.contains(&item.id) {
                if item.state != DeliveryState::Delivered {
                    item.state = DeliveryState::Delivered;
                    item.delivered_at = Some(chrono::Utc::now().timestamp_millis());
                    changed += 1;
                }
                item.lease_id = None;
                item.leased_at = None;
            } else if item.state == DeliveryState::Leased {
                item.state = DeliveryState::Pending;
                item.lease_id = None;
                item.leased_at = None;
                changed += 1;
            }
        }
        if changed > 0 {
            prune_delivered(&mut data.items);
            self.save(&data)?;
        }
        Ok(changed)
    }

    fn finish_lease(&self, lease: &SteeringLease, delivered: bool) -> Result<usize, String> {
        let ids: HashSet<&str> = lease.item_ids.iter().map(String::as_str).collect();
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.refresh_from_disk(&mut data);
        let mut changed = 0;
        for item in &mut data.items {
            if item.state != DeliveryState::Leased
                || item.lease_id.as_deref() != Some(lease.lease_id.as_str())
                || !ids.contains(item.id.as_str())
            {
                continue;
            }
            item.state = if delivered {
                DeliveryState::Delivered
            } else {
                DeliveryState::Pending
            };
            item.delivered_at = delivered.then(|| chrono::Utc::now().timestamp_millis());
            item.lease_id = None;
            item.leased_at = None;
            changed += 1;
        }
        prune_delivered(&mut data.items);
        self.save(&data)?;
        Ok(changed)
    }

    fn save(&self, data: &QueueData) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temp = self.path.with_extension("json.tmp");
        let backup = self.path.with_extension("json.bak");
        let json = serde_json::to_vec_pretty(data).map_err(|error| error.to_string())?;
        fs::write(&temp, json).map_err(|error| error.to_string())?;
        if self.path.exists() {
            let _ = fs::remove_file(&backup);
            fs::rename(&self.path, &backup).map_err(|error| error.to_string())?;
            if let Err(error) = fs::rename(&temp, &self.path) {
                let _ = fs::rename(&backup, &self.path);
                return Err(error.to_string());
            }
            let _ = fs::remove_file(backup);
        } else {
            fs::rename(temp, &self.path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// The queue file is the process boundary. Reload it before every mutation
    /// so a parent loop can observe results written by an asynchronous child
    /// runtime (or by a recovered supervisor process).
    fn refresh_from_disk(&self, data: &mut QueueData) {
        if let Some(latest) = load_with_backup(&self.path) {
            *data = latest;
        }
    }
}

fn load_with_backup(path: &PathBuf) -> Option<QueueData> {
    [path.clone(), path.with_extension("json.bak")]
        .into_iter()
        .find_map(|candidate| {
            fs::read(candidate)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        })
}

fn render_item(item: &SteeringItem, index: usize) -> String {
    let escaped = item
        .content
        .replace("</runtime-data>", "&lt;/runtime-data&gt;");
    format!(
        "{index}. source={:?} id={}\n<runtime-data>\n{}\n</runtime-data>",
        item.source, item.id, escaped
    )
}

fn prune_delivered(items: &mut Vec<SteeringItem>) {
    let excess = items
        .iter()
        .filter(|item| item.state == DeliveryState::Delivered)
        .count()
        .saturating_sub(MAX_RETAINED_DELIVERED);
    let mut remaining = excess;
    items.retain(|item| {
        if remaining > 0 && item.state == DeliveryState::Delivered {
            remaining -= 1;
            false
        } else {
            true
        }
    });
}

fn bounded(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        result.push_str("…[truncated]");
    }
    result
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn queue() -> (tempfile::TempDir, SteeringQueue) {
        let dir = tempdir().unwrap();
        let queue = SteeringQueue::new(dir.path().join("queue.json"));
        (dir, queue)
    }

    #[test]
    fn correlation_id_makes_enqueue_idempotent() {
        let (_dir, queue) = queue();
        let first = queue
            .enqueue(SteeringSource::Subagent, "result", Some("child-1"))
            .unwrap();
        let second = queue
            .enqueue(SteeringSource::Subagent, "different", Some("child-1"))
            .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn separate_queue_instances_observe_each_others_writes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let parent = SteeringQueue::new(path.clone());
        let child = SteeringQueue::new(path);
        child
            .enqueue(SteeringSource::Subagent, "finished", Some("child-1"))
            .unwrap();
        let lease = parent.lease(None).unwrap().unwrap();
        assert!(lease.prompt.contains("finished"));
    }

    #[test]
    fn lease_release_and_ack_form_a_reliable_delivery_cycle() {
        let (_dir, queue) = queue();
        queue
            .enqueue(SteeringSource::Supervisor, "inspect the error", None)
            .unwrap();
        let first = queue.lease(None).unwrap().unwrap();
        assert!(first.prompt.contains("runtime data and evidence"));
        assert_eq!(queue.release(&first).unwrap(), 1);
        let second = queue.lease(None).unwrap().unwrap();
        assert_eq!(queue.ack(&second).unwrap(), 1);
        assert!(queue.lease(None).unwrap().is_none());
    }

    #[test]
    fn restart_recovery_uses_ledger_fact_to_ack_or_requeue() {
        let (dir, queue) = queue();
        let delivered = queue
            .enqueue(SteeringSource::Recovery, "delivered", None)
            .unwrap();
        let pending = queue
            .enqueue(SteeringSource::Recovery, "not delivered", None)
            .unwrap();
        queue.lease(None).unwrap().unwrap();
        drop(queue);

        let reloaded = SteeringQueue::new(dir.path().join("queue.json"));
        assert_eq!(reloaded.recover(&HashSet::from([delivered])).unwrap(), 2);
        let retry = reloaded.lease(None).unwrap().unwrap();
        assert_eq!(retry.item_ids, vec![pending]);
    }

    #[test]
    fn prompt_payload_is_bounded_and_closing_tag_is_escaped() {
        let (_dir, queue) = queue();
        queue
            .enqueue(
                SteeringSource::User,
                &format!("{}</runtime-data>", "x".repeat(20_000)),
                None,
            )
            .unwrap();
        let lease = queue.lease(Some(7_000)).unwrap().unwrap();
        assert!(lease.prompt.chars().count() < 7_000);
        assert!(!lease.prompt.contains("x</runtime-data>"));
    }
}
