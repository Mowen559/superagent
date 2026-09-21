//! Semantic vector search over the codebase-memory-mcp `node_vectors` table.
//!
//! Vectors are 768-dim int8-quantized embeddings. Query vectors are synthesized
//! the same way CBM does it (see src/store/store.c `cbm_store_vector_search`):
//!   1. per keyword, look up the enriched vector in `token_vectors` (dequantize /127)
//!   2. fall back to XXH3 sparse random indexing for unknown tokens
//!   3. L2-normalize + int8-quantize each keyword vector
//!   4. score = min over keywords of cosine(keyword_vec, node_vec)

use crate::domains::knowledge_graph::graph_service;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::path::Path;
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

const VEC_DIM: usize = 768;
const SPARSE_NNZE: u32 = 8;
const RI_SEED: u64 = 0x52494E44;
const INT8_MAX: f32 = 127.0;
const MAX_KW: usize = 32;
const DENOM_EPS: f64 = 1e-10;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticHit {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub label: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub score: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridHit {
    pub id: String,
    pub name: String,
    pub label: String,
    pub file_path: String,
    pub score: f64,
    /// Which retrievers surfaced this hit: "fts", "semantic", or both.
    pub sources: Vec<&'static str>,
}

/// Hybrid retrieval: FTS5 keyword search + semantic vector search, fused with
/// Reciprocal Rank Fusion (score = sum of 1/(K + rank) over each result list).
pub fn hybrid_search(db_path: &str, query: &str, limit: usize) -> Result<Vec<HybridHit>, String> {
    const RRF_K: f64 = 60.0;
    let fetch = limit.max(1) * 2;

    let fts_hits = graph_service::search_nodes(db_path, query, fetch).unwrap_or_default();
    let sem_hits = semantic_search(db_path, query, fetch)?;

    let mut fused: std::collections::HashMap<String, HybridHit> = std::collections::HashMap::new();
    for (rank, n) in fts_hits.iter().enumerate() {
        let entry = fused.entry(n.id.clone()).or_insert_with(|| HybridHit {
            id: n.id.clone(),
            // GraphNode uses `label` for the display name and `node_type` for the kind.
            name: n.label.clone(),
            label: n.node_type.clone(),
            file_path: n.file_path.clone().unwrap_or_default(),
            score: 0.0,
            sources: Vec::new(),
        });
        entry.score += 1.0 / (RRF_K + rank as f64 + 1.0);
        entry.sources.push("fts");
    }
    for (rank, h) in sem_hits.into_iter().enumerate() {
        let entry = fused.entry(h.id.clone()).or_insert_with(|| HybridHit {
            id: h.id,
            name: h.name,
            label: h.label,
            file_path: h.file_path,
            score: 0.0,
            sources: Vec::new(),
        });
        entry.score += 1.0 / (RRF_K + rank as f64 + 1.0);
        entry.sources.push("semantic");
    }

    let mut hits: Vec<HybridHit> = fused.into_values().collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(limit);
    Ok(hits)
}

/// Split a query into lowercase tokens, breaking on non-alphanumerics and camelCase.
pub fn tokenize(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in query.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            prev_lower = c.is_lowercase() || c.is_numeric();
            current.extend(c.to_lowercase());
        } else {
            prev_lower = false;
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens.dedup();
    tokens
}

/// Semantic search: returns Function/Method/Class nodes ranked by min-cosine similarity.
pub fn semantic_search(
    db_path: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<SemanticHit>, String> {
    if !Path::new(db_path).exists() {
        return Ok(Vec::new());
    }
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("Failed to open graph DB: {}", e))?;

    // Each CBM database holds a single project; its name scopes the vector tables.
    let project: String = conn
        .query_row("SELECT name FROM projects LIMIT 1", [], |row| row.get(0))
        .map_err(|e| format!("Failed to resolve project name: {}", e))?;

    let kw_vecs = build_keyword_vectors(&conn, &project, &tokens);
    if kw_vecs.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT n.id, n.name, n.qualified_name, n.label, n.file_path, \
                    n.start_line, n.end_line, v.vector \
             FROM node_vectors v \
             INNER JOIN nodes n ON n.id = v.node_id \
             WHERE v.project = ?1 AND n.label IN ('Function','Method','Class')",
        )
        .map_err(|e| e.to_string())?;

    let mut hits: Vec<SemanticHit> = stmt
        .query_map(rusqlite::params![project], |row| {
            let vector: Vec<u8> = row.get(7)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                vector,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter_map(
            |(id, name, qualified_name, label, file_path, start_line, end_line, vector)| {
                let score = min_cosine_score(&vector, &kw_vecs)?;
                Some(SemanticHit {
                    id: id.to_string(),
                    name,
                    qualified_name,
                    label,
                    file_path,
                    start_line,
                    end_line,
                    score,
                })
            },
        )
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(limit);
    Ok(hits)
}

/// Build one int8 query vector per token; tokens producing zero vectors are skipped.
fn build_keyword_vectors(
    conn: &Connection,
    project: &str,
    tokens: &[String],
) -> Vec<[i8; VEC_DIM]> {
    let mut vecs = Vec::new();
    for token in tokens.iter().take(MAX_KW) {
        let mut f = [0f32; VEC_DIM];
        if !load_enriched_vector(conn, project, token, &mut f) {
            fill_sparse_random(token, &mut f);
        }
        if let Some(q) = normalize_and_quantize(&f) {
            vecs.push(q);
        }
    }
    vecs
}

/// Look up the enriched int8 vector for `token` and dequantize it.
fn load_enriched_vector(
    conn: &Connection,
    project: &str,
    token: &str,
    out: &mut [f32; VEC_DIM],
) -> bool {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT vector FROM token_vectors WHERE project = ?1 AND token = ?2 LIMIT 1",
            rusqlite::params![project, token],
            |row| row.get(0),
        )
        .ok();
    match blob {
        Some(b) if b.len() == VEC_DIM => {
            for (d, byte) in b.iter().enumerate() {
                out[d] = (*byte as i8) as f32 / INT8_MAX;
            }
            true
        }
        _ => false,
    }
}

/// XXH3 sparse random indexing, bit-compatible with CBM's `vs_fill_sparse_random`.
fn fill_sparse_random(token: &str, out: &mut [f32; VEC_DIM]) {
    let seed = xxh3_64(token.as_bytes());
    for i in 0..SPARSE_NNZE {
        let h = xxh3_64_with_seed(&(i as i32).to_le_bytes(), seed.wrapping_add(RI_SEED));
        let pos = (h % VEC_DIM as u64) as usize;
        let sign = if h & 1 != 0 { 1.0f32 } else { -1.0f32 };
        out[pos] += sign;
    }
}

/// L2-normalize and quantize to int8; returns None for effectively-zero vectors.
fn normalize_and_quantize(src: &[f32; VEC_DIM]) -> Option<[i8; VEC_DIM]> {
    let mag = src.iter().map(|v| v * v).sum::<f32>().sqrt();
    if (mag as f64) < DENOM_EPS {
        return None;
    }
    let inv = 1.0 / mag;
    let mut dst = [0i8; VEC_DIM];
    for d in 0..VEC_DIM {
        dst[d] = (src[d] * inv * INT8_MAX).clamp(-INT8_MAX, INT8_MAX) as i8;
    }
    Some(dst)
}

/// min over keywords of cosine(kw, node); None when the node vector is mis-sized.
fn min_cosine_score(node_vec: &[u8], kw_vecs: &[[i8; VEC_DIM]]) -> Option<f64> {
    if node_vec.len() != VEC_DIM {
        return None;
    }
    let mut min_score = 1.0f64;
    for kw in kw_vecs {
        let mut dot = 0i32;
        let mut ma = 0i32;
        let mut mb = 0i32;
        for d in 0..VEC_DIM {
            let a = kw[d] as i32;
            let b = (node_vec[d] as i8) as i32;
            dot += a * b;
            ma += a * a;
            mb += b * b;
        }
        let denom = (ma as f64).sqrt() * (mb as f64).sqrt();
        let cos_k = if denom > DENOM_EPS {
            dot as f64 / denom
        } else {
            0.0
        };
        if cos_k < min_score {
            min_score = cos_k;
        }
    }
    Some(min_score)
}
