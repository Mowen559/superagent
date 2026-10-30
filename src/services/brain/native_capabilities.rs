//! Readiness and adapters for native project capabilities. Readiness is not a
//! successful invocation, and analysis artifacts never prove implementation.
use crate::config::runtime::RuntimeConfig;
use crate::domains::agent_cognitive::executor::{AgentLoop, ToolExecutor};
use crate::services::brain::tool_execution_gateway::ToolRegistry;
use crate::services::supermemory_store::SupermemoryStore;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

pub fn graph_readiness(path: &Path) -> Value {
    if !path.is_file() {
        return json!({"state":"missing","ready":false,"path":path,"reason":"Run project analysis to create the graph database"});
    }
    let check = || -> Result<(i64, i64), rusqlite::Error> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // Verify the schema consumed by the graph/understand implementations.
        conn.prepare("SELECT id,label,name,qualified_name,file_path,start_line,end_line,properties FROM nodes LIMIT 0")?;
        conn.prepare("SELECT source_id,target_id,type FROM edges LIMIT 0")?;
        Ok((
            conn.query_row("SELECT count(*) FROM nodes", [], |r| r.get(0))?,
            conn.query_row("SELECT count(*) FROM edges", [], |r| r.get(0))?,
        ))
    };
    match check() {
        Ok((nodes, edges)) => {
            json!({"state":if nodes>0{"ready"}else{"empty"},"ready":nodes>0,"path":path,"nodes":nodes,"edges":edges,"reason":if nodes>0{"Readable graph schema with nodes; semantic accuracy not certified"}else{"Graph has no nodes; run project analysis"}})
        }
        Err(_) => {
            json!({"state":"invalid","ready":false,"path":path,"reason":"Graph database is unreadable or incompatible with the native nodes/edges schema"})
        }
    }
}

pub fn snapshot(root: &Path, config: &RuntimeConfig, registered: Option<Vec<String>>) -> Value {
    let project = root.to_string_lossy();
    let graph = graph_readiness(&config.codebase_memory_db_path(Some(&project)));
    let memory_path = config.supermemory_store_path(Some(&project));
    let memory = match SupermemoryStore::new(config, Some(&project)).load_result() {
        Ok(data) => {
            json!({"state":if !memory_path.exists(){"missing"}else if data.memories.is_empty(){"empty"}else{"ready"},"path":memory_path,"records":data.memories.len(),"scope":"current_project"})
        }
        Err(_) => {
            json!({"state":"invalid","path":memory_path,"reason":"Project memory could not be read or parsed"})
        }
    };
    json!({"schema_version":1,"workspace":root,"registered_tools":registered,
        "graph":graph,"understand":{"ready":graph["ready"],"tool":"project_index","depends_on":"graph","metadata_path":config.knowledge_graph_path(Some(&project))},
        "supermemory":memory,
        "rtk":{"state":"integrated","source_filter":"rtk::core::filter","terminal_filter":"LocalShellProvider -> compress_by_command","process_metrics":crate::domains::token_compression::TokenSavingsTracker::global().get_stats(),"note":"Process metrics differ from rtk gain; zero usage is not a failed readiness check"},
        "requirement_engine":{"ready":graph["ready"],"tool":"requirement_engine","actions":["analyze","plan"],"scope":"analysis_and_future_plan_only","model_route":"not_probed"},
        "execution":{"tools":["view_file","replace_in_file","run_command"],"verified":false,"reason":"Use actual worker receipts and an independent check; readiness does not certify task success"}})
}

struct CapabilityStatusTool {
    root: PathBuf,
    registry: Weak<Mutex<std::collections::HashMap<String, Arc<dyn ToolExecutor>>>>,
}
impl ToolExecutor for CapabilityStatusTool {
    fn name(&self) -> &'static str {
        "capability_status"
    }
    fn description(&self) -> &'static str {
        "Inspect current-project readiness for graph/understand, Supermemory, RTK and requirement engines. Does not call a model or build an index. Readiness is not proof of a successful run."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn prompt_projection(&self) -> Option<String> {
        Some("Use capability_status when project graph/memory readiness is uncertain. Missing indexes require analysis or an honest file-based fallback; never pretend graph results exist.".into())
    }
    fn execute(&self, _: Value) -> Result<Value, String> {
        let registered = self.registry.upgrade().map(|tools| {
            let mut names = tools.lock().unwrap().keys().cloned().collect::<Vec<_>>();
            names.sort();
            names
        });
        let config = RuntimeConfig::from_env(&crate::config::app_paths::AppPaths::new());
        Ok(snapshot(&self.root, &config, registered))
    }
}

struct RequirementEngineTool {
    root: PathBuf,
    model: Option<String>,
}
impl ToolExecutor for RequirementEngineTool {
    fn name(&self) -> &'static str {
        "requirement_engine"
    }
    fn description(&self) -> &'static str {
        "Run the existing graph-grounded requirement engine. Action analyze writes .deepcloud/requirement_impact.md; plan writes .deepcloud/execution_plan.json. Requires a ready project graph and uses the selected model. These are analysis/future-plan artifacts, never implemented code or verification."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"Action":{"type":"string","enum":["analyze","plan"]},"requirement":{"type":"string","minLength":1,"maxLength":4000}},"required":["Action","requirement"]})
    }
    fn prompt_projection(&self) -> Option<String> {
        Some("For graph-grounded change impact or file-level implementation planning, use requirement_engine after capability_status/project_index. It does not implement the plan; perform code changes and run tests separately when authorized.".into())
    }
    fn execute(&self, args: Value) -> Result<Value, String> {
        let action = args["Action"]
            .as_str()
            .ok_or("Action is required")?
            .to_owned();
        if !matches!(action.as_str(), "analyze" | "plan") {
            return Err("Unsupported requirement engine action".into());
        }
        let requirement = args["requirement"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.chars().count() <= 4000)
            .ok_or("requirement must contain 1..4000 characters")?
            .to_owned();
        let config = RuntimeConfig::from_env(&crate::config::app_paths::AppPaths::new());
        let db = config.codebase_memory_db_path(Some(&self.root.to_string_lossy()));
        if graph_readiness(&db)["ready"] != true {
            return Err("Project graph is missing, empty or incompatible. Run project analysis or use file inspection; no requirement artifact was generated.".into());
        }
        let root = self.root.clone();
        let model = self.model.clone();
        std::thread::spawn(move || {
            let runtime=tokio::runtime::Runtime::new().map_err(|_|"Requirement runtime could not start")?;
            runtime.block_on(async move {
                let client=reqwest::Client::builder().timeout(Duration::from_secs(35)).build().map_err(|_|"Cannot build model client")?;
                let provider=crate::domains::model_gateway::providers::LlmProvider::with_model_override(client,model);
                let analyzer=crate::domains::code_analysis::requirement_analyzer::RequirementAnalyzer::new(provider);
                let db=db.to_string_lossy().to_string();
                let work=async {
                    if action=="analyze"{analyzer.analyze_requirement(&root,&db,&requirement,None).await}
                    else{analyzer.plan_requirement(&root,&db,&requirement,None).await}
                };
                // Provider details may include upstream text; expose a bounded
                // category here, not credentials or arbitrary response bodies.
                tokio::time::timeout(Duration::from_secs(45),work).await.map_err(|_|"Requirement engine timed out")?
                    .map_err(|_|"Requirement engine failed; check model readiness and graph schema")?;
                let path=root.join(".deepcloud").join(if action=="analyze"{"requirement_impact.md"}else{"execution_plan.json"});
                let text=std::fs::read_to_string(&path).map_err(|_|"Requirement artifact was not written")?;
                if action=="plan" {validate_plan(&text)?;}
                Ok(json!({"status":"generated","action":action,"artifact_path":path,"scope":"analysis_or_future_plan_only","content_excerpt":text.chars().take(4000).collect::<String>(),"truncated":text.chars().count()>4000}))
            })
        }).join().map_err(|_|"Requirement adapter panicked".to_string())?
    }
}

pub fn validate_plan(text: &str) -> Result<(), String> {
    crate::domains::code_analysis::requirement_analyzer::validate_execution_plan(text)
}

pub fn register(agent: &mut AgentLoop, root: &Path, model: Option<&str>) {
    use super::tools::{
        native_graph::NativeSearchGraphTool, native_memory::NativeMemoryTool,
        project_index_tool::ProjectIndexTool, repo_map_tool::RepoMapTool,
    };
    agent.register_tool(Box::new(NativeSearchGraphTool::new(root.to_path_buf())));
    agent.register_tool(Box::new(NativeMemoryTool::new(root.to_path_buf())));
    agent.register_tool(Box::new(ProjectIndexTool::new(root.to_path_buf())));
    agent.register_tool(Box::new(RepoMapTool::new(root.to_path_buf())));
    agent.register_tool(Box::new(RequirementEngineTool {
        root: root.to_path_buf(),
        model: model.map(str::to_owned),
    }));
    let registry: ToolRegistry = agent.get_tools_ref();
    agent.register_tool(Box::new(CapabilityStatusTool {
        root: root.to_path_buf(),
        registry: Arc::downgrade(&registry),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_readiness_rejects_missing_invalid_and_empty_databases() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        assert_eq!(graph_readiness(&path)["state"], "missing");
        assert!(
            !path.exists(),
            "read-only readiness must not create a database"
        );
        std::fs::write(&path, "not SQLite").unwrap();
        assert_eq!(graph_readiness(&path)["state"], "invalid");
        std::fs::remove_file(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE nodes(id INTEGER,label TEXT,name TEXT,qualified_name TEXT,file_path TEXT,start_line INTEGER,end_line INTEGER,properties TEXT); CREATE TABLE edges(source_id INTEGER,target_id INTEGER,type TEXT);").unwrap();
        assert_eq!(graph_readiness(&path)["state"], "empty");
        conn.execute(
            "INSERT INTO nodes VALUES(1,'Function','fee','fee','calc.cjs',1,2,'{}')",
            [],
        )
        .unwrap();
        assert_eq!(graph_readiness(&path)["ready"], true);
    }
    #[test]
    fn plan_validation_rejects_prose_empty_and_scalar_outputs() {
        for text in ["Implemented everything", "[]", "{}", "[1]", "[{}]"] {
            assert!(validate_plan(text).is_err());
        }
        assert!(validate_plan(r#"[{"file":"calc.cjs","instructions":"fix boundary"}]"#).is_ok());
    }
}
