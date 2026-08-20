use serde_json::{json, Value};
use crate::config::runtime::RuntimeConfig;
use crate::models::memory::SupermemoryRecord;
use crate::models::reverse::*;
use crate::services::llm_provider::{self, LlmProvider};
use crate::services::supermemory_store::SupermemoryStore;
use crate::services::understand_service;
use crate::services::progress::{ProgressReporter, StepStatus};
use super::rules;

/// Execute the full reverse engineering pipeline:
/// extraction → project context enrichment → blueprint → tasks → persistence.
pub async fn execute(
    trace_id: &str,
    requirement: &str,
    extracted_schema: Option<Value>,
    project: Option<&str>,
    options: Option<&ReverseEngineeringOptions>,
    runtime_config: &RuntimeConfig,
    http_client: &reqwest::Client,
    reporter: Option<std::sync::Arc<dyn ProgressReporter>>,
) -> Result<ReverseEngineeringResult, String> {
    let started = chrono::Utc::now();
    let mut trace_steps: Vec<TraceStep> = Vec::new();

    if let Some(r) = &reporter { r.report("Extracting requirement schema", StepStatus::InProgress); }

    // Step 1: Extract requirement schema. Failures fall back to a default
    // schema but are reported honestly (status=fallback + warning).
    let mut extraction_status = "ok".to_string();
    let mut extraction_warnings: Vec<String> = Vec::new();
    let language = options.and_then(|o| o.language.as_deref());

    let schema = if let Some(schema_val) = extracted_schema {
        match serde_json::from_value(schema_val) {
            Ok(s) => s,
            Err(e) => {
                extraction_status = "fallback".into();
                extraction_warnings.push(format!("Provided extracted_schema is invalid: {}", e));
                default_schema(requirement)
            }
        }
    } else {
        let llm = LlmProvider::new(http_client.clone());
        if llm.is_available() && !requirement.is_empty() {
            match extract_schema_via_llm(&llm, requirement, language, project).await {
                Ok(s) => s,
                Err(e) => {
                    extraction_status = "fallback".into();
                    extraction_warnings.push(format!("LLM extraction failed: {}", e));
                    default_schema(requirement)
                }
            }
        } else {
            extraction_status = "fallback".into();
            extraction_warnings.push("LLM not configured; using default schema.".into());
            default_schema(requirement)
        }
    };

    trace_steps.push(TraceStep {
        id: uuid::Uuid::new_v4().to_string(),
        stage: "extraction".into(),
        status: extraction_status.clone(),
        started_at: started.to_rfc3339(),
        completed_at: chrono::Utc::now().to_rfc3339(),
        input: Some(json!({ "requirement": requirement })),
        output: Some(serde_json::to_value(&schema).unwrap_or_default()),
        evidence: vec![TraceEvidence {
            source: "input".into(),
            label: "User requirement".into(),
            path: None,
            value: Some(json!(requirement)),
        }],
        warnings: extraction_warnings,
    });

    if let Some(r) = &reporter { 
        r.report("Extracting requirement schema", if extraction_status == "ok" { StepStatus::Completed } else { StepStatus::Failed }); 
    }

    if let Some(r) = &reporter { r.report("Querying project context", StepStatus::InProgress); }

    // Step 2: Enrich with project context (graph modules + supermemory).
    let context_started = chrono::Utc::now();
    let mut context_warnings: Vec<String> = Vec::new();
    let mut context_evidence: Vec<TraceEvidence> = Vec::new();
    let mut context = ProjectContext {
        supermemory: vec![],
        existing_modules: vec![],
    };

    if let Some(project_path) = project {
        let existing_module_limit = options.and_then(|o| o.existing_module_limit).unwrap_or(10);
        let supermemory_limit = options.and_then(|o| o.supermemory_limit).unwrap_or(5);

        // Top-level modules from the code graph, matched against the schema
        // through the alias table to decide create_new vs extend_existing.
        let db_path = runtime_config.codebase_memory_db_path(Some(project_path));
        let db_path_str = db_path.to_string_lossy().to_string();
        match understand_service::understand_project(&db_path_str) {
            Ok(v) if v.get("indexed").and_then(|x| x.as_bool()) == Some(true) => {
                let graph_modules: Vec<String> = v
                    .get("modules")
                    .and_then(|m| m.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                context.existing_modules = match_existing_modules(&schema, &graph_modules, existing_module_limit);
                context_evidence.push(TraceEvidence {
                    source: "codebase-memory".into(),
                    label: "Top-level graph modules".into(),
                    path: Some(db_path_str.clone()),
                    value: Some(json!(graph_modules)),
                });
            }
            Ok(_) => context_warnings.push("Project graph not indexed; module matching skipped.".into()),
            Err(e) => context_warnings.push(format!("Graph lookup failed: {}", e)),
        }

        // Related memories feed the project context for downstream agents.
        let store = SupermemoryStore::new(runtime_config, Some(project_path));
        let query = format!("{} {}", schema.requirement_summary, schema.features.join(" "));
        context.supermemory = store.search(&query, supermemory_limit);
        if !context.supermemory.is_empty() {
            context_evidence.push(TraceEvidence {
                source: "supermemory".into(),
                label: format!("{} related memories", context.supermemory.len()),
                path: None,
                value: None,
            });
        }
    } else {
        context_warnings.push("No project provided; context enrichment skipped.".into());
    }

    trace_steps.push(TraceStep {
        id: uuid::Uuid::new_v4().to_string(),
        stage: "context".into(),
        status: "ok".into(),
        started_at: context_started.to_rfc3339(),
        completed_at: chrono::Utc::now().to_rfc3339(),
        input: Some(json!({ "project": project })),
        output: Some(serde_json::to_value(&context).unwrap_or_default()),
        evidence: context_evidence,
        warnings: context_warnings,
    });

    if let Some(r) = &reporter { r.report("Querying project context", StepStatus::Completed); }

    if let Some(r) = &reporter { r.report("Building architecture blueprint", StepStatus::InProgress); }

    // Step 3: Build architecture blueprint
    let blueprint = build_blueprint(&schema, &context);

    trace_steps.push(TraceStep {
        id: uuid::Uuid::new_v4().to_string(),
        stage: "blueprint".into(),
        status: "ok".into(),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: chrono::Utc::now().to_rfc3339(),
        input: None,
        output: Some(serde_json::to_value(&blueprint).unwrap_or_default()),
        evidence: vec![],
        warnings: vec![],
    });

    if let Some(r) = &reporter { r.report("Building architecture blueprint", StepStatus::Completed); }

    if let Some(r) = &reporter { r.report("Generating development tasks", StepStatus::InProgress); }

    // Step 4: Generate development tasks
    let tasks = generate_tasks(&schema, &blueprint);

    trace_steps.push(TraceStep {
        id: uuid::Uuid::new_v4().to_string(),
        stage: "tasks".into(),
        status: "ok".into(),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: chrono::Utc::now().to_rfc3339(),
        input: None,
        output: Some(json!(tasks.len())),
        evidence: vec![],
        warnings: vec![],
    });

    if let Some(r) = &reporter { r.report("Generating development tasks", StepStatus::Completed); }

    if let Some(r) = &reporter { r.report("Persisting blueprint and tasks", StepStatus::InProgress); }

    // Step 5: Persist the blueprint + tasks into supermemory so later runs
    // (and the chat endpoint) can retrieve them.
    if let Some(project_path) = project {
        let store = SupermemoryStore::new(runtime_config, Some(project_path));
        let title: String = schema.requirement_summary.chars().take(60).collect();
        let module_names: Vec<&str> = blueprint.modules.iter().map(|m| m.name.as_str()).collect();
        let task_titles: Vec<&str> = tasks.iter().map(|t| t.title.as_str()).collect();
        store.write_memory(SupermemoryRecord {
            id: format!("blueprint-{}", trace_id),
            file_path: String::new(),
            name: format!("Blueprint: {}", title),
            summary: format!(
                "{}\nModules: {}\nTasks: {}",
                blueprint.summary,
                module_names.join(", "),
                task_titles.join("; ")
            ),
            tags: vec!["blueprint".into(), "reverse-engineering".into()],
            complexity: String::new(),
            node_type: "blueprint".into(),
            source_graph_path: String::new(),
            source_label: "reverse-engineering".into(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            embedding: None,
        });
        store.add_log("sync", Some(format!("Blueprint generated for: {}", title)), None);
    }

    if let Some(r) = &reporter { r.report("Persisting blueprint and tasks", StepStatus::Completed); }

    Ok(ReverseEngineeringResult {
        trace_id: trace_id.to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        requirement_schema: schema,
        project_context: context,
        architecture_blueprint: blueprint,
        development_tasks: tasks,
        trace: trace_steps,
    })
}

async fn extract_schema_via_llm(
    llm: &LlmProvider,
    requirement: &str,
    language: Option<&str>,
    project: Option<&str>,
) -> Result<RequirementSchema, String> {
    let (directive, _) = llm_provider::lang_directive(language.unwrap_or("zh-CN"));
    let messages = vec![json!({
        "role": "user",
        "content": format!(
            "Extract a structured requirement schema from this requirement. Return JSON with fields: requirement_summary, business_domain, actors, features, business_objects, engineering_constraints, modules, apis, pages, components, data_models, business_flow, technical_points, risks, missing_questions. All fields are optional; omit any you cannot infer. Text values inside the JSON: {}\n\nRequirement: {}",
            directive, requirement
        )
    })];

    let mut system_prompt = "You are a software architect. Return only valid JSON, without markdown code fences.".to_string();
    if let Some(project) = project {
        if let Some(instructions) = crate::services::project_context::load_project_instructions(Some(std::path::Path::new(project))) {
            system_prompt.push_str(&format!("\n\n[Project Instructions from .deepcloud.md]:\n{}", instructions));
        }
    }

    let response = llm.chat(messages, Some(&system_prompt)).await?;
    let cleaned = llm_provider::strip_code_fences(&response.0);
    serde_json::from_str(&cleaned).map_err(|e| format!("Failed to parse LLM schema: {}", e))
}

fn default_schema(requirement: &str) -> RequirementSchema {
    RequirementSchema {
        requirement_summary: requirement.to_string(),
        business_domain: "general".into(),
        actors: vec!["user".into()],
        engineering_constraints: EngineeringConstraints::default(),
        business_flow: rules::default_business_flow().into_iter().map(String::from).collect(),
        missing_questions: vec!["Please provide more specific requirements.".into()],
        ..Default::default()
    }
}

/// Match schema modules/features against the project's top-level graph
/// modules, expanding keywords through the alias table.
fn match_existing_modules(
    schema: &RequirementSchema,
    graph_modules: &[String],
    limit: usize,
) -> Vec<ProjectContextMatch> {
    let aliases = rules::module_aliases();
    let mut matches: Vec<ProjectContextMatch> = Vec::new();

    for want in schema.modules.iter().chain(schema.features.iter()) {
        let want_lc = want.to_lowercase();
        // Expand the wanted keyword set through the alias table.
        let mut keywords: Vec<String> = vec![want_lc.clone()];
        for (canonical, alias_list) in &aliases {
            if alias_list.iter().any(|a| want_lc.contains(a)) {
                keywords.push((*canonical).to_string());
                keywords.extend(alias_list.iter().map(|a| (*a).to_string()));
            }
        }

        for gm in graph_modules {
            let gm_lc = gm.to_lowercase();
            if keywords.iter().any(|k| gm_lc.contains(k.as_str()))
                && !matches.iter().any(|m| m.name == *gm) {
                    matches.push(ProjectContextMatch {
                        name: gm.clone(),
                        kind: "module".into(),
                        file_path: gm.clone(),
                        matched_from: want.clone(),
                    });
                }
        }
        if matches.len() >= limit {
            break;
        }
    }

    matches.truncate(limit);
    matches
}

fn build_blueprint(schema: &RequirementSchema, context: &ProjectContext) -> ArchitectureBlueprint {
    let modules: Vec<BlueprintModule> = schema.modules.iter().map(|m| {
        // Modules matched to existing code become extend_existing.
        let existing = context
            .existing_modules
            .iter()
            .find(|em| em.matched_from == *m);
        match existing {
            Some(em) => BlueprintModule {
                name: m.clone(),
                action: "extend_existing".into(),
                evidence: vec![format!("Matched existing module '{}' in project graph", em.name)],
            },
            None => BlueprintModule {
                name: m.clone(),
                action: "create_new".into(),
                evidence: vec![format!("Derived from requirement: {}", schema.requirement_summary)],
            },
        }
    }).collect();

    ArchitectureBlueprint {
        summary: format!("Architecture for: {}", schema.requirement_summary),
        engineering_constraints: schema.engineering_constraints.clone(),
        modules,
        apis: schema.apis.clone(),
        pages: schema.pages.clone(),
        components: schema.components.clone(),
        data_models: schema.data_models.clone(),
        business_flow: schema.business_flow.clone(),
        technical_points: schema.technical_points.clone(),
        risks: schema.risks.clone(),
        missing_questions: schema.missing_questions.clone(),
    }
}

fn generate_tasks(_schema: &RequirementSchema, blueprint: &ArchitectureBlueprint) -> Vec<DevelopmentTask> {
    let mut tasks = Vec::new();
    let mut idx = 0;

    // Investigation task
    tasks.push(DevelopmentTask {
        id: format!("task-{}", idx),
        title: "Investigate existing codebase structure".into(),
        task_type: "investigate".into(),
        target: None,
        reason: "Understand current architecture before making changes".into(),
        depends_on: vec![],
    });
    idx += 1;

    // Module creation tasks
    for module in &blueprint.modules {
        tasks.push(DevelopmentTask {
            id: format!("task-{}", idx),
            title: format!("Implement module: {}", module.name),
            task_type: if module.action == "extend_existing" { "extend" } else { "create" }.into(),
            target: Some(module.name.clone()),
            reason: format!("Required by architecture blueprint ({})", module.action),
            depends_on: vec![format!("task-{}", idx - 1)],
        });
        idx += 1;
    }

    // Testing task
    tasks.push(DevelopmentTask {
        id: format!("task-{}", idx),
        title: "Write integration tests".into(),
        task_type: "test".into(),
        target: None,
        reason: "Verify all modules work together".into(),
        depends_on: vec![format!("task-{}", idx - 1)],
    });

    tasks
}
