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

const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EXTRACTED_CHARS: usize = 32 * 1024 * 1024;
const MAX_CHUNKS: usize = 4_096;
const MAX_QUERY_CHARS: usize = 4_000;
const MAX_CONTEXT_CHARS: usize = 800;
const DEFAULT_CHUNK_CHARS: usize = 1_800;

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
enum DocumentFormat {
    Text,
    Markdown,
    Docx,
    Pdf,
}

impl DocumentFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Markdown => "markdown",
            Self::Docx => "docx",
            Self::Pdf => "pdf",
        }
    }
}

enum ExtractedContent {
    Text(String),
    Pages(Vec<String>),
}

struct ExtractedDocument {
    format: DocumentFormat,
    content: ExtractedContent,
}

trait DocumentExtractor: Send + Sync {
    fn supports(&self, extension: &str) -> bool;
    fn extract(
        &self,
        bytes: &[u8],
        extension: &str,
    ) -> Result<ExtractedDocument, KnowledgeCorpusError>;
}

struct TextDocumentAdapter;
struct DocxDocumentAdapter;
struct PdfDocumentAdapter;

impl DocumentExtractor for TextDocumentAdapter {
    fn supports(&self, extension: &str) -> bool {
        matches!(
            extension,
            "" | "txt"
                | "md"
                | "markdown"
                | "rst"
                | "adoc"
                | "org"
                | "json"
                | "jsonl"
                | "yaml"
                | "yml"
                | "toml"
                | "csv"
                | "tsv"
                | "html"
                | "htm"
                | "xml"
                | "sql"
                | "rs"
                | "py"
                | "js"
                | "jsx"
                | "ts"
                | "tsx"
                | "java"
                | "kt"
                | "kts"
                | "go"
                | "c"
                | "cc"
                | "cpp"
                | "h"
                | "hpp"
                | "cs"
                | "sh"
                | "ps1"
        )
    }

    fn extract(
        &self,
        bytes: &[u8],
        extension: &str,
    ) -> Result<ExtractedDocument, KnowledgeCorpusError> {
        let text = decode_text(bytes)?
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let format = if matches!(extension, "md" | "markdown") {
            DocumentFormat::Markdown
        } else {
            DocumentFormat::Text
        };
        Ok(ExtractedDocument {
            format,
            content: ExtractedContent::Text(text),
        })
    }
}

impl DocumentExtractor for DocxDocumentAdapter {
    fn supports(&self, extension: &str) -> bool {
        extension == "docx"
    }

    fn extract(
        &self,
        bytes: &[u8],
        _extension: &str,
    ) -> Result<ExtractedDocument, KnowledgeCorpusError> {
        let cursor = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()))?;
        let mut document_xml = String::new();
        let mut document_entry = archive.by_name("word/document.xml").map_err(|error| {
            KnowledgeCorpusError::Extraction(format!("DOCX is missing word/document.xml: {error}"))
        })?;
        if document_entry.size() > MAX_EXTRACTED_CHARS as u64 {
            return Err(KnowledgeCorpusError::ExtractedTextTooLarge {
                actual: document_entry.size() as usize,
                maximum: MAX_EXTRACTED_CHARS,
            });
        }
        document_entry
            .read_to_string(&mut document_xml)
            .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()))?;
        Ok(ExtractedDocument {
            format: DocumentFormat::Docx,
            content: ExtractedContent::Text(parse_docx_xml(&document_xml)?),
        })
    }
}

impl DocumentExtractor for PdfDocumentAdapter {
    fn supports(&self, extension: &str) -> bool {
        extension == "pdf"
    }

    fn extract(
        &self,
        bytes: &[u8],
        _extension: &str,
    ) -> Result<ExtractedDocument, KnowledgeCorpusError> {
        let pages = pdf_extract::extract_text_from_mem_by_pages(bytes)
            .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()))?;
        Ok(ExtractedDocument {
            format: DocumentFormat::Pdf,
            content: ExtractedContent::Pages(pages),
        })
    }
}

/// Deep Module that owns the Knowledge Document lifecycle. Callers do not
/// understand extraction formats, Manifest layout, chunk identity, embedding
/// policy, atomic replacement or Citation construction.
pub struct KnowledgeCorpusRuntime {
    workspace_root: PathBuf,
    store: SupermemoryStore,
    chunker: ContextualChunker,
    extractors: Vec<Box<dyn DocumentExtractor>>,
    embeddings_enabled: bool,
}

impl KnowledgeCorpusRuntime {
    pub fn new(config: &RuntimeConfig) -> Result<Self, KnowledgeCorpusError> {
        let embeddings_enabled = embeddings_enabled();
        Self::with_store(
            config.project_root.clone(),
            SupermemoryStore::new(config, None),
            embeddings_enabled,
        )
    }

    /// Build a project-scoped corpus while retaining the same extraction,
    /// chunking, manifest and citation invariants as the global runtime.
    pub fn for_workspace(
        config: &RuntimeConfig,
        workspace_root: impl AsRef<Path>,
    ) -> Result<Self, KnowledgeCorpusError> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let project = workspace_root.to_string_lossy().to_string();
        Self::with_store(
            workspace_root,
            SupermemoryStore::new(config, Some(&project)),
            embeddings_enabled(),
        )
    }

    fn with_store(
        workspace_root: PathBuf,
        store: SupermemoryStore,
        embeddings_enabled: bool,
    ) -> Result<Self, KnowledgeCorpusError> {
        let workspace_root = workspace_root.canonicalize().map_err(|error| {
            KnowledgeCorpusError::Workspace(format!("{}: {error}", workspace_root.display()))
        })?;
        Ok(Self {
            workspace_root,
            store,
            chunker: ContextualChunker::new(DEFAULT_CHUNK_CHARS),
            extractors: vec![
                Box::new(DocxDocumentAdapter),
                Box::new(PdfDocumentAdapter),
                Box::new(TextDocumentAdapter),
            ],
            embeddings_enabled,
        })
    }

    pub fn ingest(
        &self,
        path: impl AsRef<Path>,
        global_context: Option<&str>,
    ) -> Result<IngestReceipt, KnowledgeCorpusError> {
        self.ingest_internal(path.as_ref(), global_context, false)
    }

    pub fn supports_file(&self, path: impl AsRef<Path>) -> bool {
        let extension = path
            .as_ref()
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        self.extractors
            .iter()
            .any(|extractor| extractor.supports(&extension))
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeHit>, KnowledgeCorpusError> {
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(KnowledgeCorpusError::QueryTooLong);
        }
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let documents = self
            .store
            .knowledge_documents()
            .map_err(KnowledgeCorpusError::Store)?;
        let mut manifest = HashMap::new();
        let mut allowed_ids = HashSet::new();
        for document in &documents {
            for chunk in &document.chunks {
                allowed_ids.insert(chunk.record_id.clone());
                manifest.insert(chunk.record_id.as_str(), (document, chunk));
            }
        }
        let query_embedding = self.best_effort_embedding(query);
        let scored = self
            .store
            .search_scored_ids_result(
                query,
                limit.clamp(1, 50),
                query_embedding.as_deref(),
                &allowed_ids,
            )
            .map_err(KnowledgeCorpusError::Store)?;
        Ok(scored
            .into_iter()
            .filter_map(|hit| {
                let (document, chunk) = manifest.get(hit.record.id.as_str())?;
                Some(KnowledgeHit {
                    document_id: document.document_id.clone(),
                    document_version: document.content_hash.clone(),
                    record_id: hit.record.id,
                    source_path: document.source_path.clone(),
                    citation: chunk.citation.clone(),
                    heading_path: chunk.heading_path.clone(),
                    content: hit.record.summary,
                    score: round_score(hit.score),
                    keyword_score: round_score(hit.keyword_score),
                    vector_score: hit.vector_score.map(round_score),
                })
            })
            .collect())
    }

    pub fn list_documents(&self) -> Result<Vec<KnowledgeDocumentRecord>, KnowledgeCorpusError> {
        self.store
            .knowledge_documents()
            .map_err(KnowledgeCorpusError::Store)
    }

    pub fn remove(&self, reference: &str) -> Result<RemovalReceipt, KnowledgeCorpusError> {
        let document = self.resolve_document(reference)?;
        let removed = self
            .store
            .delete_knowledge_document(&document.document_id)
            .map_err(KnowledgeCorpusError::Store)?
            .ok_or_else(|| KnowledgeCorpusError::NotFound(reference.to_string()))?;
        Ok(RemovalReceipt {
            document_id: removed.document_id,
            source_path: removed.source_path,
            removed_chunks: removed.chunks.len(),
        })
    }

    pub fn rebuild(&self, reference: &str) -> Result<IngestReceipt, KnowledgeCorpusError> {
        let document = self.resolve_document(reference)?;
        self.ingest_internal(
            Path::new(&document.source_path),
            Some(&document.global_context),
            true,
        )
    }

    pub fn rebuild_all(&self) -> Result<RebuildReceipt, KnowledgeCorpusError> {
        let documents = self.list_documents()?;
        let mut rebuilt = Vec::new();
        let mut failures = Vec::new();
        for document in &documents {
            match self.ingest_internal(
                Path::new(&document.source_path),
                Some(&document.global_context),
                true,
            ) {
                Ok(receipt) => rebuilt.push(receipt),
                Err(error) => failures.push(RebuildFailure {
                    document_id: document.document_id.clone(),
                    source_path: document.source_path.clone(),
                    error: error.to_string(),
                }),
            }
        }
        Ok(RebuildReceipt {
            attempted: documents.len(),
            rebuilt,
            failures,
        })
    }

    fn ingest_internal(
        &self,
        path: &Path,
        global_context: Option<&str>,
        force: bool,
    ) -> Result<IngestReceipt, KnowledgeCorpusError> {
        let canonical = self.resolve_workspace_file(path)?;
        let metadata = canonical
            .metadata()
            .map_err(|error| KnowledgeCorpusError::InvalidDocument(error.to_string()))?;
        if metadata.len() > MAX_DOCUMENT_BYTES {
            return Err(KnowledgeCorpusError::DocumentTooLarge {
                actual: metadata.len(),
                maximum: MAX_DOCUMENT_BYTES,
            });
        }
        let bytes = std::fs::read(&canonical)
            .map_err(|error| KnowledgeCorpusError::InvalidDocument(error.to_string()))?;
        let content_hash = sha256_hex(&bytes);
        let source_path = canonical.to_string_lossy().to_string();
        let existing = self
            .store
            .knowledge_document_by_source(&source_path)
            .map_err(KnowledgeCorpusError::Store)?;
        if !force
            && existing
                .as_ref()
                .is_some_and(|document| document.content_hash == content_hash)
        {
            return Ok(receipt_for(
                existing.as_ref().unwrap(),
                IngestDisposition::Unchanged,
            ));
        }

        let extension = canonical
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let extractor = self
            .extractors
            .iter()
            .find(|extractor| extractor.supports(&extension))
            .ok_or_else(|| KnowledgeCorpusError::UnsupportedFormat(extension.clone()))?;
        let extracted = extractor.extract(&bytes, &extension)?;
        let extracted_chars = match &extracted.content {
            ExtractedContent::Text(text) => text.chars().count(),
            ExtractedContent::Pages(pages) => pages.iter().map(|page| page.chars().count()).sum(),
        };
        if extracted_chars > MAX_EXTRACTED_CHARS {
            return Err(KnowledgeCorpusError::ExtractedTextTooLarge {
                actual: extracted_chars,
                maximum: MAX_EXTRACTED_CHARS,
            });
        }
        let display_name = canonical
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("document")
            .to_string();
        let derived_context = derive_global_context(&display_name, &extracted.content);
        let global_context = global_context
            .filter(|value| !value.trim().is_empty())
            .map(normalize_context)
            .unwrap_or(derived_context);
        let chunks = match &extracted.content {
            ExtractedContent::Text(text) => self.chunker.chunk_text(text, &global_context),
            ExtractedContent::Pages(pages) => self.chunker.chunk_pages(pages, &global_context),
        };
        if chunks.is_empty() {
            return Err(KnowledgeCorpusError::EmptyDocument);
        }
        if chunks.len() > MAX_CHUNKS {
            return Err(KnowledgeCorpusError::TooManyChunks {
                actual: chunks.len(),
                maximum: MAX_CHUNKS,
            });
        }

        let document_id = format!(
            "doc-{:016x}",
            xxhash_rust::xxh3::xxh3_64(source_path.as_bytes())
        );
        let revision = match existing.as_ref() {
            Some(document) if document.content_hash == content_hash => document.revision,
            Some(document) => document.revision.saturating_add(1),
            None => 1,
        };
        let rendered = chunks
            .iter()
            .map(ContextualChunk::to_llm_string)
            .collect::<Vec<_>>();
        let embeddings = self.best_effort_embeddings(&rendered);
        let updated_at = chrono::Utc::now().to_rfc3339();
        let mut chunk_manifest = Vec::with_capacity(chunks.len());
        let mut records = Vec::with_capacity(chunks.len());
        for (chunk, content) in chunks.into_iter().zip(rendered) {
            let record_id = format!(
                "knowledge:{}:{}:{}",
                document_id,
                &content_hash[..16],
                chunk.chunk_index
            );
            let citation = citation_for(&source_path, extracted.format, &chunk);
            let mut tags = vec![
                "knowledge".to_string(),
                "ingested".to_string(),
                format!("document:{document_id}"),
                format!("format:{}", extracted.format.as_str()),
            ];
            if let Some(heading) = chunk.heading_path.last() {
                tags.push(format!("heading:{}", truncate_chars(heading, 80)));
            }
            let embedding = embeddings
                .as_ref()
                .and_then(|values| values.get(chunk.chunk_index).cloned());
            records.push(SupermemoryRecord {
                id: record_id.clone(),
                file_path: source_path.clone(),
                name: format!("{} (Chunk {})", display_name, chunk.chunk_index + 1),
                summary: content,
                tags,
                complexity: "knowledge_node".to_string(),
                node_type: "document_chunk".to_string(),
                source_graph_path: format!("knowledge://{}/{}", document_id, chunk.chunk_index),
                source_label: citation.clone(),
                updated_at: updated_at.clone(),
                embedding,
            });
            chunk_manifest.push(KnowledgeChunkRecord {
                record_id,
                chunk_index: chunk.chunk_index,
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                start_paragraph: chunk.start_paragraph,
                end_paragraph: chunk.end_paragraph,
                start_page: chunk.start_page,
                end_page: chunk.end_page,
                heading_path: chunk.heading_path,
                citation,
            });
        }

        let disposition = if force {
            IngestDisposition::Rebuilt
        } else if existing.is_some() {
            IngestDisposition::Updated
        } else {
            IngestDisposition::Inserted
        };
        let document = KnowledgeDocumentRecord {
            document_id,
            source_path,
            display_name,
            content_hash,
            revision,
            format: extracted.format.as_str().to_string(),
            byte_size: metadata.len(),
            global_context,
            updated_at,
            chunks: chunk_manifest,
        };
        self.store
            .replace_knowledge_document(document.clone(), records)
            .map_err(KnowledgeCorpusError::Store)?;
        Ok(receipt_for(&document, disposition))
    }

    fn resolve_workspace_file(&self, path: &Path) -> Result<PathBuf, KnowledgeCorpusError> {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };
        let canonical = candidate.canonicalize().map_err(|error| {
            KnowledgeCorpusError::InvalidDocument(format!("{}: {error}", candidate.display()))
        })?;
        if !canonical.starts_with(&self.workspace_root) {
            return Err(KnowledgeCorpusError::OutsideWorkspace(
                canonical.display().to_string(),
            ));
        }
        if !canonical.is_file() {
            return Err(KnowledgeCorpusError::InvalidDocument(
                canonical.display().to_string(),
            ));
        }
        Ok(canonical)
    }

    fn resolve_document(
        &self,
        reference: &str,
    ) -> Result<KnowledgeDocumentRecord, KnowledgeCorpusError> {
        let documents = self.list_documents()?;
        if let Some(document) = documents
            .iter()
            .find(|document| document.document_id == reference || document.source_path == reference)
        {
            return Ok(document.clone());
        }
        let canonical = self.resolve_workspace_file(Path::new(reference)).ok();
        canonical
            .and_then(|path| {
                let source = path.to_string_lossy();
                documents
                    .into_iter()
                    .find(|document| document.source_path == source)
            })
            .ok_or_else(|| KnowledgeCorpusError::NotFound(reference.to_string()))
    }

    fn best_effort_embedding(&self, text: &str) -> Option<Vec<f32>> {
        if !self.embeddings_enabled {
            return None;
        }
        crate::services::brain::embedding_service::get_embedding_service()
            .ok()
            .and_then(|service| {
                use crate::services::brain::embedding_service::EmbeddingService;
                service.embed_text(text).ok()
            })
    }

    fn best_effort_embeddings(&self, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        if !self.embeddings_enabled || texts.is_empty() {
            return None;
        }
        crate::services::brain::embedding_service::get_embedding_service()
            .ok()
            .and_then(|service| {
                use crate::services::brain::embedding_service::EmbeddingService;
                let borrowed = texts.iter().map(String::as_str).collect::<Vec<_>>();
                service.embed_batch(borrowed).ok()
            })
            .filter(|embeddings| embeddings.len() == texts.len())
    }
}

fn embeddings_enabled() -> bool {
    std::env::var("HUB_KNOWLEDGE_EMBEDDINGS")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "off" | "false"
            )
        })
        .unwrap_or(true)
}

pub fn shared_runtime(config: &RuntimeConfig) -> Result<Arc<KnowledgeCorpusRuntime>, String> {
    KnowledgeCorpusRuntime::new(config)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

fn decode_text(bytes: &[u8]) -> Result<String, KnowledgeCorpusError> {
    if let Some(payload) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(payload.to_vec())
            .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()));
    }
    if let Some(payload) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        if payload.len() % 2 != 0 {
            return Err(KnowledgeCorpusError::Extraction(
                "invalid UTF-16LE byte length".to_string(),
            ));
        }
        let units = payload
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16(&units)
            .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()));
    }
    if let Some(payload) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        if payload.len() % 2 != 0 {
            return Err(KnowledgeCorpusError::Extraction(
                "invalid UTF-16BE byte length".to_string(),
            ));
        }
        let units = payload
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16(&units)
            .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()))
}

fn parse_docx_xml(xml: &str) -> Result<String, KnowledgeCorpusError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut paragraphs = Vec::new();
    let mut paragraph = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => paragraph.push_str(
                &text
                    .unescape()
                    .map_err(|error| KnowledgeCorpusError::Extraction(error.to_string()))?,
            ),
            Ok(Event::Empty(tag)) if matches!(tag.name().as_ref(), b"w:tab" | b"w:br") => {
                paragraph.push(' ')
            }
            Ok(Event::End(tag)) if tag.name().as_ref() == b"w:p" => {
                let value = paragraph.trim();
                if !value.is_empty() {
                    paragraphs.push(value.to_string());
                }
                paragraph.clear();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(KnowledgeCorpusError::Extraction(format!(
                    "invalid DOCX XML: {error}"
                )))
            }
        }
    }
    Ok(paragraphs.join("\n\n"))
}

fn derive_global_context(display_name: &str, content: &ExtractedContent) -> String {
    let preview = match content {
        ExtractedContent::Text(text) => text.as_str(),
        ExtractedContent::Pages(pages) => pages.first().map(String::as_str).unwrap_or(""),
    };
    let preview = preview
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    normalize_context(&format!(
        "Document {display_name}. {}",
        truncate_chars(&preview, 500)
    ))
}

fn normalize_context(value: &str) -> String {
    truncate_chars(
        &value.split_whitespace().collect::<Vec<_>>().join(" "),
        MAX_CONTEXT_CHARS,
    )
}

fn citation_for(source: &str, format: DocumentFormat, chunk: &ContextualChunk) -> String {
    match (chunk.start_page, chunk.end_page) {
        (Some(start), Some(end)) if start == end => format!("{source}#page={start}"),
        (Some(start), Some(end)) => format!("{source}#page={start}-{end}"),
        _ if format == DocumentFormat::Docx => format!(
            "{source}#paragraph={}-{}",
            chunk.start_paragraph, chunk.end_paragraph
        ),
        _ => format!("{source}#L{}-L{}", chunk.start_line, chunk.end_line),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn receipt_for(
    document: &KnowledgeDocumentRecord,
    disposition: IngestDisposition,
) -> IngestReceipt {
    IngestReceipt {
        disposition,
        document_id: document.document_id.clone(),
        source_path: document.source_path.clone(),
        content_hash: document.content_hash.clone(),
        revision: document.revision,
        format: document.format.clone(),
        byte_size: document.byte_size,
        chunk_count: document.chunks.len(),
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn round_score(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as FmtWrite;
    use std::io::Write as IoWrite;

    fn runtime(temp: &tempfile::TempDir) -> KnowledgeCorpusRuntime {
        KnowledgeCorpusRuntime::with_store(
            temp.path().to_path_buf(),
            SupermemoryStore::from_path(temp.path().join(".supermemory/store.json")),
            false,
        )
        .unwrap()
    }

    #[test]
    fn ingest_is_idempotent_and_update_replaces_old_version() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("guide.md");
        std::fs::write(&path, "# Guide\n\natomic manifest").unwrap();
        let corpus = runtime(&temp);

        let inserted = corpus.ingest(&path, None).unwrap();
        let unchanged = corpus.ingest(&path, None).unwrap();
        assert_eq!(inserted.disposition, IngestDisposition::Inserted);
        assert_eq!(unchanged.disposition, IngestDisposition::Unchanged);
        assert_eq!(inserted.content_hash, unchanged.content_hash);

        std::fs::write(&path, "# Guide\n\nupdated citation content").unwrap();
        let updated = corpus.ingest(&path, None).unwrap();
        assert_eq!(updated.disposition, IngestDisposition::Updated);
        assert_eq!(updated.revision, 2);
        let data = corpus.store.load_result().unwrap();
        assert_eq!(data.corpus.documents.len(), 1);
        assert_eq!(
            data.memories
                .iter()
                .filter(|record| record.node_type == "document_chunk")
                .count(),
            updated.chunk_count
        );
    }

    #[test]
    fn retrieval_returns_scores_and_line_citation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("guide.md");
        std::fs::write(&path, "# Runtime\n\ncorpus manifest is atomic").unwrap();
        let corpus = runtime(&temp);
        corpus.ingest(&path, None).unwrap();

        let hits = corpus.search("corpus manifest", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score > 0.0);
        assert!(hits[0].citation.contains("#L"));
        assert_eq!(hits[0].heading_path, vec!["Runtime"]);
    }

    #[test]
    fn delete_and_rebuild_are_manifest_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("guide.txt");
        std::fs::write(&path, "knowledge corpus").unwrap();
        let corpus = runtime(&temp);
        let inserted = corpus.ingest(&path, None).unwrap();
        let rebuilt = corpus.rebuild(&inserted.document_id).unwrap();
        assert_eq!(rebuilt.disposition, IngestDisposition::Rebuilt);
        let removed = corpus.remove(&inserted.document_id).unwrap();
        assert_eq!(removed.removed_chunks, inserted.chunk_count);
        assert!(corpus.list_documents().unwrap().is_empty());
    }

    #[test]
    fn workspace_guard_rejects_external_document() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let path = external.path().join("outside.md");
        std::fs::write(&path, "outside").unwrap();
        let corpus = runtime(&workspace);
        assert!(matches!(
            corpus.ingest(&path, None),
            Err(KnowledgeCorpusError::OutsideWorkspace(_))
        ));
    }

    #[test]
    fn docx_xml_preserves_paragraphs_and_entities() {
        let xml = r#"<w:document xmlns:w="urn:test"><w:body><w:p><w:r><w:t>A &amp; B</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:body></w:document>"#;
        assert_eq!(parse_docx_xml(xml).unwrap(), "A & B\n\nSecond");
    }

    #[test]
    fn docx_adapter_reads_the_real_archive_seam() {
        let xml = r#"<w:document xmlns:w="urn:test"><w:body><w:p><w:r><w:t>Corpus DOCX</w:t></w:r></w:p></w:body></w:document>"#;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(xml.as_bytes()).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let extracted = DocxDocumentAdapter.extract(&bytes, "docx").unwrap();
        match extracted.content {
            ExtractedContent::Text(text) => assert_eq!(text, "Corpus DOCX"),
            ExtractedContent::Pages(_) => panic!("DOCX must produce paragraph text"),
        }
    }

    #[test]
    fn pdf_adapter_extracts_text_with_page_locality() {
        let bytes = minimal_pdf("Knowledge corpus PDF");
        let extracted = PdfDocumentAdapter.extract(&bytes, "pdf").unwrap();
        match extracted.content {
            ExtractedContent::Pages(pages) => {
                assert_eq!(pages.len(), 1);
                assert!(pages[0].contains("Knowledge corpus PDF"));
            }
            ExtractedContent::Text(_) => panic!("PDF must preserve page locality"),
        }
    }

    fn minimal_pdf(text: &str) -> Vec<u8> {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        let stream = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET");
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", stream.len(), stream),
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            writeln!(&mut pdf, "{} 0 obj\n{}\nendobj", index + 1, object).unwrap();
        }
        let xref = pdf.len();
        writeln!(&mut pdf, "xref\n0 {}", objects.len() + 1).unwrap();
        pdf.push_str("0000000000 65535 f \n");
        for offset in offsets {
            writeln!(&mut pdf, "{offset:010} 00000 n ").unwrap();
        }
        write!(
            &mut pdf,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref
        )
        .unwrap();
        pdf.into_bytes()
    }
}
