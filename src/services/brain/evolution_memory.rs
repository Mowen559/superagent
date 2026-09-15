use crate::services::brain::run_plan::TaskKind;
use crate::services::brain::events::SessionEvent;
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

/// Hermes 真实实战反思沉淀的经验单元 (Distilled Episodic Lesson)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistilledLesson {
    pub id: String,
    pub domain: String,             // 领域/模块标签 (如 "engine-service", "iam", "controller", "mybatis", "rust")
    pub trigger_scenario: String,   // 触发场景/任务特征
    pub trap_pattern: String,       // 踩坑反模式
    pub golden_solution: String,    // 经过工程验证的黄金解决方案
    pub confidence_score: f32,      // 置信度 (0.0 - 1.0)
    pub usage_count: u32,           // 命中复用次数
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
    #[serde(default)]
    lessons: Vec<DistilledLesson>,
}

/// 证据驱动与轨迹自省进化的核心经验记忆体 (Hermes Evolution Engine)
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

    /// 记录并持久化一条提炼出的实战经验
    pub fn record_lesson(&self, lesson: DistilledLesson) -> io::Result<()> {
        let mut data = self.load()?;
        if let Some(existing) = data.lessons.iter_mut().find(|l| l.id == lesson.id || l.trigger_scenario == lesson.trigger_scenario) {
            existing.confidence_score = (existing.confidence_score * 0.7 + lesson.confidence_score * 0.3).min(1.0);
            existing.usage_count += 1;
            existing.updated_at = chrono::Utc::now().to_rfc3339();
        } else {
            data.lessons.push(lesson);
        }
        self.save(&data)
    }

    /// 根据当前任务上下文，自适应检索关联度最高的实战经验 (Hermes Dynamic Recall)
    pub fn retrieve_relevant_lessons(&self, query: &str) -> io::Result<Vec<DistilledLesson>> {
        let q_lower = query.to_lowercase();
        let words: Vec<&str> = q_lower.split_whitespace().filter(|w| w.len() > 2).collect();

        let data = self.load()?;
        let mut scored: Vec<(f32, DistilledLesson)> = data.lessons
            .into_iter()
            .filter_map(|lesson| {
                let mut score = lesson.confidence_score;
                let text = format!("{} {} {} {}", lesson.domain, lesson.trigger_scenario, lesson.trap_pattern, lesson.golden_solution).to_lowercase();
                let mut matched_words = 0;
                for w in &words {
                    if text.contains(w) {
                        matched_words += 1;
                        score += 0.5;
                    }
                }
                if matched_words > 0 || words.is_empty() {
                    Some((score, lesson))
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(3).map(|(_, l)| l).collect())
    }

    /// 查阅所有已提炼沉淀的实战经验
    pub fn list_lessons(&self) -> io::Result<Vec<DistilledLesson>> {
        let mut lessons = self.load()?.lessons;
        lessons.sort_by(|a, b| b.confidence_score.partial_cmp(&a.confidence_score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(lessons)
    }

    /// 动态 ELO 置信度强化与衰减反馈 (Hermes 3 RLVR Feedback Loop)
    pub fn update_lesson_feedback(&self, lesson_id: &str, positive: bool) -> io::Result<Option<DistilledLesson>> {
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
            let pruned = data.lessons.remove(idx);
            eprintln!(
                "\x1b[33m[Evolution Pruning] Pruned sub-par lesson '{}' (Confidence: {:.2} < 0.40)\x1b[0m",
                pruned.id, pruned.confidence_score
            );
        }

        self.save(&data)?;
        Ok(updated)
    }

    /// 主动扫描并清理过期的低置信度经验条目
    pub fn prune_stale_lessons(&self) -> io::Result<usize> {
        let mut data = self.load()?;
        let initial_len = data.lessons.len();
        data.lessons.retain(|l| l.confidence_score >= 0.40 || l.usage_count < 2);
        let pruned_count = initial_len.saturating_sub(data.lessons.len());
        if pruned_count > 0 {
            self.save(&data)?;
        }
        Ok(pruned_count)
    }

    /// 基于会话真实执行轨迹提炼实战经验 (Hermes Trajectory Distillation)
    pub fn distill_trajectory(
        &self,
        prompt: &str,
        events: &[SessionEvent],
        is_success: bool,
    ) -> Option<DistilledLesson> {
        if !is_success || events.is_empty() {
            return None;
        }

        // 分析事件序列中是否存在错误纠偏（从失败到最终成功是最佳经验源）
        let mut had_error = false;
        let mut error_snippet = String::new();
        let mut corrective_tools = Vec::new();

        for event in events {
            match event {
                SessionEvent::ToolResult { result, .. } => {
                    let res_str = result.to_string();
                    if res_str.contains("error")
                        || res_str.contains("failed")
                        || res_str.contains("TargetContent not found")
                        || res_str.contains("timed out")
                        || res_str.contains("cannot find")
                    {
                        had_error = true;
                        if error_snippet.is_empty() {
                            error_snippet = res_str.chars().take(120).collect();
                        }
                    }
                }
                SessionEvent::ToolCall { name, .. } => {
                    if had_error {
                        corrective_tools.push(name.clone());
                    }
                }
                _ => {}
            }
        }

        if !had_error {
            return None; // 顺畅一次性完成无需记录坑点
        }

        let domain = if prompt.contains("controller") || prompt.contains("api") {
            "controller".to_string()
        } else if prompt.contains("job") || prompt.contains("spider") || prompt.contains("engine") {
            "engine-service".to_string()
        } else if prompt.contains("user") || prompt.contains("iam") || prompt.contains("auth") {
            "iam-service".to_string()
        } else if prompt.contains("mybatis") || prompt.contains("sql") || prompt.contains("repository") {
            "mybatis".to_string()
        } else if prompt.contains("ui") || prompt.contains("page") || prompt.contains("frontend") {
            "frontend".to_string()
        } else {
            "general".to_string()
        };

        let lesson = DistilledLesson {
            id: format!("lesson-{}", &Uuid::new_v4().to_string()[..8]),
            domain,
            trigger_scenario: prompt.chars().take(100).collect(),
            trap_pattern: format!("Encountered roadblock: {}", error_snippet.trim()),
            golden_solution: format!(
                "Pivoted strategy and resolved by leveraging tools: {:?}",
                corrective_tools
            ),
            confidence_score: 0.85,
            usage_count: 1,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        let _ = self.record_lesson(lesson.clone());
        Some(lesson)
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
                lessons: Vec::new(),
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

    #[test]
    fn test_distill_and_retrieve_lesson() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("strategies.json");
        let memory = EvolutionMemory::new(path);

        let events = vec![
            SessionEvent::UserInput("重构 spider job controller 接口".to_string()),
            SessionEvent::ToolCall {
                id: "1".to_string(),
                name: "replace_in_file".to_string(),
                arguments: serde_json::json!({ "file": "JobController.java" }),
            },
            SessionEvent::ToolResult {
                id: "1".to_string(),
                result: serde_json::json!({ "error": "TargetContent not found" }),
            },
            SessionEvent::ToolCall {
                id: "2".to_string(),
                name: "view_outline".to_string(),
                arguments: serde_json::json!({ "file": "JobController.java" }),
            },
            SessionEvent::ToolResult {
                id: "2".to_string(),
                result: serde_json::json!({ "outline": "public Result<Long> createJob" }),
            },
        ];

        let distilled = memory.distill_trajectory("重构 spider job controller 接口", &events, true);
        assert!(distilled.is_some());
        let lesson = distilled.unwrap();
        assert_eq!(lesson.domain, "controller");
        assert!(lesson.trap_pattern.contains("TargetContent not found"));

        let retrieved = memory.retrieve_relevant_lessons("job controller").unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].id, lesson.id);
    }

    #[test]
    fn test_lesson_elo_reward_and_pruning() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("strategies.json");
        let memory = EvolutionMemory::new(path);

        let lesson = DistilledLesson {
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
