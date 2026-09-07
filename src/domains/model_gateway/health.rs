use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::domains::model_gateway::routing::ProviderAttemptOutcome;

static GLOBAL_PROVIDER_HEALTH: OnceLock<ProviderHealthRuntime> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
pub struct ProviderHealthPolicy {
    pub failure_threshold: u32,
    pub base_cooldown: Duration,
    pub max_cooldown: Duration,
    pub max_entries: usize,
}

impl Default for ProviderHealthPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            base_cooldown: Duration::from_secs(15),
            max_cooldown: Duration::from_secs(5 * 60),
            max_entries: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAdmission {
    Allowed,
    Rejected { retry_after_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthSnapshot {
    pub provider_key: String,
    pub circuit: ProviderCircuitState,
    pub consecutive_failures: u32,
    pub total_successes: u64,
    pub total_recoverable_failures: u64,
    pub latency_ema_ms: Option<f64>,
    pub retry_after_ms: u64,
}

#[derive(Clone)]
pub struct ProviderHealthRuntime {
    policy: ProviderHealthPolicy,
    entries: Arc<Mutex<HashMap<String, ProviderHealthEntry>>>,
}

#[derive(Debug, Clone)]
struct ProviderHealthEntry {
    circuit: ProviderCircuitState,
    consecutive_failures: u32,
    total_successes: u64,
    total_recoverable_failures: u64,
    latency_ema_ms: Option<f64>,
    open_count: u32,
    open_until: Option<Instant>,
    probe_in_flight: bool,
    last_touched: Instant,
}

impl ProviderHealthEntry {
    fn new(now: Instant) -> Self {
        Self {
            circuit: ProviderCircuitState::Closed,
            consecutive_failures: 0,
            total_successes: 0,
            total_recoverable_failures: 0,
            latency_ema_ms: None,
            open_count: 0,
            open_until: None,
            probe_in_flight: false,
            last_touched: now,
        }
    }
}

impl Default for ProviderHealthRuntime {
    fn default() -> Self {
        Self::with_policy(ProviderHealthPolicy::default())
    }
}

impl ProviderHealthRuntime {
    pub fn global() -> &'static Self {
        GLOBAL_PROVIDER_HEALTH.get_or_init(Self::default)
    }

    pub fn with_policy(policy: ProviderHealthPolicy) -> Self {
        let policy = ProviderHealthPolicy {
            failure_threshold: policy.failure_threshold.max(1),
            base_cooldown: policy.base_cooldown.max(Duration::from_millis(1)),
            max_cooldown: policy.max_cooldown.max(policy.base_cooldown),
            max_entries: policy.max_entries.max(1),
        };
        Self {
            policy,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn begin_attempt(&self, provider_key: &str) -> ProviderAdmission {
        self.begin_attempt_at(provider_key, Instant::now())
    }

    pub fn finish_attempt(
        &self,
        provider_key: &str,
        outcome: ProviderAttemptOutcome,
        duration_ms: u64,
    ) {
        self.finish_attempt_at(provider_key, outcome, duration_ms, Instant::now());
    }

    /// Release a half-open Probe when the request never reached the Provider.
    pub fn cancel_attempt(&self, provider_key: &str) {
        let now = Instant::now();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(provider_key) {
            entry.last_touched = now;
            entry.probe_in_flight = false;
            if entry.circuit == ProviderCircuitState::HalfOpen {
                entry.circuit = ProviderCircuitState::Open;
                entry.open_until = Some(now);
            }
        }
    }

    pub fn snapshot(&self, provider_key: &str) -> Option<ProviderHealthSnapshot> {
        self.snapshot_at(provider_key, Instant::now())
    }

    pub fn snapshots(&self) -> Vec<ProviderHealthSnapshot> {
        let now = Instant::now();
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut snapshots = entries
            .iter()
            .map(|(key, entry)| snapshot_entry(key, entry, now))
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.provider_key.cmp(&right.provider_key));
        snapshots
    }

    pub fn reset(&self, provider_key: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(provider_key)
            .is_some()
    }

    fn begin_attempt_at(&self, provider_key: &str, now: Instant) -> ProviderAdmission {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ensure_capacity(&mut entries, self.policy.max_entries, provider_key);
        let entry = entries
            .entry(provider_key.to_string())
            .or_insert_with(|| ProviderHealthEntry::new(now));
        entry.last_touched = now;

        match entry.circuit {
            ProviderCircuitState::Closed => ProviderAdmission::Allowed,
            ProviderCircuitState::Open => {
                let open_until = entry.open_until.unwrap_or(now);
                if now < open_until {
                    ProviderAdmission::Rejected {
                        retry_after_ms: duration_ms(open_until.duration_since(now)),
                    }
                } else if entry.probe_in_flight {
                    ProviderAdmission::Rejected { retry_after_ms: 1 }
                } else {
                    entry.circuit = ProviderCircuitState::HalfOpen;
                    entry.probe_in_flight = true;
                    ProviderAdmission::Allowed
                }
            }
            ProviderCircuitState::HalfOpen => {
                if entry.probe_in_flight {
                    ProviderAdmission::Rejected { retry_after_ms: 1 }
                } else {
                    entry.probe_in_flight = true;
                    ProviderAdmission::Allowed
                }
            }
        }
    }

    fn finish_attempt_at(
        &self,
        provider_key: &str,
        outcome: ProviderAttemptOutcome,
        duration_ms: u64,
        now: Instant,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ensure_capacity(&mut entries, self.policy.max_entries, provider_key);
        let entry = entries
            .entry(provider_key.to_string())
            .or_insert_with(|| ProviderHealthEntry::new(now));
        entry.last_touched = now;
        entry.probe_in_flight = false;

        match outcome {
            ProviderAttemptOutcome::Succeeded => {
                entry.total_successes = entry.total_successes.saturating_add(1);
                entry.consecutive_failures = 0;
                entry.open_count = 0;
                entry.open_until = None;
                entry.circuit = ProviderCircuitState::Closed;
                entry.latency_ema_ms = Some(match entry.latency_ema_ms {
                    Some(previous) => previous * 0.8 + duration_ms as f64 * 0.2,
                    None => duration_ms as f64,
                });
            }
            ProviderAttemptOutcome::Authentication => {
                entry.total_recoverable_failures =
                    entry.total_recoverable_failures.saturating_add(1);
                entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
                open_circuit(entry, &self.policy, now);
            }
            ProviderAttemptOutcome::RateLimited | ProviderAttemptOutcome::Transient => {
                let was_probe = entry.circuit == ProviderCircuitState::HalfOpen;
                entry.total_recoverable_failures =
                    entry.total_recoverable_failures.saturating_add(1);
                entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
                if was_probe || entry.consecutive_failures >= self.policy.failure_threshold {
                    open_circuit(entry, &self.policy, now);
                } else {
                    entry.circuit = ProviderCircuitState::Closed;
                }
            }
            ProviderAttemptOutcome::CircuitOpen => {}
            ProviderAttemptOutcome::ContextOverflow | ProviderAttemptOutcome::Permanent => {
                if entry.circuit == ProviderCircuitState::HalfOpen {
                    entry.circuit = ProviderCircuitState::Open;
                    entry.open_until = Some(now);
                }
            }
        }
    }

    fn snapshot_at(&self, provider_key: &str, now: Instant) -> Option<ProviderHealthSnapshot> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        entries
            .get(provider_key)
            .map(|entry| snapshot_entry(provider_key, entry, now))
    }
}

fn open_circuit(entry: &mut ProviderHealthEntry, policy: &ProviderHealthPolicy, now: Instant) {
    entry.open_count = entry.open_count.saturating_add(1);
    let exponent = entry.open_count.saturating_sub(1).min(20);
    let multiplier = 2_u32.saturating_pow(exponent);
    let cooldown = policy
        .base_cooldown
        .saturating_mul(multiplier)
        .min(policy.max_cooldown);
    entry.circuit = ProviderCircuitState::Open;
    entry.open_until = Some(now + cooldown);
}

fn snapshot_entry(
    provider_key: &str,
    entry: &ProviderHealthEntry,
    now: Instant,
) -> ProviderHealthSnapshot {
    let retry_after_ms = entry
        .open_until
        .filter(|deadline| *deadline > now)
        .map(|deadline| duration_ms(deadline.duration_since(now)))
        .unwrap_or(0);
    ProviderHealthSnapshot {
        provider_key: provider_key.to_string(),
        circuit: entry.circuit,
        consecutive_failures: entry.consecutive_failures,
        total_successes: entry.total_successes,
        total_recoverable_failures: entry.total_recoverable_failures,
        latency_ema_ms: entry.latency_ema_ms,
        retry_after_ms,
    }
}

fn ensure_capacity(
    entries: &mut HashMap<String, ProviderHealthEntry>,
    max_entries: usize,
    incoming_key: &str,
) {
    if entries.contains_key(incoming_key) || entries.len() < max_entries {
        return;
    }
    if let Some(oldest) = entries
        .iter()
        .min_by_key(|(_, entry)| entry.last_touched)
        .map(|(key, _)| key.clone())
    {
        entries.remove(&oldest);
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> ProviderHealthRuntime {
        ProviderHealthRuntime::with_policy(ProviderHealthPolicy {
            failure_threshold: 3,
            base_cooldown: Duration::from_secs(10),
            max_cooldown: Duration::from_secs(60),
            max_entries: 2,
        })
    }

    #[test]
    fn opens_after_threshold_and_allows_exactly_one_half_open_probe() {
        let runtime = runtime();
        let now = Instant::now();
        for offset in 0..3 {
            assert_eq!(
                runtime.begin_attempt_at("provider", now + Duration::from_millis(offset)),
                ProviderAdmission::Allowed
            );
            runtime.finish_attempt_at(
                "provider",
                ProviderAttemptOutcome::Transient,
                10,
                now + Duration::from_millis(offset),
            );
        }
        assert!(matches!(
            runtime.begin_attempt_at("provider", now + Duration::from_secs(1)),
            ProviderAdmission::Rejected { .. }
        ));

        let probe_time = now + Duration::from_secs(11);
        assert_eq!(
            runtime.begin_attempt_at("provider", probe_time),
            ProviderAdmission::Allowed
        );
        assert!(matches!(
            runtime.begin_attempt_at("provider", probe_time),
            ProviderAdmission::Rejected { .. }
        ));
        runtime.finish_attempt_at(
            "provider",
            ProviderAttemptOutcome::Succeeded,
            50,
            probe_time,
        );
        assert_eq!(
            runtime.snapshot_at("provider", probe_time).unwrap().circuit,
            ProviderCircuitState::Closed
        );
    }

    #[test]
    fn authentication_opens_immediately_but_request_errors_do_not_poison_health() {
        let runtime = runtime();
        let now = Instant::now();
        runtime.finish_attempt_at("auth", ProviderAttemptOutcome::Authentication, 1, now);
        assert_eq!(
            runtime.snapshot_at("auth", now).unwrap().circuit,
            ProviderCircuitState::Open
        );

        runtime.finish_attempt_at("request", ProviderAttemptOutcome::Permanent, 1, now);
        runtime.finish_attempt_at("request", ProviderAttemptOutcome::ContextOverflow, 1, now);
        let snapshot = runtime.snapshot_at("request", now).unwrap();
        assert_eq!(snapshot.circuit, ProviderCircuitState::Closed);
        assert_eq!(snapshot.consecutive_failures, 0);
        assert_eq!(snapshot.total_recoverable_failures, 0);
    }

    #[test]
    fn health_table_is_bounded_and_success_tracks_latency_ema() {
        let runtime = runtime();
        let now = Instant::now();
        runtime.finish_attempt_at("old", ProviderAttemptOutcome::Succeeded, 100, now);
        runtime.finish_attempt_at(
            "kept",
            ProviderAttemptOutcome::Succeeded,
            200,
            now + Duration::from_secs(1),
        );
        runtime.finish_attempt_at(
            "new",
            ProviderAttemptOutcome::Succeeded,
            300,
            now + Duration::from_secs(2),
        );
        assert!(runtime.snapshot_at("old", now).is_none());
        assert_eq!(runtime.snapshots().len(), 2);
        assert_eq!(
            runtime.snapshot_at("new", now).unwrap().latency_ema_ms,
            Some(300.0)
        );
    }
}
