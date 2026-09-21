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
        "Hermes Episodic Reflection & Distillation: Query historical field lessons or explicitly record a newly verified trap/solution pair to evolve the agent's long-term heuristics."
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
                let lessons = self.memory.retrieve_relevant_lessons(query).map_err(|e| e.to_string())?;
                Ok(json!({
                    "status": "success",
                    "query": query,
                    "matched_count": lessons.len(),
                    "lessons": lessons
                }))
            }
            "record_lesson" => {
                let domain = args.get("Domain").and_then(|v| v.as_str()).unwrap_or("general");
                let trap = args.get("TrapPattern").and_then(|v| v.as_str()).ok_or("Missing required 'TrapPattern'")?;
                let solution = args.get("GoldenSolution").and_then(|v| v.as_str()).ok_or("Missing required 'GoldenSolution'")?;
                let scenario = args.get("Query").and_then(|v| v.as_str()).unwrap_or(trap);

                let lesson = DistilledLesson {
                    id: format!("lesson-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                    domain: domain.to_string(),
                    trigger_scenario: scenario.to_string(),
                    trap_pattern: trap.to_string(),
                    golden_solution: solution.to_string(),
                    confidence_score: 0.9,
                    usage_count: 1,
                    updated_at: chrono::Utc::now().to_rfc3339(),
                };
                self.memory.record_lesson(lesson.clone()).map_err(|e| e.to_string())?;

                Ok(json!({
                    "status": "lesson_recorded",
                    "lesson": lesson,
                    "message": "Lesson successfully distilled and persisted to Hermes memory."
                }))
            }
            "list_mcps" => Ok(json!({
                "status": "success",
                "mcps": ["hermes-reflection", "openclaw-tool-synthesis", "rtk-compactor"]
            })),
            _ => Err(format!("Unknown Hermes action: {}", action)),
        }
    }
}
