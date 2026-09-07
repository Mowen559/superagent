use crate::models::graph::{GraphEdge, GraphNode, GraphResponse};
use crate::services::code_intelligence::CodeIntelligenceRuntime;
use std::collections::HashSet;

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
