//! MCP (Model Context Protocol) 工具清单与输入 Schema 定义
//! 从 `mcp_server.rs` 抽离 340 行静态协议声明，实现协议契约与分发逻辑分离。

use serde_json::{json, Value};

/// 返回 Hub 作为 MCP Server 暴露的全部工具 Schema 列表
pub fn build_tools_list_response() -> Value {
    json!({
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
                    "required": ["project_path"]
                }
            },
            {
                "name": "shadow_git_history",
                "description": "Get shadow git snapshot history",
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
                "description": "Rollback workspace to a shadow git snapshot hash",
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
                "description": "Search project supermemory for past architecture patterns or rules",
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
                "description": "Write a new memory or rule to project supermemory",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" },
                        "name": { "type": "string" },
                        "summary": { "type": "string" },
                        "tags": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["project_path", "name", "summary"]
                }
            },
            {
                "name": "capability_catalog",
                "description": "Query the Hub-Rs Capability Certification Matrix (Certified / Experimental / Disabled)",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "search_graph",
                "description": "Search codebase knowledge graph nodes by label, name_pattern (regex/substring), file_pattern, and degree filters",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Name pattern or keyword to match" },
                        "name_pattern": { "type": "string", "description": "Alias for query (regex/substring)" },
                        "label": { "type": "string", "description": "Node label filter (Function, Class, Method, Route, etc.)" },
                        "file_pattern": { "type": "string", "description": "File path substring/glob filter" },
                        "min_degree": { "type": "number", "description": "Minimum total degree (in + out)" },
                        "max_degree": { "type": "number", "description": "Maximum total degree (e.g. 0 for dead code)" },
                        "limit": { "type": "number" },
                        "offset": { "type": "number" },
                        "project_path": { "type": "string" }
                    }
                }
            },
            {
                "name": "search_code",
                "description": "Grep-like text search across all indexed project files",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Text pattern to search for" },
                        "file_pattern": { "type": "string", "description": "Optional file path filter" },
                        "limit": { "type": "number" },
                        "project_path": { "type": "string" }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "classify_update",
                "description": "Classify pending git changes to decide whether knowledge graph needs FULL_REBUILD, INCREMENTAL_UPDATE, or SKIP",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" }
                    }
                }
            },
            {
                "name": "detect_language_concepts",
                "description": "Scan project or source snippet for programming language educational concepts and idioms",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" },
                        "content": { "type": "string", "description": "Optional raw source snippet; if omitted, scans the indexed project" },
                        "limit": { "type": "number" }
                    }
                }
            },
            {
                "name": "check_graph_freshness",
                "description": "Check whether the knowledge graph SQLite index is fresh or stale relative to source file modification times",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" }
                    }
                }
            },
            {
                "name": "query_complexity",
                "description": "Analyze project structural hotspot complexity (fan-in, fan-out, coupling risk)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" },
                        "limit": { "type": "number" }
                    }
                }
            },
            {
                "name": "detect_code_clones",
                "description": "Compare two source code snippets using 64-perm MinHash Jaccard similarity to detect code clones",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "code_a": { "type": "string" },
                        "code_b": { "type": "string" }
                    },
                    "required": ["code_a", "code_b"]
                }
            },
            {
                "name": "list_indexed_projects",
                "description": "Discover and list all projects in projects_root along with their knowledge graph index status",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "pipeline_compress",
                "description": "Apply declarative regex pipeline filter rules (strip ANSI, drop patterns, keep patterns, head/tail) to noisy command output",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output": { "type": "string" },
                        "rule_name": { "type": "string", "description": "Optional builtin rule name (cargo_clippy, git_log, npm_install)" }
                    },
                    "required": ["output"]
                }
            },
            {
                "name": "query_graph",
                "description": "Execute a read-only SQL/Cypher-like query or structural filter on the knowledge graph",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "SQL SELECT query against nodes / edges tables" },
                        "project_path": { "type": "string" }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "trace_path",
                "description": "BFS multi-hop call-chain and impact tracing (inbound callers, outbound callees, or both)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "function_name": { "type": "string", "description": "Symbol name or qualified name to trace" },
                        "direction": { "type": "string", "enum": ["inbound", "outbound", "both"], "description": "Trace direction (default: both)" },
                        "depth": { "type": "number", "description": "Max BFS hops (default: 3, max: 6)" },
                        "mode": { "type": "string", "enum": ["calls", "data_flow", "cross_service", "all"], "description": "Edge filter mode (default: calls)" },
                        "project_path": { "type": "string" }
                    },
                    "required": ["function_name"]
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
    })
}
