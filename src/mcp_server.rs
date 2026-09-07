use crate::config::runtime::RuntimeConfig;
use crate::services::git_service;
use crate::domains::model_gateway::providers::LlmProvider;
use crate::services::requirement_analyzer::RequirementAnalyzer;
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
                        crate::services::code_intelligence::CodeIntelligenceRuntime::new(&db_path);
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
                _ => {
                    json!({ "isError": true, "content": [{ "type": "text", "text": "Unknown tool" }] })
                }
            };
            Ok(res)
        }
        _ => Err((-32601, "Method not found".to_string())),
    }
}
