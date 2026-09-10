use crate::config::runtime::RuntimeConfig;
use crate::models::memory::{KnowledgeChunkRecord, KnowledgeDocumentRecord, SupermemoryRecord};
use crate::services::brain::contextual_chunker::{ContextualChunk, ContextualChunker};
use crate::services::supermemory_store::SupermemoryStore;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;


use super::*;

pub(crate) const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_EXTRACTED_CHARS: usize = 32 * 1024 * 1024;
pub(crate) const MAX_CHUNKS: usize = 4_096;
pub(crate) const MAX_QUERY_CHARS: usize = 4_000;
pub(crate) const MAX_CONTEXT_CHARS: usize = 800;
pub(crate) const DEFAULT_CHUNK_CHARS: usize = 1_800;

#[derive(Debug, thiserror::Error)]
pub enum KnowledgeCorpusError {
    #[error("workspace root is unavailable: {0}")]
    Workspace(String),
    #[error("document is outside the configured workspace: {0}")]
    OutsideWorkspace(String),
    #[error("document does not exist or is not a regular file: {0}")]
    InvalidDocument(String),
    #[error("document is {actual} bytes; maximum is {maximum} bytes")]
    DocumentTooLarge { actual: u64, maximum: u64 },
    #[error("unsupported document format: {0}")]
    UnsupportedFormat(String),
    #[error("failed to extract document: {0}")]
    Extraction(String),
    #[error("document contains no extractable text")]
    EmptyDocument,
    #[error("extracted text is {actual} characters; maximum is {maximum} characters")]
    ExtractedTextTooLarge { actual: usize, maximum: usize },
    #[error("document produced {actual} chunks; maximum is {maximum}")]
    TooManyChunks { actual: usize, maximum: usize },
    #[error("knowledge query exceeds {MAX_QUERY_CHARS} characters")]
    QueryTooLong,
    #[error("knowledge document was not found: {0}")]
    NotFound(String),
    #[error("knowledge store failure: {0}")]
    Store(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestDisposition {
    Inserted,
    Updated,
    Unchanged,
    Rebuilt,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReceipt {
    pub disposition: IngestDisposition,
    pub document_id: String,
    pub source_path: String,
    pub content_hash: String,
    pub revision: u64,
    pub format: String,
    pub byte_size: u64,
    pub chunk_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalReceipt {
    pub document_id: String,
    pub source_path: String,
    pub removed_chunks: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildFailure {
    pub document_id: String,
    pub source_path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildReceipt {
    pub attempted: usize,
    pub rebuilt: Vec<IngestReceipt>,
    pub failures: Vec<RebuildFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeHit {
    pub document_id: String,
    pub document_version: String,
    pub record_id: String,
    pub source_path: String,
    pub citation: String,
    pub heading_path: Vec<String>,
    pub content: String,
    pub score: f32,
    pub keyword_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_score: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentFormat {
    Text,
    Markdown,
    Docx,
    Pdf,
}

impl DocumentFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Markdown => "markdown",
            Self::Docx => "docx",
            Self::Pdf => "pdf",
        }
    }
}

pub(crate) enum ExtractedContent {
    Text(String),
    Pages(Vec<String>),
}

pub(crate) struct ExtractedDocument {
    pub(crate) format: DocumentFormat,
    pub(crate) content: ExtractedContent,
}

