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
use super::types::*;

pub struct KnowledgeCorpusRuntime {
    pub(crate) workspace_root: PathBuf,
    pub(crate) store: SupermemoryStore,
    pub(crate) chunker: ContextualChunker,
    pub(crate) extractors: Vec<Box<dyn DocumentExtractor>>,
    pub(crate) embeddings_enabled: bool,
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

    pub(crate) fn with_store(
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

    pub(crate) fn ingest_internal(
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

    pub(crate) fn resolve_workspace_file(&self, path: &Path) -> Result<PathBuf, KnowledgeCorpusError> {
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

    pub(crate) fn resolve_document(
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

    pub(crate) fn best_effort_embedding(&self, text: &str) -> Option<Vec<f32>> {
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

    pub(crate) fn best_effort_embeddings(&self, texts: &[String]) -> Option<Vec<Vec<f32>>> {
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

pub(crate) fn embeddings_enabled() -> bool {
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


pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn receipt_for(
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

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn round_score(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}


