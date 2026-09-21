use crate::config::runtime::RuntimeConfig;
use crate::domains::version_control::git_service;
use crate::domains::model_gateway::providers::LlmProvider;
use crate::domains::code_analysis::requirement_analyzer::RequirementAnalyzer;
use crate::services::workspace_guard::WorkspaceGuard;
use reqwest::Client;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use tokio::runtime::Builder;

pub fn run_mcp() -> ! {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for MCP");

    runtime.block_on(async {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut handle = stdin.lock();
        let mut buffer = String::new();

        let config = RuntimeConfig::from_env(&crate::config::app_paths::AppPaths::new());

        loop {
            buffer.clear();
            match handle.read_line(&mut buffer) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if let Ok(req) = serde_json::from_str::<Value>(&buffer) {
                        if let Some(id) = req.get("id") {
                            let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
                            let empty_params = json!({});
                            let params = req.get("params").unwrap_or(&empty_params);

                            let rpc_response = match handle_request(method, params, &config).await {
                                Ok(res) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": res
                                }),
                                Err((code, msg)) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": {
                                        "code": code,
                                        "message": msg
                                    }
                                }),
                            };

                            if let Ok(res_str) = serde_json::to_string(&rpc_response) {
                                println!("{}", res_str);
                                stdout.flush().unwrap();
                            }
                        } else if let Some(method) = req.get("method").and_then(|m| m.as_str()) {
                            // Notifications
                            if method == "notifications/initialized" {
                                // Ignore or log
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    std::process::exit(0);
}

async fn handle_request(
    method: &str,
    params: &Value,
    config: &RuntimeConfig,
) -> Result<Value, (i32, String)> {
    match method {
        "initialize" => {
            // Auto-provision RTK rules for Antigravity silently in the background
            let ctx = rtk::hooks::init::InitContext {
                verbose: 0,
                dry_run: false,
            };
            let _ = rtk::hooks::init::run_antigravity_mode(ctx);

            Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "experimental": { "capability_catalog": true }
                },
                "serverInfo": {
                    "name": "deepcloud-hub",
                    "version": "0.1.0"
                }
            }))
        }
        "tools/list" => Ok(json!({
            "tools": [
                {
                    "name": "analyze_requirement",
                    "description": "Analyze a requirement and predict impacted files",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" },
                            "requirement": { "type": "string" }
                        },
                        "required": ["project_path", "requirement"]
                    }
                },
                {
                    "name": "shadow_git_snapshot",
                    "description": "Create a shadow git snapshot of current working directory",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" },
                            "message": { "type": "string" }
                        },
                        "required": ["project_path", "message"]
                    }
                },
                {
                    "name": "shadow_git_history",
                    "description": "Get history of shadow git snapshots",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" },
                            "limit": { "type": "number" }
                        },
                        "required": ["project_path"]
                    }
                },
                {
                    "name": "shadow_git_rollback",
                    "description": "Rollback working directory to a specific shadow git snapshot",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" },
                            "hash": { "type": "string" }
                        },
                        "required": ["project_path", "hash"]
                    }
                },
                {
                    "name": "supermemory_search",
                    "description": "Search user preferences and project key information from supermemory",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" },
                            "query": { "type": "string" },
                            "limit": { "type": "number" }
                        },
                        "required": ["project_path", "query"]
                    }
                },
                {
                    "name": "supermemory_write",
                    "description": "Write a user preference, context, or key project information into supermemory",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" },
                            "name": { "type": "string" },
                            "summary": { "type": "string" },
                            "tags": {
                                "type": "array",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["project_path", "name", "summary"]
                    }
                },
                {
                    "name": "capability_catalog",
                    "description": "List certified and experimental hub-rs capabilities with evidence",
                    "inputSchema": { "type": "object", "properties": {} }
                },
                {
                    "name": "search_graph",
                    "description": "Search code symbols and files in the Knowledge Graph by label, regex, file pattern, and degree filters",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Symbol name or search query" },
                            "label": { "type": "string", "description": "Filter by label (Function, Class, Interface, Route, File)" },
                            "name_pattern": { "type": "string", "description": "Name wildcard pattern" },
                            "file_pattern": { "type": "string", "description": "File path pattern" },
                            "min_degree": { "type": "number", "description": "Minimum node degree (in + out edges)" },
                            "max_degree": { "type": "number", "description": "Maximum node degree" },
                            "limit": { "type": "number", "description": "Max results to return (default 50)" },
                            "offset": { "type": "number", "description": "Pagination offset (default 0)" },
                            "project_path": { "type": "string", "description": "Optional project root path" }
                        }
                    }
                },
                {
                    "name": "search_code",
                    "description": "Perform scoped text search strictly within indexed repository source files",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Text or regex to search" },
                            "file_pattern": { "type": "string", "description": "File glob or pattern (e.g. *.rs)" },
                            "limit": { "type": "number", "description": "Max matches (default 50)" },
                            "project_path": { "type": "string" }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "classify_update",
                    "description": "Evaluate workspace changes to decide incremental graph update strategy (SKIP, PARTIAL_UPDATE, ARCHITECTURE_UPDATE, FULL_UPDATE)",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "detect_language_concepts",
                    "description": "Detect software engineering design patterns and programming paradigms (DI, Observer, Middleware, Singleton, Repository, Concurrency, etc.)",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string", "description": "Optional project root path" },
                            "content": { "type": "string", "description": "Optional raw code content to analyze directly" },
                            "limit": { "type": "number", "description": "Max nodes to sample from graph (default 200)" }
                        }
                    }
                },
                {
                    "name": "check_graph_freshness",
                    "description": "Detect synchronization freshness and Git commit drift between knowledge graph and physical repo",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string", "description": "Optional project root path" }
                        }
                    }
                },
                {
                    "name": "query_complexity",
                    "description": "Query code complexity, nesting loop depth, and detect hot-path bottlenecks like linear scans in loops",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string", "description": "Optional project root path" },
                            "limit": { "type": "number", "description": "Top N complexity functions to return (default 20)" }
                        }
                    }
                },
                {
                    "name": "detect_code_clones",
                    "description": "Calculate MinHash approximate Jaccard similarity between two code snippets to detect duplication or clones",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "code_a": { "type": "string", "description": "First code snippet" },
                            "code_b": { "type": "string", "description": "Second code snippet" }
                        },
                        "required": ["code_a", "code_b"]
                    }
                },
                {
                    "name": "list_indexed_projects",
                    "description": "Discover all indexed knowledge graph databases and their node/edge scale and health metadata",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "pipeline_compress",
                    "description": "Compress command output using RTK 8-stage declarative pipeline filter engine",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "output": { "type": "string", "description": "Raw terminal or command output" },
                            "rule_name": { "type": "string", "description": "Built-in rule name (e.g. cargo-clippy, eslint, terraform-plan)" }
                        },
                        "required": ["output"]
                    }
                },
                {
                    "name": "get_callers",
                    "description": "Find all functions or methods that call the target symbol ID",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "target_id": { "type": "string", "description": "Target symbol ID" },
                            "limit": { "type": "number", "description": "Max results (default 20)" },
                            "project_path": { "type": "string" }
                        },
                        "required": ["target_id"]
                    }
                },
                {
                    "name": "get_callees",
                    "description": "Find all functions or methods called by the source symbol ID",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "source_id": { "type": "string", "description": "Source symbol ID" },
                            "limit": { "type": "number", "description": "Max results (default 20)" },
                            "project_path": { "type": "string" }
                        },
                        "required": ["source_id"]
                    }
                },
                {
                    "name": "get_impact",
                    "description": "Analyze change blast radius and transitive downstream impact of a symbol",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "target_id": { "type": "string", "description": "Target symbol ID" },
                            "depth": { "type": "number", "description": "Impact analysis depth (default 3)" },
                            "project_path": { "type": "string" }
                        },
                        "required": ["target_id"]
                    }
                },
                {
                    "name": "trace_path",
                    "description": "Trace multi-hop call chain and dependency paths between symbols",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "function_name": { "type": "string", "description": "Symbol name or qualified name" },
                            "direction": { "type": "string", "enum": ["inbound", "outbound", "both"], "description": "Trace direction (default: outbound)" },
                            "depth": { "type": "number", "description": "Max traversal depth (default: 3)" },
                            "project_path": { "type": "string" }
                        },
                        "required": ["function_name"]
                    }
                },
                {
                    "name": "get_graph_schema",
                    "description": "Inspect knowledge graph schema, node label distribution, and edge types",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "get_code_snippet",
                    "description": "Retrieve precise source code snippet with line numbers and caller/callee neighbors",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "qualified_name": { "type": "string", "description": "Symbol qualified name" },
                            "include_neighbors": { "type": "boolean", "description": "Whether to include callers/callees (default: true)" },
                            "project_path": { "type": "string" }
                        },
                        "required": ["qualified_name"]
                    }
                },
                {
                    "name": "get_dead_code",
                    "description": "Detect orphan / unreferenced code symbols (in-degree 0 without callers)",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "limit": { "type": "number", "description": "Max symbols to return (default 20)" },
                            "project_path": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "get_architecture",
                    "description": "Get deterministic architectural layer analysis and dependency violations",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "detect_changes",
                    "description": "Analyze uncommitted git changes and map downstream impact & risk level",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "manage_adr",
                    "description": "Manage Architecture Decision Records (list, get, create)",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "enum": ["list", "get", "create"], "description": "Action to perform" },
                            "id": { "type": "string", "description": "ADR number or ID (for get)" },
                            "title": { "type": "string", "description": "ADR title (for create)" },
                            "status": { "type": "string", "description": "ADR status (Proposed, Accepted, etc.)" },
                            "context": { "type": "string", "description": "ADR background context" },
                            "decision": { "type": "string", "description": "ADR decision statement" },
                            "consequences": { "type": "string", "description": "ADR positive/negative consequences" },
                            "project_path": { "type": "string" }
                        },
                        "required": ["action"]
                    }
                },
                {
                    "name": "get_onboarding_guide",
                    "description": "Generate comprehensive project onboarding guide with tech stack, layers, and reading order",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "get_token_savings",
                    "description": "Get global RTK token savings metrics and reduction statistics",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let empty_args = json!({});
            let args = params.get("arguments").unwrap_or(&empty_args);

            let res = match name {
                "analyze_requirement" => {
                    let project = args
                        .get("project_path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let req = args
                        .get("requirement")
                        .and_then(|r| r.as_str())
                        .unwrap_or("");

                    let guard = match WorkspaceGuard::new(&config.projects_root) {
                        Ok(g) => g,
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid root: {}", e) }] }),
                            )
                        }
                    };
                    let safe_project = match guard.resolve_safe(project) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid project path: {}", e) }] }),
                            )
                        }
                    };

                    let llm = LlmProvider::new(Client::new());
                    let analyzer = RequirementAnalyzer::new(llm);
                    let db_path = config.codebase_memory_db_path(Some(&safe_project));
                    let rt =
                        crate::domains::code_analysis::code_intelligence::CodeIntelligenceRuntime::new(&db_path);
                    match analyzer
                        .analyze_requirement(
                            std::path::Path::new(&safe_project),
                            &rt.db_path_str(),
                            req,
                            None,
                        )
                        .await
                    {
                        Ok(_) => {
                            json!({ "content": [{ "type": "text", "text": "Requirement analysis generated successfully." }] })
                        }
                        Err(e) => {
                            json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error: {}", e) }] })
                        }
                    }
                }
                "shadow_git_snapshot" => {
                    let project = args
                        .get("project_path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let msg = args
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("AI Edit Snapshot");

                    let guard = match WorkspaceGuard::new(&config.projects_root) {
                        Ok(g) => g,
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid root: {}", e) }] }),
                            )
                        }
                    };
                    let safe_project = match guard.resolve_safe(project) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid project path: {}", e) }] }),
                            )
                        }
                    };

                    match git_service::record_ai_snapshot(&safe_project, msg) {
                        Ok(hash) => {
                            json!({ "content": [{ "type": "text", "text": format!("Snapshot created: {}", hash) }] })
                        }
                        Err(e) => {
                            json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error: {}", e) }] })
                        }
                    }
                }
                "shadow_git_history" => {
                    let project = args
                        .get("project_path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;

                    let guard = match WorkspaceGuard::new(&config.projects_root) {
                        Ok(g) => g,
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid root: {}", e) }] }),
                            )
                        }
                    };
                    let safe_project = match guard.resolve_safe(project) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid project path: {}", e) }] }),
                            )
                        }
                    };

                    match git_service::get_ai_history(&safe_project, limit) {
                        Ok(history) => {
                            json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&history).unwrap_or_default() }] })
                        }
                        Err(e) => {
                            json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error: {}", e) }] })
                        }
                    }
                }
                "shadow_git_rollback" => {
                    let project = args
                        .get("project_path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let hash = args.get("hash").and_then(|h| h.as_str()).unwrap_or("");

                    let guard = match WorkspaceGuard::new(&config.projects_root) {
                        Ok(g) => g,
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid root: {}", e) }] }),
                            )
                        }
                    };
                    let safe_project = match guard.resolve_safe(project) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid project path: {}", e) }] }),
                            )
                        }
                    };

                    match git_service::rollback_to_snapshot(&safe_project, hash) {
                        Ok(_) => {
                            json!({ "content": [{ "type": "text", "text": format!("Rolled back to {}", hash) }] })
                        }
                        Err(e) => {
                            json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error: {}", e) }] })
                        }
                    }
                }
                "supermemory_search" => {
                    let project = args
                        .get("project_path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(5) as usize;

                    let guard = match WorkspaceGuard::new(&config.projects_root) {
                        Ok(g) => g,
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid root: {}", e) }] }),
                            )
                        }
                    };
                    let safe_project = match guard.resolve_safe(project) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid project path: {}", e) }] }),
                            )
                        }
                    };

                    let store = crate::services::supermemory_store::SupermemoryStore::new(
                        config,
                        Some(&safe_project),
                    );
                    let results = store.search(query, limit);

                    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&results).unwrap_or_default() }] })
                }
                "supermemory_write" => {
                    let project = args
                        .get("project_path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let name = args.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let summary = args.get("summary").and_then(|s| s.as_str()).unwrap_or("");
                    let tags: Vec<String> = args
                        .get("tags")
                        .and_then(|t| t.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let guard = match WorkspaceGuard::new(&config.projects_root) {
                        Ok(g) => g,
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid root: {}", e) }] }),
                            )
                        }
                    };
                    let safe_project = match guard.resolve_safe(project) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return Ok(
                                json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid project path: {}", e) }] }),
                            )
                        }
                    };

                    let store = crate::services::supermemory_store::SupermemoryStore::new(
                        config,
                        Some(&safe_project),
                    );
                    let record = crate::models::memory::SupermemoryRecord {
                        id: uuid::Uuid::new_v4().to_string(),
                        file_path: "".to_string(),
                        name: name.to_string(),
                        summary: summary.to_string(),
                        tags,
                        complexity: "low".to_string(),
                        node_type: "preference".to_string(),
                        source_graph_path: "".to_string(),
                        source_label: "MCP".to_string(),
                        updated_at: chrono::Utc::now().to_rfc3339(),
                        embedding: None,
                    };
                    store.write_memory(record);

                    json!({ "content": [{ "type": "text", "text": "Successfully written to supermemory." }] })
                }
                "capability_catalog" => {
                    let gatekeeper = crate::services::brain::capabilities::certification::CertificationGatekeeper::with_builtins();
                    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&gatekeeper.list_capabilities()).unwrap_or_default() }] })
                }
                "search_graph" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let query = args.get("query").or_else(|| args.get("name_pattern")).and_then(|q| q.as_str());
                    let label = args.get("label").and_then(|l| l.as_str());
                    let file_pattern = args.get("file_pattern").and_then(|f| f.as_str());
                    let min_degree = args.get("min_degree").and_then(|d| d.as_u64()).map(|d| d as usize);
                    let max_degree = args.get("max_degree").and_then(|d| d.as_u64()).map(|d| d as usize);
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;
                    let offset = args.get("offset").and_then(|o| o.as_u64()).unwrap_or(0) as usize;

                    let rt = crate::domains::code_analysis::code_intelligence::CodeIntelligenceRuntime::new(&db_path_str);
                    match rt.search_graph(label, query, file_pattern, min_degree, max_degree, limit, offset) {
                        Ok(results) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&results).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Search graph failed: {}", e) }] }),
                    }
                }
                "search_code" => {
                    let (safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
                    let file_pattern = args.get("file_pattern").and_then(|f| f.as_str());
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;

                    let rt = crate::domains::code_analysis::code_intelligence::CodeIntelligenceRuntime::new(&db_path_str);
                    let proj_path = std::path::Path::new(&safe_project);
                    match rt.search_code(query, file_pattern, Some(proj_path), limit) {
                        Ok(matches) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&matches).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Search code failed: {}", e) }] }),
                    }
                }
                "classify_update" => {
                    let (safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let proj_path = std::path::Path::new(&safe_project);
                    match crate::domains::version_control::change_classifier::ChangeClassifier::classify_changes(proj_path, Some(&db_path_str)) {
                        Ok(decision) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&decision).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Classify update failed: {}", e) }] }),
                    }
                }
                "detect_language_concepts" => {
                    if let Some(content) = args.get("content").and_then(|c| c.as_str()) {
                        let concepts = crate::services::language_lesson::detect_concepts_in_content(content, "");
                        json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&concepts).unwrap_or_default() }] })
                    } else {
                        let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                            Ok(pair) => pair,
                            Err(err_json) => return Ok(err_json),
                        };
                        let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(200) as usize;
                        match crate::services::language_lesson::detect_project_concepts(&db_path_str, limit) {
                            Ok(report) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&report).unwrap_or_default() }] }),
                            Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Detect concepts failed: {}", e) }] }),
                        }
                    }
                }
                "check_graph_freshness" => {
                    let (safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let proj_path = std::path::Path::new(&safe_project);
                    let db_path = std::path::Path::new(&db_path_str);
                    let result = crate::domains::knowledge_graph::graph_freshness::GraphFreshnessDetector::check_freshness(db_path, proj_path);
                    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default() }] })
                }
                "query_complexity" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
                    match crate::domains::code_analysis::code_complexity::ComplexityAnalyzer::analyze_project_complexity(&db_path_str, limit) {
                        Ok(report) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&report).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Query complexity failed: {}", e) }] }),
                    }
                }
                "detect_code_clones" => {
                    let code_a = args.get("code_a").and_then(|c| c.as_str()).unwrap_or("");
                    let code_b = args.get("code_b").and_then(|c| c.as_str()).unwrap_or("");
                    let sim = crate::domains::code_analysis::code_similarity::CodeSimilarityDetector::compare_snippets(code_a, code_b);
                    let clone_type = if sim >= 0.90 {
                        "EXACT_OR_RENAME"
                    } else if sim >= 0.70 {
                        "HIGH_SIMILARITY"
                    } else if sim >= 0.30 {
                        "STRUCTURAL_SIMILARITY"
                    } else {
                        "DISTINCT"
                    };
                    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&json!({
                        "similarity": (sim * 1000.0).round() / 1000.0,
                        "cloneType": clone_type,
                    })).unwrap_or_default() }] })
                }
                "list_indexed_projects" => {
                    let root = &config.projects_root;
                    let projects = crate::domains::project_management::project_manager::ProjectIndexManager::discover_projects(root);
                    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&projects).unwrap_or_default() }] })
                }
                "pipeline_compress" => {
                    let output = args.get("output").and_then(|o| o.as_str()).unwrap_or("");
                    let rule_name = args.get("rule_name").and_then(|r| r.as_str());
                    let rule = match rule_name {
                        Some(name) => crate::services::pipeline_filter::get_builtin_rules().iter().find(|r| r.name == name),
                        None => crate::services::pipeline_filter::get_builtin_rules().first(),
                    };
                    let filtered = if let Some(r) = rule {
                        crate::services::pipeline_filter::PipelineFilterEngine::filter(output, r)
                    } else {
                        output.to_string()
                    };
                    json!({ "content": [{ "type": "text", "text": filtered }] })
                }
                "get_callers" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let target_id = args.get("target_id").and_then(|q| q.as_str()).unwrap_or("");
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
                    match crate::domains::knowledge_graph::graph_service::get_callers(&db_path_str, target_id, limit) {
                        Ok(nodes) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&nodes).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Get callers failed: {}", e) }] }),
                    }
                }
                "get_callees" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let source_id = args.get("source_id").and_then(|q| q.as_str()).unwrap_or("");
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
                    match crate::domains::knowledge_graph::graph_service::get_callees(&db_path_str, source_id, limit) {
                        Ok(nodes) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&nodes).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Get callees failed: {}", e) }] }),
                    }
                }
                "get_impact" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let target_id = args.get("target_id").and_then(|q| q.as_str()).unwrap_or("");
                    let depth = args.get("depth").and_then(|d| d.as_u64()).unwrap_or(3) as usize;
                    match crate::domains::knowledge_graph::graph_service::get_impact(&db_path_str, target_id, depth) {
                        Ok(nodes) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&nodes).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Get impact failed: {}", e) }] }),
                    }
                }
                "trace_path" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let function_name = args.get("function_name").and_then(|f| f.as_str()).unwrap_or("");
                    let direction = args.get("direction").and_then(|d| d.as_str()).unwrap_or("outbound");
                    let depth = args.get("depth").and_then(|d| d.as_u64()).unwrap_or(3) as usize;
                    match crate::domains::knowledge_graph::graph_service::trace_path(&db_path_str, function_name, direction, depth, "calls") {
                        Ok(trace) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&trace).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Trace path failed: {}", e) }] }),
                    }
                }
                "get_graph_schema" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    match crate::domains::knowledge_graph::graph_service::get_graph_schema(&db_path_str) {
                        Ok(schema) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&schema).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Get graph schema failed: {}", e) }] }),
                    }
                }
                "get_code_snippet" => {
                    let (safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let qualified_name = args.get("qualified_name").and_then(|q| q.as_str()).unwrap_or("");
                    let include_neighbors = args.get("include_neighbors").and_then(|b| b.as_bool()).unwrap_or(true);
                    match crate::domains::knowledge_graph::graph_service::get_code_snippet(
                        &db_path_str,
                        qualified_name,
                        include_neighbors,
                        Some(std::path::Path::new(&safe_project)),
                    ) {
                        Ok(snippet) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&snippet).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Get code snippet failed: {}", e) }] }),
                    }
                }
                "get_dead_code" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
                    match crate::domains::knowledge_graph::graph_service::get_dead_code(&db_path_str, limit) {
                        Ok(dead) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&dead).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Get dead code failed: {}", e) }] }),
                    }
                }
                "get_architecture" => {
                    let (_safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    match crate::domains::code_analysis::architecture_analyzer::ArchitectureAnalyzer::detect_layers_deterministic(&db_path_str) {
                        Ok(arch_report) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&arch_report).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Architecture analysis failed: {}", e) }] }),
                    }
                }
                "detect_changes" => {
                    let (safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    match crate::domains::version_control::diff_impact::DiffImpactAnalyzer::detect_changes(
                        std::path::Path::new(&safe_project),
                        &db_path_str,
                    ) {
                        Ok(report) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&report).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Detect changes failed: {}", e) }] }),
                    }
                }
                "manage_adr" => {
                    let (safe_project, _db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    let root = std::path::Path::new(&safe_project);
                    let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("list");
                    match action {
                        "create" => {
                            let title = args.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled ADR");
                            let status = args.get("status").and_then(|s| s.as_str()).unwrap_or("Accepted");
                            let context = args.get("context").and_then(|c| c.as_str()).unwrap_or("");
                            let decision = args.get("decision").and_then(|d| d.as_str()).unwrap_or("");
                            let consequences = args.get("consequences").and_then(|c| c.as_str()).unwrap_or("");
                            match crate::domains::project_management::adr_service::AdrService::create_adr(root, title, status, context, decision, consequences) {
                                Ok(meta) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&meta).unwrap_or_default() }] }),
                                Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Create ADR failed: {}", e) }] }),
                            }
                        }
                        "get" => {
                            let id = args.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            match crate::domains::project_management::adr_service::AdrService::get_adr(root, id) {
                                Ok(detail) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&detail).unwrap_or_default() }] }),
                                Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Get ADR failed: {}", e) }] }),
                            }
                        }
                        _ => {
                            match crate::domains::project_management::adr_service::AdrService::list_adrs(root) {
                                Ok(list) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&list).unwrap_or_default() }] }),
                                Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("List ADRs failed: {}", e) }] }),
                            }
                        }
                    }
                }
                "get_onboarding_guide" => {
                    let (safe_project, db_path_str) = match resolve_safe_project(args, config) {
                        Ok(pair) => pair,
                        Err(err_json) => return Ok(err_json),
                    };
                    match crate::domains::project_management::onboard_service::OnboardService::generate_guide(
                        std::path::Path::new(&safe_project),
                        Some(&db_path_str),
                    ) {
                        Ok(guide) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&guide).unwrap_or_default() }] }),
                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Generate onboarding guide failed: {}", e) }] }),
                    }
                }
                "get_token_savings" => {
                    let stats = crate::domains::token_compression::TokenSavingsTracker::global().get_stats();
                    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&stats).unwrap_or_default() }] })
                }
                _ => {
                    json!({ "isError": true, "content": [{ "type": "text", "text": "Unknown tool" }] })
                }
            };
            Ok(res)
        }
        _ => Err((-32601, "Method not found".to_string())),
    }
}

fn resolve_safe_project(args: &Value, config: &RuntimeConfig) -> Result<(String, String), Value> {
    let project = args
        .get("project_path")
        .and_then(|p| p.as_str())
        .unwrap_or("");
    let safe_project = if project.is_empty() {
        if !config.projects_root.as_os_str().is_empty() {
            config.projects_root.to_string_lossy().to_string()
        } else {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        }
    } else {
        let guard = match WorkspaceGuard::new(&config.projects_root) {
            Ok(g) => g,
            Err(e) => {
                return Err(json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid root: {}", e) }] }));
            }
        };
        match guard.resolve_safe(project) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                return Err(json!({ "isError": true, "content": [{ "type": "text", "text": format!("Invalid project path: {}", e) }] }));
            }
        }
    };
    let db_path = config.codebase_memory_db_path(Some(&safe_project));
    let db_path_str = db_path.to_string_lossy().to_string();
    Ok((safe_project, db_path_str))
}

