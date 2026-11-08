//! Runtime-owned project notes. Recall is historical evidence, never authority.
use crate::{
    config::runtime::RuntimeConfig,
    models::memory::MemoryContext,
    services::supermemory_store::{ScoredMemoryRecord, SupermemoryStore},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

pub fn project_identity(project: &Path) -> String {
    std::fs::canonicalize(project)
        .unwrap_or_else(|_| project.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// One identity and managed note destination for explicit and automatic notes.
pub fn managed_scope(config: &RuntimeConfig, workspace: &Path) -> (PathBuf, PathBuf) {
    let origin=crate::infrastructure::fs_path::resolve_origin_project_from_baseline(workspace,&[]);
    let project=std::fs::canonicalize(&origin).unwrap_or(origin);
    let path=config.project_notes_path(&project);
    (project,path)
}

fn bound_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("source_file must be a project-relative file without traversal".into());
    }
    let root = std::fs::canonicalize(root).map_err(|_| "Source workspace unavailable")?;
    let file = std::fs::canonicalize(root.join(path)).map_err(|_| "Source file unavailable")?;
    if !file.starts_with(&root)
        || !file.is_file()
        || std::fs::metadata(&file)
            .map_err(|_| "Source metadata unavailable")?
            .len()
            > 2 * 1024 * 1024
    {
        return Err("Source file is outside workspace, not a file, or exceeds 2 MiB".into());
    }
    Ok(file)
}

pub fn file_hash(root: &Path, relative: &str) -> Result<String, String> {
    let bytes = std::fs::read(bound_file(root, relative)?).map_err(|_| "Source file unreadable")?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub fn new_context(
    project: &Path,
    workspace: &Path,
    source_file: Option<&str>,
    expires_at: Option<&str>,
) -> Result<MemoryContext, String> {
    let context = MemoryContext {
        project: project_identity(project),
        source: "native_super_memory".into(),
        kind: "project_note".into(),
        confidence: 0.5,
        state: "candidate".into(),
        revision: 1,
        source_file: source_file.map(str::to_owned),
        source_hash: source_file
            .map(|file| file_hash(workspace, file))
            .transpose()?,
        expires_at: expires_at.map(str::to_owned),
    };
    validate_context(&context)?;
    Ok(context)
}

pub fn validate_context(context: &MemoryContext) -> Result<(), String> {
    if context.project.is_empty()
        || context.source.is_empty()
        || context.kind.is_empty()
        || !context.confidence.is_finite()
        || !(0.0..=1.0).contains(&context.confidence)
        || !matches!(context.state.as_str(), "candidate" | "active" | "revoked")
        || context.source_file.is_some() != context.source_hash.is_some()
    {
        return Err("Invalid memory context".into());
    }
    if let Some(expiry) = &context.expires_at {
        let expiry = chrono::DateTime::parse_from_rfc3339(expiry)
            .map_err(|_| "expires_at must be RFC3339")?;
        if expiry <= chrono::Utc::now() {
            return Err("expires_at must be in the future".into());
        }
    }
    Ok(())
}

pub fn context_eligible(context: Option<&MemoryContext>, scope: Option<(&Path, &Path)>) -> bool {
    let Some(context) = context else {
        return true;
    };
    if !matches!(context.state.as_str(), "candidate" | "active")
        || !context.confidence.is_finite()
        || !(0.0..=1.0).contains(&context.confidence)
    {
        return false;
    }
    if let Some(expiry) = &context.expires_at {
        if chrono::DateTime::parse_from_rfc3339(expiry)
            .map_or(true, |date| date <= chrono::Utc::now())
        {
            return false;
        }
    }
    let Some((project, source_root)) = scope else {
        return false;
    };
    if context.project != project_identity(project) {
        return false;
    }
    match (&context.source_file, &context.source_hash) {
        (None, None) => true,
        (Some(file), Some(hash)) => {
            file_hash(source_root, file).is_ok_and(|current| &current == hash)
        }
        _ => false,
    }
}

pub fn reliability(context: Option<&MemoryContext>) -> &'static str {
    if context.is_some() {
        "unverified_project_note"
    } else {
        "legacy_unverified"
    }
}

pub struct MemoryRecall {
    pub hits: Vec<ScoredMemoryRecord>,
    pub warnings: Vec<String>,
}

/// Managed keys, including tombstones, shadow legacy copies. Never merge global notes.
pub fn recall(
    config: &RuntimeConfig,
    workspace: &Path,
    query: Option<&str>,
    limit: usize,
) -> MemoryRecall {
    let project =
        crate::infrastructure::fs_path::resolve_origin_project_from_baseline(workspace, &[]);
    let mut paths = vec![
        config.project_notes_path(&project),
        config.legacy_project_notes_path(&project),
        config.supermemory_store_path(Some(&workspace.to_string_lossy())),
    ];
    let origin = config.supermemory_store_path(Some(&project.to_string_lossy()));
    if !paths.contains(&origin) {
        paths.push(origin);
    }
    let mut seen = HashSet::new();
    let mut result = MemoryRecall {
        hits: Vec::new(),
        warnings: Vec::new(),
    };
    for path in paths {
        let path_is_legacy_managed=path==config.legacy_project_notes_path(&project);
        let store = SupermemoryStore::from_path(path);
        let data = match store.load_result() {
            Ok(data) => data,
            Err(error) => {
                result.warnings.push(error);
                continue;
            }
        };
        let reserved = data
            .memories
            .iter()
            .map(|m| m.id.clone())
            .collect::<Vec<_>>();
        let candidates = if let Some(query) = query {
            match store.search_project_context_result(query, 100, &project, workspace) {
                Ok(hits) => hits,
                Err(error) => {
                    result.warnings.push(error);
                    Vec::new()
                }
            }
        } else {
            data.memories
                .into_iter()
                .filter(|m| {
                    context_eligible(data.memory_context.get(&m.id), Some((&project, workspace)))
                })
                .map(|record| ScoredMemoryRecord {
                    context: data.memory_context.get(&record.id).cloned(),
                    record,
                    score: 0.0,
                    keyword_score: 0.0,
                    vector_score: None,
                })
                .collect()
        };
        result.hits.extend(
            candidates
                .into_iter()
                .map(|mut hit| {if path_is_legacy_managed {hit.record.source_label=format!("legacy_managed_project_notes:{}",hit.record.source_label);} hit})
                .filter(|hit| !seen.contains(&hit.record.id)),
        );
        seen.extend(reserved);
    }
    if query.is_some() {
        result.hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.record.id.cmp(&b.record.id))
        });
    } else {
        result
            .hits
            .sort_by(|a, b| b.record.updated_at.cmp(&a.record.updated_at));
    }
    result.hits.truncate(limit);
    result
}

/// Deliberately bounded canary protection, not a general credential detector.
pub fn reject_sensitive(value: &impl Serialize) -> Result<(), String> {
    lazy_static::lazy_static! {
        static ref SECRET: regex::Regex = regex::Regex::new(r"(?i)(?:\bsk-[a-z0-9_-]{16,}|\bbearer\s+[a-z0-9._~+/-]{16,}|[?&](?:token|access_token|api_key|apikey|signature|sig|authorization|auth)=[^&\s\x22<>]{8,})").expect("constant regex");
    }
    fn contains(value: &serde_json::Value, pattern: &regex::Regex) -> bool {
        match value {
            serde_json::Value::String(s) => pattern.is_match(s),
            serde_json::Value::Array(items) => items.iter().any(|item| contains(item, pattern)),
            serde_json::Value::Object(items) => items
                .iter()
                .any(|(key, item)| pattern.is_match(key) || contains(item, pattern)),
            _ => false,
        }
    }
    if contains(
        &serde_json::to_value(value).map_err(|_| "Unable to validate memory")?,
        &SECRET,
    ) {
        return Err("Memory rejected: recognizable credential or signed URL; remove sensitive values before storing".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn long_natural_query_projects_complete_notes_before_optional_inventory() {
        let root=tempfile::tempdir().unwrap();let config=config(root.path());
        let project=root.path().join("project");std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("README.md"),"authoritative conventions\n".repeat(200)).unwrap();
        for i in 0..40 {std::fs::create_dir(project.join(format!("module{i}"))).unwrap();}
        let store=SupermemoryStore::from_path(config.project_notes_path(&project));
        store.write_context_memory(record("ticket_reference","unique note value"),new_context(&project,&project,None,None).unwrap()).unwrap();
        let query="只读查询：本项目 ticket_reference 约定是什么？说明实际来源与验证范围。如果找不到就说找不到；不要猜测或写入记忆。";
        assert_eq!(recall(&config,&project,Some(query),5).hits.len(),1);
        let prelude=crate::services::brain::run_prelude::RunPreludeRuntime::new(config.clone(),project.to_string_lossy()).prepare(query);
        assert_eq!(prelude.project_note_evidence_count(),1);
        assert!(prelude.prompt_projection().contains("unique note value"));
        assert!(prelude.prompt_projection().contains("</knowledge-evidence>"));
        assert!(prelude.prompt_projection().chars().count()<=6000);
        let near=recall(&config,&project,Some("这是一个很长的问题，希望核对 ticket_reference_backup 的正式记录，请给出实际已有数据的来源和有效范围，不要用近似标识代替，也不要猜测。"),5);
        assert!(near.hits.iter().all(|hit|hit.keyword_score<1.0));
        store.revoke_context_memory("ticket_reference").unwrap();
        assert_eq!(crate::services::brain::run_prelude::RunPreludeRuntime::new(config,project.to_string_lossy()).prepare(query).project_note_evidence_count(),0);
    }
    #[test]
    fn partial_note_text_cannot_count_as_complete_automatic_evidence() {
        let root=tempfile::tempdir().unwrap();let config=config(root.path());
        let store=SupermemoryStore::from_path(config.project_notes_path(root.path()));
        store.write_context_memory(record("long_note",&"x".repeat(3000)),new_context(root.path(),root.path(),None,None).unwrap()).unwrap();
        let prelude=crate::services::brain::run_prelude::RunPreludeRuntime::new(config,root.path().to_string_lossy()).prepare("long_note");
        assert!(prelude.prompt_projection().contains("truncated=\"true\""));
        assert_eq!(prelude.project_note_evidence_count(),0);
    }
    use super::*;
    use crate::models::memory::{SupermemoryRecord, SupermemoryStoreData};
    fn config(root: &Path) -> RuntimeConfig {
        RuntimeConfig {
            workspace_root: root.into(),
            understand_anything_root: root.join("understand"),
            shadow_git_root: root.join("cache"),
            project_root: root.into(),
            projects_root: root.join("projects"),
            codebase_memory_cache_root: root.join("graphs"),
            codebase_memory_bin: "missing".into(),
            rtk_bin: "unused".into(),
            sso_api_url: String::new(),
        }
    }
    #[test]
    fn automatic_and_explicit_notes_share_origin_scope_and_preserve_legacy_files() {
        use crate::domains::agent_cognitive::plan_reflection::ReflectionEngine;
        let root=tempfile::tempdir().unwrap();let cfg=config(root.path());
        let origin=root.path().join("origin");std::fs::create_dir(&origin).unwrap();
        let mut runs=Vec::new();
        for name in ["one","two"] {let parent=root.path().join(name);let workspace=parent.join("workspace");std::fs::create_dir_all(&workspace).unwrap();std::fs::write(parent.join("baseline.json"),serde_json::json!({"projectPath":origin}).to_string()).unwrap();runs.push(workspace);}
        let (project,path)=managed_scope(&cfg,&runs[0]);let store=SupermemoryStore::from_path(&path);
        store.write_context_memory(record("explicit-note","scope_sentinel"),new_context(&project,&runs[0],None,None).unwrap()).unwrap();
        let legacy=cfg.legacy_project_notes_path(&project);let old=SupermemoryStore::from_path(&legacy);
        old.write_context_memory(record("old-note","legacy_sentinel"),new_context(&project,&origin,None,None).unwrap()).unwrap();let original=std::fs::read(&legacy).unwrap();
        let events=crate::services::brain::recovery_candidate::tests::successful_fixture(true);
        assert_eq!(ReflectionEngine::distill_and_persist(&events,&cfg,Some(&runs[0].to_string_lossy())),1);
        let mut next=cfg.clone();next.shadow_git_root=root.path().join("different-shadow");next.workspace_root=runs[1].clone();
        assert_eq!(managed_scope(&next,&runs[1]).1,path,"different Run/cwd must keep one managed destination");
        let data=SupermemoryStore::from_path(managed_scope(&next,&runs[1]).1).load_result().unwrap();assert_eq!(data.memories.len(),2);
        assert!(data.memory_context.values().any(|c|c.source=="automatic_reflection"&&c.kind=="experience_candidate"&&c.state=="candidate"&&c.project==project_identity(&origin)));
        assert!(recall(&next,&runs[1],None,10).hits.len()>=2);
        let legacy_hit=recall(&cfg,&runs[1],Some("legacy_sentinel"),10);assert!(legacy_hit.hits.iter().any(|h|h.record.source_label.starts_with("legacy_managed_project_notes:")));
        assert_eq!(std::fs::read(legacy).unwrap(),original);assert!(!origin.join(".supermemory").exists());
        let other=root.path().join("other");std::fs::create_dir(&other).unwrap();assert!(recall(&cfg,&other,Some("scope_sentinel"),10).hits.is_empty());
    }

    #[test]
    fn project_evolution_survives_run_change_and_rejects_foreign_or_unscoped_data() {
        use super::super::evolution_memory::{EvolutionMemory,EvolutionReport};
        let root=tempfile::tempdir().unwrap();let cfg=config(root.path());let project=root.path().join("source");let other=root.path().join("other");std::fs::create_dir(&project).unwrap();std::fs::create_dir(&other).unwrap();
        let old_path=root.path().join("legacy-global.json");let old=EvolutionMemory::new(old_path.clone());let report=EvolutionReport{task_kind:super::super::run_plan::TaskKind::Execution,completed:true,plan_closed:true,replans:0};old.observe(&report).unwrap();let old_bytes=std::fs::read(&old_path).unwrap();
        let current=EvolutionMemory::for_project(&cfg,&project);assert!(current.list().unwrap().is_empty(),"legacy global must not be silently adopted");current.observe(&report).unwrap();
        let fresh=EvolutionMemory::for_project(&cfg,&project);assert!(!fresh.list().unwrap().is_empty());assert_eq!(fresh.scope_receipt()["storageKind"],"project_runtime");
        let destination=cfg.project_notes_path(&other).with_file_name("evolution.json");std::fs::create_dir_all(destination.parent().unwrap()).unwrap();std::fs::copy(cfg.project_notes_path(&project).with_file_name("evolution.json"),&destination).unwrap();
        assert!(EvolutionMemory::for_project(&cfg,&other).list().is_err(),"copied foreign identity must fail closed");
        std::fs::write(&destination,&old_bytes).unwrap();assert!(EvolutionMemory::for_project(&cfg,&other).list().is_err(),"unscoped data is historical, not current project evidence");assert_eq!(std::fs::read(old_path).unwrap(),old_bytes);
    }
    fn record(key: &str, content: &str) -> SupermemoryRecord {
        serde_json::from_value(
            serde_json::json!({"id":key,"name":key,"summary":content,"sourceLabel":"fixture"}),
        )
        .unwrap()
    }
    #[test]
    fn survives_new_run_without_origin_writes_and_does_not_leak_projects() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let project = root.path().join("project-a");
        let other = root.path().join("project-b");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&other).unwrap();
        let mut workspaces = Vec::new();
        for name in ["run-1", "run-2"] {
            let workspace = root.path().join(name).join("workspace");
            std::fs::create_dir_all(&workspace).unwrap();
            std::fs::write(
                workspace.parent().unwrap().join("baseline.json"),
                serde_json::json!({"projectPath":project}).to_string(),
            )
            .unwrap();
            workspaces.push(workspace);
        }
        let store = SupermemoryStore::from_path(config.project_notes_path(&project));
        store
            .write_context_memory(
                record("ticket_reference", "random convention marker"),
                new_context(&project, &workspaces[0], None, None).unwrap(),
            )
            .unwrap();
        assert_eq!(
            recall(&config, &workspaces[1], Some("ticket_reference"), 5).hits[0]
                .record
                .summary,
            "random convention marker"
        );
        assert!(recall(&config, &other, None, 10).hits.is_empty());
        assert!(!project.join(".supermemory").exists());
        assert!(!workspaces[0].join(".supermemory").exists());
        let status=crate::services::brain::native_capabilities::snapshot(&workspaces[1],&config,None);
        assert_eq!(status["supermemory"]["project_notes"]["eligible_records"],1);
        assert_eq!(status["supermemory"]["project_notes"]["validation"],"unverified_history");
        let prelude = crate::services::brain::run_prelude::RunPreludeRuntime::new(
            config.clone(),
            workspaces[1].to_string_lossy(),
        )
        .prepare("ticket_reference");
        assert!(prelude
            .prompt_projection()
            .contains("unverified_project_note"));
        assert!(prelude.prompt_projection().contains("native_super_memory"));
        store.revoke_context_memory("ticket_reference").unwrap();
        assert!(recall(&config, &workspaces[1], None, 10).hits.is_empty());
    }
    #[test]
    fn revisions_expiry_and_source_changes_filter_before_search_and_prelude() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("source.txt"), "v1").unwrap();
        let store = SupermemoryStore::from_path(config.project_notes_path(&project));
        let context = new_context(&project, &project, Some("source.txt"), None).unwrap();
        store
            .write_context_memory(record("boundnote", "file convention"), context.clone())
            .unwrap();
        store
            .write_context_memory(record("boundnote", "file convention v2"), context)
            .unwrap();
        assert_eq!(
            store.load_result().unwrap().memory_context["boundnote"].revision,
            2
        );
        assert_eq!(
            recall(&config, &project, Some("boundnote"), 5).hits.len(),
            1
        );
        std::fs::write(project.join("source.txt"), "changed").unwrap();
        assert!(recall(&config, &project, Some("boundnote"), 5)
            .hits
            .is_empty());
        assert!(
            !crate::services::brain::run_prelude::RunPreludeRuntime::new(
                config.clone(),
                project.to_string_lossy()
            )
            .prepare("boundnote")
            .prompt_projection()
            .contains("file convention v2")
        );
        let mut data = store.load_result().unwrap();
        let metadata = data.memory_context.get_mut("boundnote").unwrap();
        metadata.source_file = None;
        metadata.source_hash = None;
        metadata.expires_at = Some("2000-01-01T00:00:00Z".into());
        store.save_result(&data).unwrap();
        assert!(recall(&config, &project, None, 10).hits.is_empty());
        data.memory_context.get_mut("boundnote").unwrap().expires_at = Some("invalid".into());
        store.save_result(&data).unwrap();
        assert!(recall(&config, &project, None, 10).hits.is_empty());
        let metadata = data.memory_context.get_mut("boundnote").unwrap();
        metadata.expires_at = None;
        metadata.project = "different-project".into();
        store.save_result(&data).unwrap();
        assert!(recall(&config, &project, None, 10).hits.is_empty());
    }
    #[test]
    fn legacy_is_labelled_and_revoked_managed_key_cannot_resurrect_legacy_copy() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut data = SupermemoryStoreData::default();
        data.memories.push(record("lesson", "legacy fixture"));
        let legacy = SupermemoryStore::new(&config, Some(&project.to_string_lossy()));
        legacy.save_result(&data).unwrap();
        let recalled = recall(&config, &project, None, 10);
        assert_eq!(
            reliability(recalled.hits[0].context.as_ref()),
            "legacy_unverified"
        );
        let prelude = crate::services::brain::run_prelude::RunPreludeRuntime::new(
            config.clone(),
            project.to_string_lossy(),
        )
        .prepare("lesson");
        assert!(prelude.prompt_projection().contains("legacy_unverified"));
        assert!(!prelude
            .prompt_projection()
            .contains("distilled-engineering-rules"));
        let managed = SupermemoryStore::from_path(config.project_notes_path(&project));
        managed
            .write_context_memory(
                record("lesson", "new candidate"),
                new_context(&project, &project, None, None).unwrap(),
            )
            .unwrap();
        managed.revoke_context_memory("lesson").unwrap();
        assert!(recall(&config, &project, Some("lesson"), 10)
            .hits
            .is_empty());
        assert_eq!(
            legacy.load_result().unwrap().memories[0].summary,
            "legacy fixture"
        );
    }
    #[test]
    fn credential_canaries_reject_without_changing_checkpoint_or_echoing_secret() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let store = SupermemoryStore::from_path(root.path().join("notes.json"));
        store.save_result(&SupermemoryStoreData::default()).unwrap();
        let before = std::fs::read(root.path().join("notes.json")).unwrap();
        for fake in [
            "sk-FAKE_CANARY_NOT_REAL_12345678",
            "Bearer FAKE_CANARY_TOKEN_12345678",
            "https://example.invalid/test?signature=FAKE_SIGNATURE_12345678",
        ] {
            let error = store
                .write_context_memory(
                    record("canary", fake),
                    new_context(root.path(), root.path(), None, None).unwrap(),
                )
                .unwrap_err();
            assert!(!error.contains(fake));
            assert_eq!(
                std::fs::read(root.path().join("notes.json")).unwrap(),
                before
            );
            let mut data = SupermemoryStoreData::default();
            data.memories.push(record("canary", fake));
            assert!(store.save_result(&data).is_err());
        }
        assert!(reject_sensitive(
            &serde_json::json!({"safe":"cloud receipt id and ordinary project conventions"})
        )
        .is_ok());
        assert_ne!(
            config.project_notes_path(root.path()),
            config.supermemory_store_path(Some(&root.path().to_string_lossy()))
        );
    }
    #[test]
    fn rejects_source_escape_and_surfaces_corrupt_store_without_hiding_as_empty() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(root.path().join("outside.txt"), "outside").unwrap();
        assert!(new_context(&project, &project, Some("../outside.txt"), None).is_err());
        assert!(new_context(
            &project,
            &project,
            Some(root.path().join("outside.txt").to_str().unwrap()),
            None
        )
        .is_err());
        assert!(new_context(&project, &project, None, Some("invalid")).is_err());
        assert!(new_context(&project, &project, None, Some("2000-01-01T00:00:00Z")).is_err());
        let path = config.project_notes_path(&project);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{invalid").unwrap();
        let recalled = recall(&config, &project, Some("anything"), 5);
        assert!(!recalled.warnings.is_empty());
        assert!(recalled.hits.is_empty());
    }
}
