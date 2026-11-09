use crate::services::brain::run_plan::TaskKind;
use crate::services::brain::events::SessionEvent;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use uuid::Uuid;

static EVOLUTION_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// Hermes 真实实战反思沉淀的经验单元 (Distilled Episodic Lesson)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistilledLesson {
    pub id: String,
    pub domain: String,             // 领域/模块标签 (如 "engine-service", "iam", "controller", "mybatis", "rust")
    pub trigger_scenario: String,   // 触发场景/任务特征
    pub trap_pattern: String,       // 踩坑反模式
    pub golden_solution: String,    // Historical field name; automatic records are observations, not certified fixes
    pub confidence_score: f32,      // 置信度 (0.0 - 1.0)
    pub usage_count: u32,           // 命中复用次数
    pub updated_at: String,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub recovery_evidence: Option<super::recovery_candidate::RecoveryCandidate>,
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
    #[serde(default, skip_serializing_if="Option::is_none")]
    source_project: Option<String>,
    strategies: Vec<EvolutionStrategy>,
    #[serde(default)]
    lessons: Vec<DistilledLesson>,
}

/// 证据驱动与轨迹自省进化的核心经验记忆体 (Hermes Evolution Engine)
#[derive(Debug, Clone)]
pub struct EvolutionMemory {
    workspace:Option<PathBuf>,
    config:Option<crate::config::runtime::RuntimeConfig>,
    path: PathBuf,
    project: Option<String>,
}

impl EvolutionMemory {
    pub fn scope_receipt(&self)->serde_json::Value {
        serde_json::json!({"sourceProject":self.project,"storageKind":if self.project.is_some(){"project_runtime"}else{"legacy_unscoped"},"storeTarget":self.path,"reliability":"unverified_historical_experience"})
    }
    pub fn default_location() -> Self {
        let paths = crate::config::app_paths::AppPaths::new();
        Self::new(paths.data_path(&["evolution", "strategies.json"]))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path,project:None,workspace:None,config:None }
    }

    pub fn for_project(config:&crate::config::runtime::RuntimeConfig,workspace:&std::path::Path)->Self {
        let (project,notes)=super::project_memory::managed_scope(config,workspace);
        Self {config:Some(config.clone()),workspace:Some(workspace.to_path_buf()),path:notes.with_file_name("evolution.json"),project:Some(super::project_memory::project_identity(&project))}
    }

    /// 记录并持久化一条提炼出的实战经验
    pub fn record_lesson(&self, lesson: DistilledLesson) -> io::Result<()> {
        self.record_lesson_confirmed(lesson).map(|_|())
    }

    pub fn record_lesson_confirmed(&self, lesson:DistilledLesson)->io::Result<DistilledLesson> {
        super::project_memory::reject_sensitive(&lesson).map_err(|_|io::Error::other("sensitive memory rejected"))?;
        self.with_write_lock(|| {
            let mut data=self.load()?;
            let expected=if let Some(existing)=data.lessons.iter_mut().find(|l|l.id==lesson.id || (lesson.recovery_evidence.is_none()&&l.recovery_evidence.is_none()&&l.trigger_scenario==lesson.trigger_scenario)) {
                existing.confidence_score=(existing.confidence_score*0.7+lesson.confidence_score*0.3).min(1.0);
                existing.usage_count+=1;existing.updated_at=chrono::Utc::now().to_rfc3339();existing.clone()
            } else {data.lessons.push(lesson.clone());lesson};
            self.save(&data)?;
            let saved=self.load()?.lessons.into_iter().find(|l|l.id==expected.id).ok_or_else(||io::Error::other("readback missing"))?;
            if serde_json::to_value(&saved).ok()!=serde_json::to_value(&expected).ok(){return Err(io::Error::other("readback mismatch"));}
            Ok(saved)
        })
    }

    fn with_write_lock<T>(&self,action:impl FnOnce()->io::Result<T>)->io::Result<T> {
        let _guard=EVOLUTION_WRITE_LOCK.lock().map_err(|_|io::Error::other("memory lock unavailable"))?;
        let parent=self.path.parent().unwrap_or(std::path::Path::new("."));fs::create_dir_all(parent)?;
        let mut name=self.path.file_name().unwrap_or_default().to_os_string();name.push(".lock");
        let file=std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(parent.join(name))?;
        let mut lock=fd_lock::RwLock::new(file);let _lock=lock.write()?;
        action()
    }

    /// 根据当前任务上下文，自适应检索关联度最高的实战经验 (Hermes Dynamic Recall)
    pub fn retrieve_relevant_lessons(&self,_query:&str)->io::Result<Vec<DistilledLesson>> {
        // Old lesson scores and success counts are inspection history, not
        // held-out evidence. Active experiences are projected by Governance.
        self.load()?;Ok(vec![])
    }

    pub fn workspace(&self)->Option<&std::path::Path>{self.workspace.as_deref()}
    pub fn project_scope(&self)->Option<(&crate::config::runtime::RuntimeConfig,&std::path::Path)>{Some((self.config.as_ref()?,self.workspace.as_deref()?))}

    /// 查阅所有已提炼沉淀的实战经验
    pub fn list_lessons(&self) -> io::Result<Vec<DistilledLesson>> {
        let mut lessons = self.load()?.lessons;
        lessons.sort_by(|a, b| b.confidence_score.partial_cmp(&a.confidence_score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(lessons)
    }

    /// 动态 ELO 置信度强化与衰减反馈 (Hermes 3 RLVR Feedback Loop)
    pub fn update_lesson_feedback(&self, lesson_id: &str, positive: bool) -> io::Result<Option<DistilledLesson>> {
        self.with_write_lock(|| {
        let mut data = self.load()?;
        let mut updated = None;
        let mut prune_index = None;

        for (idx, lesson) in data.lessons.iter_mut().enumerate() {
            if lesson.id == lesson_id {
                lesson.usage_count += 1;
                lesson.updated_at = chrono::Utc::now().to_rfc3339();
                if positive {
                    lesson.confidence_score = (lesson.confidence_score + 0.05).min(1.0);
                } else {
                    lesson.confidence_score = (lesson.confidence_score - 0.15).max(0.0);
                }

                // 自动淘汰机制：若置信度低于 0.40 且被使用过，予以自动淘汰剪枝
                if lesson.confidence_score < 0.40 && lesson.usage_count >= 2 {
                    prune_index = Some(idx);
                } else {
                    updated = Some(lesson.clone());
                }
                break;
            }
        }

        if let Some(idx) = prune_index {
            data.lessons.remove(idx);

        }

        self.save(&data)?;
        Ok(updated)
        })
    }

    /// 主动扫描并清理过期的低置信度经验条目
    pub fn prune_stale_lessons(&self) -> io::Result<usize> {
        self.with_write_lock(|| {
        let mut data = self.load()?;
        let initial_len = data.lessons.len();
        data.lessons.retain(|l| l.confidence_score >= 0.40 || l.usage_count < 2);
        let pruned_count = initial_len.saturating_sub(data.lessons.len());
        if pruned_count > 0 {
            self.save(&data)?;
        }
        Ok(pruned_count)
        })
    }

    /// 基于会话真实执行轨迹提炼实战经验 (Hermes Trajectory Distillation)
    pub fn distill_trajectory(
        &self,
        prompt: &str,
        events: &[SessionEvent],
        is_success: bool,
    ) -> Option<DistilledLesson> {
        if !is_success {return None;}
        // Prompt text, command output and edit contents are deliberately excluded.
        let _ = prompt;
        let evidence=super::recovery_candidate::extract(events,true).into_iter().next()?;
        if let Some(root)=self.workspace.as_ref() {
            if !super::recovery_candidate::is_current(&evidence,root) {return None;}
        }
        let lesson = DistilledLesson {
            id:format!("lesson-{}",Uuid::new_v4()),domain:"observed_recovery_candidate".into(),
            trigger_scenario:format!("Same-check recovery {}",evidence.failure.action_digest),
            trap_pattern:evidence.failure_reason.clone(),golden_solution:evidence.summary(),
            confidence_score:0.0,usage_count:0,updated_at:chrono::Utc::now().to_rfc3339(),
            recovery_evidence:Some(evidence),
        };
        super::project_memory::reject_sensitive(&lesson).ok()?;
        self.record_lesson(lesson.clone()).ok()?;
        self.list_lessons().ok()?.into_iter().find(|saved|saved.id==lesson.id)
    }

    pub fn observe(&self,report:&EvolutionReport)->io::Result<()> {
        self.observe_confirmed(report).map(|_|())
    }

    pub fn observe_confirmed(&self,report:&EvolutionReport)->io::Result<Vec<EvolutionStrategy>> {
        self.with_write_lock(|| {
            let mut data=self.load()?;data.version=1;
            let succeeded=report.completed&&report.plan_closed;
            let (id,guidance)=base_strategy(report.task_kind);
            let mut ids=vec![id.to_string()];update_strategy(&mut data,id,report.task_kind,guidance,succeeded);
            if report.replans>0 {
                let recovery_id=format!("recovery.{}.change-approach",task_kind_name(report.task_kind));
                update_strategy(&mut data,&recovery_id,report.task_kind,"After repeated failures, preserve verified facts, revise the hypothesis, and switch tool or evidence source before retrying.",succeeded);
                ids.push(recovery_id);
            }
            let expected:Vec<_>=data.strategies.iter().filter(|s|ids.contains(&s.id)).cloned().collect();
            self.save(&data)?;let saved=self.load()?;
            if expected.iter().any(|e|!saved.strategies.iter().any(|s|s.id==e.id&&serde_json::to_value(s).ok()==serde_json::to_value(e).ok())) {return Err(io::Error::other("readback mismatch"));}
            Ok(expected)
        })
    }

    pub fn promoted_guidance(&self,_task_kind:TaskKind)->io::Result<Vec<String>> {
        self.load()?;Ok(vec![])
    }

    pub fn list(&self) -> io::Result<Vec<EvolutionStrategy>> {
        let mut strategies = self.load()?.strategies;
        strategies.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(strategies)
    }

    fn load(&self) -> io::Result<EvolutionData> {
        if self.path.exists()&&!self.path.is_file(){return Err(io::Error::other("memory target is not a file"));}
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
                source_project:self.project.clone(),
                strategies: Vec::new(),
                lessons: Vec::new(),
            });
        };
        let data:EvolutionData=serde_json::from_slice(&fs::read(source)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if self.project.is_some() && data.source_project!=self.project {
            return Err(io::Error::new(io::ErrorKind::InvalidData,"evolution memory source does not match current project; legacy data was not adopted"));
        }
        Ok(data)
    }

    fn save(&self, data: &EvolutionData) -> io::Result<()> {
        super::project_memory::reject_sensitive(data).map_err(|_|io::Error::other("sensitive memory rejected"))?;
        if self.path.exists()&&!self.path.is_file(){return Err(io::Error::other("memory target is not a file"));}
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
    strategy.state = if strategy.failures >= REJECTION_FAILURES && strategy.successes == 0 {
        StrategyState::Rejected
    } else {StrategyState::Candidate};
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
    fn counters_never_replace_heldout_promotion() {
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
            0
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
    fn unvalidated_strategy_stays_inactive_after_reload() {
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
            0
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

    #[test]
    fn test_distill_and_retrieve_lesson() {
        let directory=tempdir().unwrap();let memory=EvolutionMemory::new(directory.path().join("evolution.json"));
        let events=super::super::recovery_candidate::tests::successful_fixture(false);
        let lesson=memory.distill_trajectory("sk-not-to-be-copied-1234567890",&events,true).unwrap();
        assert_eq!(lesson.domain,"observed_recovery_candidate");
        assert!(lesson.recovery_evidence.is_some());
        assert_eq!(memory.list_lessons().unwrap().len(),1);
        assert!(memory.retrieve_relevant_lessons("").unwrap().is_empty(),"candidate cannot become active guidance");
        assert!(!std::fs::read_to_string(directory.path().join("evolution.json")).unwrap().contains("sk-not-to-be-copied"));
        assert!(memory.distill_trajectory("repair",&events,false).is_none());
    }

    #[test]
    fn test_lesson_elo_reward_and_pruning() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("strategies.json");
        let memory = EvolutionMemory::new(path);

        let lesson = DistilledLesson {
            recovery_evidence:None,
            id: "les_elo_1".to_string(),
            domain: "mybatis".to_string(),
            trigger_scenario: "querying active users".to_string(),
            trap_pattern: "using raw query wrapper".to_string(),
            golden_solution: "use lambda query wrapper".to_string(),
            confidence_score: 0.50,
            usage_count: 1,
            updated_at: "2026-09-15T00:00:00Z".to_string(),
        };
        memory.record_lesson(lesson.clone()).unwrap();

        // 1. Positive reward
        let rewarded = memory.update_lesson_feedback("les_elo_1", true).unwrap().unwrap();
        assert!((rewarded.confidence_score - 0.55).abs() < f32::EPSILON);
        assert_eq!(rewarded.usage_count, 2);

        // 2. Penalty
        let penalized = memory.update_lesson_feedback("les_elo_1", false).unwrap().unwrap();
        assert!((penalized.confidence_score - 0.40).abs() < f32::EPSILON);

        // 3. Further penalty drops below 0.40 -> automatically pruned!
        let pruned = memory.update_lesson_feedback("les_elo_1", false).unwrap();
        assert!(pruned.is_none());

        // Verify lesson is no longer in list
        let lessons = memory.list_lessons().unwrap();
        assert!(lessons.is_empty());
    }
}
