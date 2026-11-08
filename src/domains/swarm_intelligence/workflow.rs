//! 声明式 DAG 智能体工作流流水线 (Agentic Workflow Pipeline)
//! 支持多阶段编排、阶段质量闸门检验、自动化流转与失败隔离。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub name: String,
    pub action: String,
    pub target_agent: Option<String>,
    pub status: WorkflowStatus,
    pub input_data: serde_json::Value,
    pub output_data: Option<serde_json::Value>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStage {
    pub id: String,
    pub name: String,
    pub steps: Vec<WorkflowStep>,
    pub status: WorkflowStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInstance {
    pub id: String,
    pub name: String,
    pub description: String,
    pub stages: Vec<WorkflowStage>,
    pub current_stage_index: usize,
    pub status: WorkflowStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct WorkflowEngine {
    workflows: HashMap<String, WorkflowInstance>,
}

impl WorkflowEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            workflows: HashMap::new(),
        };

        // 预设经典开发与审查闭环标准工作流 (Standard SDD Pipeline)
        let sample_id = "wf-sdd-standard".to_string();
        let sample = WorkflowInstance {
            id: sample_id.clone(),
            name: "Standard SDD Engineering Pipeline".to_string(),
            description: "End-to-end Spec -> Code -> Verify -> Review delivery pipeline".to_string(),
            stages: vec![
                WorkflowStage {
                    id: "stage-1-plan".to_string(),
                    name: "Architecture & Spec Design".to_string(),
                    status: WorkflowStatus::Pending,
                    steps: vec![
                        WorkflowStep {
                            id: "step-1-1".to_string(),
                            name: "Analyze Task Requirements".to_string(),
                            action: "search_graph".to_string(),
                            target_agent: Some("agent-leader".to_string()),
                            status: WorkflowStatus::Pending,
                            input_data: serde_json::json!({ "scope": "workspace" }),
                            output_data: None,
                            error_message: None,
                        },
                    ],
                },
                WorkflowStage {
                    id: "stage-2-code".to_string(),
                    name: "Surgical Implementation".to_string(),
                    status: WorkflowStatus::Pending,
                    steps: vec![
                        WorkflowStep {
                            id: "step-2-1".to_string(),
                            name: "Author Domain Logic".to_string(),
                            action: "replace_in_file".to_string(),
                            target_agent: Some("agent-coder".to_string()),
                            status: WorkflowStatus::Pending,
                            input_data: serde_json::json!({ "mode": "surgical" }),
                            output_data: None,
                            error_message: None,
                        },
                    ],
                },
                WorkflowStage {
                    id: "stage-3-verify".to_string(),
                    name: "Automated Verification Gate".to_string(),
                    status: WorkflowStatus::Pending,
                    steps: vec![
                        WorkflowStep {
                            id: "step-3-1".to_string(),
                            name: "Compile & Test Execution".to_string(),
                            action: "verify_task_tool".to_string(),
                            target_agent: Some("agent-tester".to_string()),
                            status: WorkflowStatus::Pending,
                            input_data: serde_json::json!({ "command": "cargo test" }),
                            output_data: None,
                            error_message: None,
                        },
                    ],
                },
                WorkflowStage {
                    id: "stage-4-review".to_string(),
                    name: "Code Review & Security Audit".to_string(),
                    status: WorkflowStatus::Pending,
                    steps: vec![
                        WorkflowStep {
                            id: "step-4-1".to_string(),
                            name: "Automated 5-Gate Review".to_string(),
                            action: "code_review".to_string(),
                            target_agent: Some("agent-reviewer".to_string()),
                            status: WorkflowStatus::Pending,
                            input_data: serde_json::json!({ "strict": true }),
                            output_data: None,
                            error_message: None,
                        },
                    ],
                },
            ],
            current_stage_index: 0,
            status: WorkflowStatus::Pending,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        engine.workflows.insert(sample_id, sample);
        engine
    }

    pub fn list_workflows(&self) -> Vec<WorkflowInstance> {
        self.workflows.values().cloned().collect()
    }

    pub fn get_workflow(&self, id: &str) -> Option<&WorkflowInstance> {
        self.workflows.get(id)
    }

    pub fn create_workflow(&mut self, name: &str, description: &str, mut stages: Vec<WorkflowStage>) -> String {
        // A caller supplies a definition, never execution receipts.
        for stage in &mut stages {
            stage.status = WorkflowStatus::Pending;
            for step in &mut stage.steps {
                step.status = WorkflowStatus::Pending;
                step.output_data = None;
                step.error_message = None;
            }
        }
        let id = format!("wf-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let instance = WorkflowInstance {
            id: id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            stages,
            current_stage_index: 0,
            status: WorkflowStatus::Pending,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.workflows.insert(id.clone(), instance);
        id
    }

    pub fn advance_workflow(&mut self, id: &str) -> Result<WorkflowInstance, String> {
        if !self.workflows.contains_key(id) {
            return Err("Workflow not found".into());
        }
        // The metadata engine has no execution adapter. A coordinator must
        // supply real Run/step receipts before any transition is possible.
        Err("Workflow execution runtime is not connected".into())
    }
}

lazy_static::lazy_static! {
    pub static ref GLOBAL_WORKFLOW_ENGINE: Arc<RwLock<WorkflowEngine>> = Arc::new(RwLock::new(WorkflowEngine::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unexecuted(workflow: &WorkflowInstance) {
        assert!(matches!(workflow.status, WorkflowStatus::Pending));
        assert_eq!(workflow.current_stage_index, 0);
        for stage in &workflow.stages {
            assert!(matches!(stage.status, WorkflowStatus::Pending));
            for step in &stage.steps {
                assert!(matches!(step.status, WorkflowStatus::Pending));
                assert!(step.output_data.is_none());
                assert!(step.error_message.is_none());
            }
        }
    }

    #[test]
    fn template_has_no_unobserved_execution_or_approval() {
        let engine = WorkflowEngine::new();
        assert_unexecuted(engine.get_workflow("wf-sdd-standard").unwrap());
    }

    #[test]
    fn execution_without_runtime_preserves_entire_workflow() {
        let mut engine = WorkflowEngine::new();
        let before = serde_json::to_value(engine.get_workflow("wf-sdd-standard")).unwrap();
        assert!(engine.advance_workflow("wf-sdd-standard").is_err());
        let after = serde_json::to_value(engine.get_workflow("wf-sdd-standard")).unwrap();
        assert_eq!(before, after, "No receipt may be manufactured by advancing a definition");
    }

    #[test]
    fn empty_workflow_cannot_be_completed_by_execution_request() {
        let mut engine = WorkflowEngine::new();
        let id = engine.create_workflow("empty", "no execution steps", vec![]);
        let before = serde_json::to_value(engine.get_workflow(&id)).unwrap();
        assert!(engine.advance_workflow(&id).is_err());
        assert_eq!(before, serde_json::to_value(engine.get_workflow(&id)).unwrap());
    }

    #[test]
    fn creating_definition_does_not_accept_caller_supplied_success() {
        let mut engine = WorkflowEngine::new();
        let mut stages = engine.get_workflow("wf-sdd-standard").unwrap().stages.clone();
        for stage in &mut stages {
            stage.status = WorkflowStatus::Succeeded;
            for step in &mut stage.steps {
                step.status = WorkflowStatus::Succeeded;
                step.output_data = Some(serde_json::json!({"untrusted": "success"}));
                step.error_message = Some("old execution".into());
            }
        }
        let id = engine.create_workflow("definition", "not a run", stages);
        assert_unexecuted(engine.get_workflow(&id).unwrap());
    }

    #[test]
    fn unknown_workflow_remains_missing() {
        let mut engine = WorkflowEngine::new();
        assert_eq!(engine.advance_workflow("unknown").unwrap_err(), "Workflow not found");
    }
}
