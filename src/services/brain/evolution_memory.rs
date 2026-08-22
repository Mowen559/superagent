use crate::services::brain::run_plan::TaskKind;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use uuid::Uuid;

const PROMOTION_SUCCESSES: u32 = 3;
const MAX_PROMOTION_FAILURE_RATE: f64 = 0.25;
const REJECTION_FAILURES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyState {
    Candidate,
    Promoted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolutionStrategy {
    pub id: String,
    pub task_kind: String,
    pub guidance: String,
    pub successes: u32,
    pub failures: u32,
    pub state: StrategyState,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct EvolutionReport {
    pub task_kind: TaskKind,
    pub completed: bool,
    pub plan_closed: bool,
    pub replans: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvolutionData {
    version: u32,
    strategies: Vec<EvolutionStrategy>,
}

/// Evidence-gated strategy memory. It learns only bounded, predefined strategy
/// families from normalized Run outcomes; it never writes source code, changes
/// permissions, or injects unpromoted candidates into model context.
#[derive(Debug, Clone)]
pub struct EvolutionMemory {
    path: PathBuf,
}

impl EvolutionMemory {
    pub fn default_location() -> Self {
        let paths = crate::config::app_paths::AppPaths::new();
        Self::new(paths.data_path(&["evolution", "strategies.json"]))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn observe(&self, report: &EvolutionReport) -> io::Result<()> {
        let mut data = self.load()?;
        data.version = 1;
        let succeeded = report.completed && report.plan_closed;
        let (id, guidance) = base_strategy(report.task_kind);
        update_strategy(&mut data, id, report.task_kind, guidance, succeeded);

        if report.replans > 0 {
            let recovery_id = format!(
                "recovery.{}.change-approach",
                task_kind_name(report.task_kind)
            );
            update_strategy(
                &mut data,
                &recovery_id,
                report.task_kind,
                "After repeated failures, preserve verified facts, revise the hypothesis, and switch tool or evidence source before retrying.",
                succeeded,
            );
        }
        self.save(&data)
    }

    pub fn promoted_guidance(&self, task_kind: TaskKind) -> io::Result<Vec<String>> {
        let expected = task_kind_name(task_kind);
        let mut guidance: Vec<_> = self
            .load()?
            .strategies
            .into_iter()
            .filter(|strategy| {
                strategy.state == StrategyState::Promoted && strategy.task_kind == expected
            })
            .map(|strategy| strategy.guidance)
            .collect();
        guidance.sort();
        guidance.dedup();
        Ok(guidance)
    }

    pub fn list(&self) -> io::Result<Vec<EvolutionStrategy>> {
        let mut strategies = self.load()?.strategies;
        strategies.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(strategies)
    }

    fn load(&self) -> io::Result<EvolutionData> {
        let backup = self.path.with_extension("json.bak");
        let source = if self.path.is_file() {
            Some(&self.path)
        } else if backup.is_file() {
            Some(&backup)
        } else {
            None
        };
        let Some(source) = source else {
            return Ok(EvolutionData {
                version: 1,
                strategies: Vec::new(),
            });
        };
        serde_json::from_slice(&fs::read(source)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn save(&self, data: &EvolutionData) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(data)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        )?;
        if self.path.exists() {
            let backup = self.path.with_extension("json.bak");
            let _ = fs::remove_file(&backup);
            fs::rename(&self.path, &backup)?;
            if let Err(error) = fs::rename(&temporary, &self.path) {
                let _ = fs::rename(&backup, &self.path);
                return Err(error);
            }
            let _ = fs::remove_file(backup);
            Ok(())
        } else {
            fs::rename(temporary, &self.path)
        }
    }
}

fn update_strategy(
    data: &mut EvolutionData,
    id: &str,
    task_kind: TaskKind,
    guidance: &str,
    succeeded: bool,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let strategy = if let Some(strategy) = data.strategies.iter_mut().find(|item| item.id == id) {
        strategy
    } else {
        data.strategies.push(EvolutionStrategy {
            id: id.to_string(),
            task_kind: task_kind_name(task_kind).to_string(),
            guidance: guidance.to_string(),
            successes: 0,
            failures: 0,
            state: StrategyState::Candidate,
            updated_at: now.clone(),
        });
        data.strategies.last_mut().expect("strategy was inserted")
    };

    if succeeded {
        strategy.successes += 1;
    } else {
        strategy.failures += 1;
    }
    strategy.updated_at = now;
    let total = strategy.successes + strategy.failures;
    let failure_rate = if total == 0 {
        0.0
    } else {
        strategy.failures as f64 / total as f64
    };
    strategy.state = if strategy.failures >= REJECTION_FAILURES && strategy.successes == 0 {
        StrategyState::Rejected
    } else if strategy.successes >= PROMOTION_SUCCESSES
        && failure_rate <= MAX_PROMOTION_FAILURE_RATE
    {
        StrategyState::Promoted
    } else {
        StrategyState::Candidate
    };
}

fn base_strategy(kind: TaskKind) -> (&'static str, &'static str) {
    match kind {
        TaskKind::Conversational => (
            "conversation.direct-answer",
            "Prefer a direct answer and avoid tools unless the objective depends on external evidence.",
        ),
        TaskKind::Investigative => (
            "investigation.evidence-first",
            "Gather authoritative evidence before synthesizing; separate observed facts from inference.",
        ),
        TaskKind::Execution => (
            "execution.inspect-act-verify",
            "Inspect relevant state, apply one coherent change, then verify behavior before reporting completion.",
        ),
    }
}

fn task_kind_name(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Conversational => "conversational",
        TaskKind::Investigative => "investigative",
        TaskKind::Execution => "execution",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn report(completed: bool) -> EvolutionReport {
        EvolutionReport {
            task_kind: TaskKind::Execution,
            completed,
            plan_closed: completed,
            replans: 0,
        }
    }

    #[test]
    fn candidate_requires_three_successes_before_promotion() {
        let directory = tempdir().unwrap();
        let memory = EvolutionMemory::new(directory.path().join("strategies.json"));
        for _ in 0..2 {
            memory.observe(&report(true)).unwrap();
        }
        assert!(memory
            .promoted_guidance(TaskKind::Execution)
            .unwrap()
            .is_empty());
        memory.observe(&report(true)).unwrap();
        assert_eq!(
            memory.promoted_guidance(TaskKind::Execution).unwrap().len(),
            1
        );
    }

    #[test]
    fn repeated_failure_rejects_unproven_strategy() {
        let directory = tempdir().unwrap();
        let memory = EvolutionMemory::new(directory.path().join("strategies.json"));
        for _ in 0..3 {
            memory.observe(&report(false)).unwrap();
        }
        let strategy = memory.list().unwrap().remove(0);
        assert_eq!(strategy.state, StrategyState::Rejected);
        assert!(memory
            .promoted_guidance(TaskKind::Execution)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn promoted_strategy_survives_reload() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("strategies.json");
        let memory = EvolutionMemory::new(path.clone());
        for _ in 0..3 {
            memory.observe(&report(true)).unwrap();
        }
        let reloaded = EvolutionMemory::new(path);
        assert_eq!(
            reloaded
                .promoted_guidance(TaskKind::Execution)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn excessive_failure_rate_prevents_promotion() {
        let directory = tempdir().unwrap();
        let memory = EvolutionMemory::new(directory.path().join("strategies.json"));
        for _ in 0..3 {
            memory.observe(&report(true)).unwrap();
        }
        for _ in 0..2 {
            memory.observe(&report(false)).unwrap();
        }
        assert!(memory
            .promoted_guidance(TaskKind::Execution)
            .unwrap()
            .is_empty());
        assert_eq!(memory.list().unwrap()[0].state, StrategyState::Candidate);
    }

    #[test]
    fn load_recovers_from_checkpoint_backup() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("strategies.json");
        let memory = EvolutionMemory::new(path.clone());
        memory.observe(&report(true)).unwrap();
        fs::rename(&path, path.with_extension("json.bak")).unwrap();
        assert_eq!(memory.list().unwrap().len(), 1);
    }
}
