use crate::services::brain::evolution_memory::{DistilledLesson, EvolutionMemory};
use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::{json, Value};

pub struct HermesTool {
    memory: EvolutionMemory,
}

impl HermesTool {
    pub fn new(memory: EvolutionMemory) -> Self {
        Self { memory }
    }
}

impl Default for HermesTool {
    fn default() -> Self {
        Self::new(EvolutionMemory::default_location())
    }
}

impl ToolExecutor for HermesTool {
    fn name(&self) -> &'static str {
        "hermes_workflow"
    }

    fn description(&self) -> &'static str {
        "Hermes Episodic Reflection & Distillation: Query currently valid governed experiences or save an unverified trap/solution draft. Activation requires a trusted held-out pass; legacy ELO history does not grant authority."
    }

    fn prompt_projection(&self)->Option<String>{
        Some("[hermes_workflow] Query current-project governed experiences with Action=query_lessons. Only currently valid held-out candidates are returned; legacy ELO counts are not proof. Action=record_lesson creates a quarantined proposal, never an active rule. list_mcps is metadata only. Use super_memory for explicit user notes. Never store credentials.".into())
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "Action": {
                    "type": "string",
                    "enum": ["query_lessons", "record_lesson", "list_mcps"],
                    "description": "Action to perform: 'query_lessons' to recall past experiences, 'record_lesson' to distill a new lesson, 'list_mcps' to list capabilities."
                },
                "Query": {
                    "type": "string",
                    "description": "Search keyword or scenario for querying relevant lessons."
                },
                "Domain": {
                    "type": "string",
                    "description": "Domain of the lesson (e.g. 'controller', 'mybatis', 'engine-service', 'frontend')."
                },
                "TrapPattern": {
                    "type": "string",
                    "description": "The mistake or anti-pattern encountered."
                },
                "GoldenSolution": {
                    "type": "string",
                    "description": "The verified solution or architectural pattern to apply."
                }
            },
            "required": ["Action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args.get("Action").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "query_lessons" => {
                let query = args.get("Query").and_then(|v| v.as_str()).unwrap_or("");
                let lessons=if let Some((config,workspace))=self.memory.project_scope() {
                    crate::services::brain::project_memory::recall(config,workspace,Some(query),100).hits.into_iter()
                        .filter(|h|h.context.as_ref().is_some_and(|c|c.source=="governed_experience"))
                        .take(3).map(|h|json!({"id":h.record.id,"name":h.record.name,"observation":h.record.summary,"source":h.record.source_label,"reliability":"trusted_profile_scope_only"})).collect::<Vec<_>>()
                } else {vec![]};
                Ok(json!({
                    "status": "success",
                    "query": query,
                    "matched_count": lessons.len(),
                    "lessons": lessons
                    ,"scope":self.memory.scope_receipt()
                }))
            }
            "record_lesson" => {
                let domain = args.get("Domain").and_then(|v| v.as_str()).unwrap_or("general");
                let trap = args.get("TrapPattern").and_then(|v| v.as_str()).ok_or("Missing required 'TrapPattern'")?;
                let solution = args.get("GoldenSolution").and_then(|v| v.as_str()).ok_or("Missing required 'GoldenSolution'")?;
                let scenario = args.get("Query").and_then(|v| v.as_str()).unwrap_or(trap);

                let lesson = DistilledLesson {
                    recovery_evidence:None,
                    id: format!("lesson-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                    domain: domain.to_string(),
                    trigger_scenario: scenario.to_string(),
                    trap_pattern: trap.to_string(),
                    golden_solution: solution.to_string(),
                    confidence_score: 0.9,
                    usage_count: 1,
                    updated_at: chrono::Utc::now().to_rfc3339(),
                };
                let (config,workspace)=self.memory.project_scope().ok_or("Scoped project required for a lesson candidate")?;
                let record=crate::models::memory::SupermemoryRecord {
                    id:lesson.id.clone(),name:format!("Lesson candidate: {}",lesson.domain),
                    summary:format!("Unverified draft. Scenario: {}\nTrap: {}\nProposed guidance: {}",lesson.trigger_scenario,lesson.trap_pattern,lesson.golden_solution),tags:vec!["experience_candidate".into()],file_path:String::new(),complexity:"unverified_candidate".into(),node_type:"reflexion".into(),source_graph_path:String::new(),source_label:"hermes_candidate".into(),updated_at:lesson.updated_at.clone(),embedding:None,
                };
                let saved=crate::services::brain::memory_persistence::persist_lesson_draft(config,workspace,record).map_err(|e|e.reason().to_string())?;
                Ok(json!({"status":"candidate","candidate_id":saved.id,"scope":self.memory.scope_receipt(),"message":"隔离经验草稿已保存，未通过不同目标留出验证前不参与召回。"}))
            }
            "list_mcps" => Ok(json!({
                "status": "success",
                "mcps": ["hermes-reflection", "openclaw-tool-synthesis", "rtk-compactor"]
            })),
            _ => Err(format!("Unknown Hermes action: {}", action)),
        }
    }
}
