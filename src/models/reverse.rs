use serde::{Deserialize, Serialize};
use super::memory::SupermemoryRecord;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RequirementSchema {
    pub requirement_summary: String,
    pub business_domain: String,
    pub actors: Vec<String>,
    pub features: Vec<String>,
    pub business_objects: Vec<String>,
    pub engineering_constraints: EngineeringConstraints,
    pub modules: Vec<String>,
    pub apis: Vec<String>,
    pub pages: Vec<String>,
    pub components: Vec<String>,
    pub data_models: Vec<String>,
    pub business_flow: Vec<String>,
    pub technical_points: Vec<String>,
    pub risks: Vec<String>,
    pub missing_questions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineeringConstraints {
    pub target_platform: String,
    pub frontend: FrontendConstraints,
    pub backend: BackendConstraints,
    pub integration: IntegrationConstraints,
    pub testing: TestingConstraints,
    pub delivery: DeliveryConstraints,
}

impl Default for EngineeringConstraints {
    fn default() -> Self {
        Self {
            target_platform: "web".into(),
            frontend: FrontendConstraints::default(),
            backend: BackendConstraints::default(),
            integration: IntegrationConstraints::default(),
            testing: TestingConstraints::default(),
            delivery: DeliveryConstraints::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FrontendConstraints {
    pub enabled: Option<bool>,
    pub language: String,
    pub framework: String,
    pub ui_library: String,
    pub state_management: String,
}

impl Default for FrontendConstraints {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            language: "TypeScript".into(),
            framework: "React".into(),
            ui_library: "Tailwind CSS".into(),
            state_management: "zustand".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendConstraints {
    pub enabled: Option<bool>,
    pub language: String,
    pub runtime: String,
    pub framework: String,
    pub database: String,
    pub orm: String,
}

impl Default for BackendConstraints {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            language: "Rust".into(),
            runtime: "tokio".into(),
            framework: "axum".into(),
            database: "SQLite".into(),
            orm: "rusqlite".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationConstraints {
    pub external_services: Vec<String>,
    pub auth_required: Option<bool>,
    pub ide_plugin_supported: Option<bool>,
}

impl Default for IntegrationConstraints {
    fn default() -> Self {
        Self {
            external_services: vec![],
            auth_required: Some(false),
            ide_plugin_supported: Some(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestingConstraints {
    pub unit: bool,
    pub integration: bool,
    pub e2e: bool,
}

impl Default for TestingConstraints {
    fn default() -> Self {
        Self { unit: true, integration: true, e2e: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeliveryConstraints {
    pub generate_blueprint: bool,
    pub generate_tasks: bool,
    pub generate_code: bool,
    pub trace_required: bool,
}

impl Default for DeliveryConstraints {
    fn default() -> Self {
        Self {
            generate_blueprint: true,
            generate_tasks: true,
            generate_code: false,
            trace_required: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverseEngineeringRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    /// Project path used to enrich the result with graph + supermemory context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ReverseEngineeringOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverseEngineeringOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supermemory_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_module_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvidence {
    pub source: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub id: String,
    pub stage: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    pub evidence: Vec<TraceEvidence>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContextMatch {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub matched_from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureBlueprint {
    pub summary: String,
    pub engineering_constraints: EngineeringConstraints,
    pub modules: Vec<BlueprintModule>,
    pub apis: Vec<String>,
    pub pages: Vec<String>,
    pub components: Vec<String>,
    pub data_models: Vec<String>,
    pub business_flow: Vec<String>,
    pub technical_points: Vec<String>,
    pub risks: Vec<String>,
    pub missing_questions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintModule {
    pub name: String,
    pub action: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevelopmentTask {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub task_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub reason: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverseEngineeringResult {
    pub trace_id: String,
    pub generated_at: String,
    pub requirement_schema: RequirementSchema,
    pub project_context: ProjectContext,
    pub architecture_blueprint: ArchitectureBlueprint,
    pub development_tasks: Vec<DevelopmentTask>,
    pub trace: Vec<TraceStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    pub supermemory: Vec<SupermemoryRecord>,
    pub existing_modules: Vec<ProjectContextMatch>,
}
