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


use crate::shared_kernel::text::truncate_chars;
use super::types::*;

pub(crate) trait DocumentExtractor: Send + Sync {
    fn supports(&self, extension: &str) -> bool;
    fn extract(
        &self,
        bytes: &[u8],
        extension: &str,
    ) -> Result<ExtractedDocument, KnowledgeCorpusError>;
}

pub(crate) struct TextDocumentAdapter;
pub(crate) struct DocxDocumentAdapter;
pub(crate) struct PdfDocumentAdapter;

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

pub(crate) fn decode_text(bytes: &[u8]) -> Result<String, KnowledgeCorpusError> {
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

pub(crate) fn parse_docx_xml(xml: &str) -> Result<String, KnowledgeCorpusError> {
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

pub(crate) fn derive_global_context(display_name: &str, content: &ExtractedContent) -> String {
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

pub(crate) fn normalize_context(value: &str) -> String {
    truncate_chars(
        &value.split_whitespace().collect::<Vec<_>>().join(" "),
        MAX_CONTEXT_CHARS,
    )
}

pub(crate) fn citation_for(source: &str, format: DocumentFormat, chunk: &ContextualChunk) -> String {
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


