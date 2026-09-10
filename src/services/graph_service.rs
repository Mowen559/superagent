use crate::models::graph::{GraphEdge, GraphNode, GraphResponse};
use crate::services::code_intelligence::{
    CodeIntelligenceRuntime, CodeSnippetResult, DeadCodeSymbol, GraphSchema, TracePathResult,
};
use std::collections::HashSet;
use std::path::Path;

pub fn get_project_graph(
    db_path: &str,
    max_nodes: usize,
    labels: Option<&[String]>,
) -> Result<GraphResponse, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(GraphResponse {
            nodes: vec![],
            edges: vec![],
            total_nodes: 0,
        });
    }

    let nodes = rt.query_nodes(max_nodes, labels)?;
    let total = nodes.len();
    let edges = if labels.is_some() {
        let ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
        rt.query_edges(max_nodes)?
            .into_iter()
            .filter(|e| ids.contains(&e.source) && ids.contains(&e.target))
            .collect()
    } else {
        rt.query_edges(max_nodes)?
    };

    Ok(GraphResponse {
        nodes,
        edges,
        total_nodes: total,
    })
}

pub fn search_nodes(db_path: &str, query: &str, limit: usize) -> Result<Vec<GraphNode>, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() || query.trim().is_empty() {
        return Ok(Vec::new());
    }

    match rt.query_nodes_fts(query, limit) {
        Ok(nodes) if !nodes.is_empty() => Ok(nodes),
        _ => rt.search_nodes_like(query, limit),
    }
}

pub fn get_project_summary(db_path: &str) -> Result<Option<String>, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(None);
    }
    rt.get_project_summary()
}

pub fn get_file_nodes(db_path: &str, file_path: &str) -> Result<Vec<GraphNode>, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(Vec::new());
    }
    rt.get_file_nodes(file_path)
}

pub fn get_callers(db_path: &str, target_id: &str, limit: usize) -> Result<Vec<GraphNode>, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(Vec::new());
    }
    rt.get_callers(target_id, limit)
}

pub fn get_callees(db_path: &str, source_id: &str, limit: usize) -> Result<Vec<GraphNode>, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(Vec::new());
    }
    rt.get_callees(source_id, limit)
}

pub fn get_impact(db_path: &str, target_id: &str, depth: usize) -> Result<Vec<GraphNode>, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(Vec::new());
    }
    rt.get_impact(target_id, depth)
}

pub fn trace_path(
    db_path: &str,
    function_name: &str,
    direction: &str,
    depth: usize,
    mode: &str,
) -> Result<TracePathResult, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Err("Graph database not found (not indexed)".to_string());
    }
    rt.trace_path(function_name, direction, depth, mode)
}

pub fn get_graph_schema(db_path: &str) -> Result<GraphSchema, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Err("Graph database not found (not indexed)".to_string());
    }
    rt.get_graph_schema()
}

pub fn get_code_snippet(
    db_path: &str,
    qualified_name: &str,
    include_neighbors: bool,
    project_root: Option<&Path>,
) -> Result<CodeSnippetResult, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Err("Graph database not found (not indexed)".to_string());
    }
    rt.get_code_snippet(qualified_name, include_neighbors, project_root)
}

pub fn get_dead_code(db_path: &str, limit: usize) -> Result<Vec<DeadCodeSymbol>, String> {
    let rt = CodeIntelligenceRuntime::new(db_path);
    if !rt.is_indexed() {
        return Ok(Vec::new());
    }
    rt.get_dead_code(limit)
}

