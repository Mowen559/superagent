use crate::{
    domains::agent_cognitive::executor::ToolExecutor,
    models::memory::SupermemoryRecord,
    services::{brain::project_memory, supermemory_store::SupermemoryStore},
};
use serde_json::{json, Value};

pub struct NativeMemoryTool {
    pub workspace_root: std::path::PathBuf,
}
impl NativeMemoryTool {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self { workspace_root }
    }
}
impl ToolExecutor for NativeMemoryTool {
    fn name(&self) -> &'static str {
        "super_memory"
    }
    fn description(&self) -> &'static str {
        "Project-scoped persistent notes: store, retrieve by exact key, search, list, revoke. Notes are unverified historical data; relevance does not establish truth. Never store credentials."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["store","retrieve","search","list","revoke"]},
            "key":{"type":"string","description":"Exact key required for store/retrieve/revoke"},
            "query":{"type":"string"},"content":{"type":"string","description":"Non-sensitive note for store"},
            "source_file":{"type":"string","description":"Optional existing workspace-relative file; changed contents invalidate recall"},
            "expires_at":{"type":"string","description":"Optional future RFC3339 expiry"}
        },"required":["action"]})
    }
    fn prompt_projection(&self) -> Option<String> {
        Some("[super_memory] Search/retrieve relevant project notes before repeating investigation. Store useful non-sensitive project conventions or lessons as candidate evidence. Store/revoke mutate persistent project notes; read-only tasks cannot do so. Source binding and expiry are optional. Recall never promotes a note to verified authority; recheck against current files and user authorization.".into())
    }
    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args["action"].as_str().ok_or("Missing action")?;
        let config = crate::config::runtime::RuntimeConfig::from_env(
            &crate::config::app_paths::AppPaths::new(),
        );
        let (project,path) = project_memory::managed_scope(&config,&self.workspace_root);
        let store = SupermemoryStore::from_path(&path);
        let key = || {
            args["key"]
                .as_str()
                .filter(|key| !key.trim().is_empty() && key.len() <= 200)
                .ok_or("Non-empty key (maximum 200 bytes) required".to_string())
        };
        match action {
            "store" => {
                let key = key()?;
                let content = args["content"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty() && s.len() <= 32_000)
                    .ok_or("Non-empty content (maximum 32000 bytes) required")?;
                let optional = |name: &str| -> Result<Option<&str>, String> {
                    match args.get(name) {
                        None | Some(Value::Null) => Ok(None),
                        Some(Value::String(s)) => Ok(Some(s)),
                        _ => Err(format!("{name} must be a string")),
                    }
                };
                let context = project_memory::new_context(
                    &project,
                    &self.workspace_root,
                    optional("source_file")?,
                    optional("expires_at")?,
                )?;
                let record = SupermemoryRecord {
                    id: key.into(),
                    name: key.into(),
                    summary: content.into(),
                    tags: vec!["project_note".into()],
                    file_path: String::new(),
                    complexity: String::new(),
                    node_type: "project_note".into(),
                    source_graph_path: String::new(),
                    source_label: "native_super_memory".into(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    embedding: None,
                };
                store.write_context_memory(record, context)?;
                let mut data = store.load_result()?;
                if !data.memories.iter().any(|record|record.id==key && record.summary==content) {
                    return Err("Memory readback did not match the requested note".into());
                }
                let context = data.memory_context
                    .remove(key)
                    .ok_or("Stored context unavailable")?;
                Ok(
                    json!({"status":"stored","key":key,"store_target":path,"context":context,"reliability":"unverified_project_note","operation_verified":true}),
                )
            }
            "revoke" => {
                let key = key()?;
                store.revoke_context_memory(key)?;
                let data=store.load_result()?;
                if !data.memory_context.get(key).is_some_and(|context|context.state=="revoked") {
                    return Err("Memory revocation readback did not match".into());
                }
                Ok(json!({"status":"revoked","key":key,"project":project,"operation_verified":true}))
            }
            "retrieve" | "search" | "list" => {
                let query = match action {
                    "retrieve" => Some(key()?),
                    "search" => Some(args["query"].as_str().ok_or("Missing query")?),
                    _ => None,
                };
                let mut recalled = project_memory::recall(
                    &config,
                    &self.workspace_root,
                    if action == "retrieve" { None } else { query },
                    usize::MAX,
                );
                if action == "retrieve" {
                    recalled
                        .hits
                        .retain(|hit| Some(hit.record.id.as_str()) == query);
                }
                recalled
                    .hits
                    .truncate(if action == "list" { 10 } else { 5 });
                let results = recalled.hits.iter().map(|hit| json!({"key":hit.record.id,"name":hit.record.name,"content":hit.record.summary,
                    "source":hit.record.source_label,"updatedAt":hit.record.updated_at,"context":hit.context,
                    "reliability":project_memory::reliability(hit.context.as_ref())})).collect::<Vec<_>>();
                Ok(
                    json!({"status":"success","project":project,"results":results,"warnings":recalled.warnings}),
                )
            }
            _ => Err("Unknown memory action".into()),
        }
    }
}
