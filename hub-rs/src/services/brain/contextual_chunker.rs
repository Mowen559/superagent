//! Structure-aware, character-safe chunking for Knowledge Documents.
//!
//! The chunker keeps headings, paragraph ranges, line ranges and PDF page
//! ranges so retrieval can return a verifiable Citation instead of an opaque
//! slice of text.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextualChunk {
    pub chunk_index: usize,
    pub global_context: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_paragraph: usize,
    pub end_paragraph: usize,
    pub start_page: Option<usize>,
    pub end_page: Option<usize>,
    pub heading_path: Vec<String>,
}

impl ContextualChunk {
    /// Render the bounded context prefix consumed by the embedding and LLM
    /// layers. Retrieval callers still receive the raw Citation separately.
    pub fn to_llm_string(&self) -> String {
        if self.global_context.trim().is_empty() {
            return self.content.clone();
        }
        format!(
            "(Document context: {})\n{}",
            self.global_context, self.content
        )
    }
}

#[derive(Debug, Clone)]
struct TextBlock {
    content: String,
    start_line: usize,
    end_line: usize,
    paragraph: usize,
    heading_path: Vec<String>,
}

pub struct ContextualChunker {
    pub max_chunk_size: usize,
}

impl ContextualChunker {
    pub fn new(max_chunk_size: usize) -> Self {
        Self {
            max_chunk_size: max_chunk_size.max(64),
        }
    }

    /// Chunk UTF-8 text at heading/paragraph seams. The limit is measured in
    /// Unicode scalar values, never bytes, so CJK text cannot be split at an
    /// invalid UTF-8 offset.
    pub fn chunk_text(&self, text: &str, global_context: &str) -> Vec<ContextualChunk> {
        let blocks = parse_blocks(text);
        self.chunk_blocks(blocks, global_context)
    }

    /// Chunk PDF pages independently so every result has an exact page
    /// Citation. Page-local line and paragraph ranges remain available for
    /// diagnostics, while the page range is the authoritative PDF location.
    pub fn chunk_pages(&self, pages: &[String], global_context: &str) -> Vec<ContextualChunk> {
        let mut chunks = Vec::new();
        for (page_index, page) in pages.iter().enumerate() {
            let mut page_chunks = self.chunk_text(page, global_context);
            for chunk in &mut page_chunks {
                chunk.chunk_index = chunks.len();
                chunk.start_page = Some(page_index + 1);
                chunk.end_page = Some(page_index + 1);
            }
            chunks.extend(page_chunks);
        }
        chunks
    }

    fn chunk_blocks(&self, blocks: Vec<TextBlock>, global_context: &str) -> Vec<ContextualChunk> {
        let mut chunks = Vec::new();
        let mut current: Option<ContextualChunk> = None;

        for block in blocks {
            for fragment in split_by_chars(&block.content, self.max_chunk_size) {
                let separator_len = current.as_ref().map_or(0, |_| 2);
                let would_exceed = current.as_ref().is_some_and(|chunk| {
                    chunk.content.chars().count() + separator_len + fragment.chars().count()
                        > self.max_chunk_size
                });
                let crosses_heading = current
                    .as_ref()
                    .is_some_and(|chunk| chunk.heading_path != block.heading_path);
                if would_exceed || crosses_heading {
                    if let Some(chunk) = current.take() {
                        chunks.push(chunk);
                    }
                }

                if let Some(chunk) = current.as_mut() {
                    chunk.content.push_str("\n\n");
                    chunk.content.push_str(&fragment);
                    chunk.end_line = block.end_line;
                    chunk.end_paragraph = block.paragraph;
                } else {
                    current = Some(ContextualChunk {
                        chunk_index: chunks.len(),
                        global_context: global_context.to_string(),
                        content: fragment,
                        start_line: block.start_line,
                        end_line: block.end_line,
                        start_paragraph: block.paragraph,
                        end_paragraph: block.paragraph,
                        start_page: None,
                        end_page: None,
                        heading_path: block.heading_path.clone(),
                    });
                }
            }
        }

        if let Some(chunk) = current {
            chunks.push(chunk);
        }
        for (index, chunk) in chunks.iter_mut().enumerate() {
            chunk.chunk_index = index;
        }
        chunks
    }
}

fn parse_blocks(text: &str) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    let mut paragraph_lines = Vec::<String>::new();
    let mut paragraph_start = 1;
    let mut paragraph_index = 0;
    let mut heading_path = Vec::<String>::new();

    for (zero_based, line) in text.lines().enumerate() {
        let line_number = zero_based + 1;
        if line.trim().is_empty() {
            flush_paragraph(
                &mut blocks,
                &mut paragraph_lines,
                paragraph_start,
                line_number.saturating_sub(1),
                &heading_path,
                &mut paragraph_index,
            );
            paragraph_start = line_number + 1;
            continue;
        }

        if let Some((level, title)) = markdown_heading(line) {
            flush_paragraph(
                &mut blocks,
                &mut paragraph_lines,
                paragraph_start,
                line_number.saturating_sub(1),
                &heading_path,
                &mut paragraph_index,
            );
            heading_path.truncate(level.saturating_sub(1));
            heading_path.push(title);
            paragraph_index += 1;
            blocks.push(TextBlock {
                content: line.trim().to_string(),
                start_line: line_number,
                end_line: line_number,
                paragraph: paragraph_index,
                heading_path: heading_path.clone(),
            });
            paragraph_start = line_number + 1;
            continue;
        }

        if paragraph_lines.is_empty() {
            paragraph_start = line_number;
        }
        paragraph_lines.push(line.to_string());
    }

    flush_paragraph(
        &mut blocks,
        &mut paragraph_lines,
        paragraph_start,
        text.lines().count().max(paragraph_start),
        &heading_path,
        &mut paragraph_index,
    );
    blocks
}

fn flush_paragraph(
    blocks: &mut Vec<TextBlock>,
    lines: &mut Vec<String>,
    start_line: usize,
    end_line: usize,
    heading_path: &[String],
    paragraph_index: &mut usize,
) {
    if lines.is_empty() {
        return;
    }
    *paragraph_index += 1;
    blocks.push(TextBlock {
        content: lines.join("\n"),
        start_line,
        end_line: end_line.max(start_line),
        paragraph: *paragraph_index,
        heading_path: heading_path.to_vec(),
    });
    lines.clear();
}

fn markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let title = trimmed[level..].trim();
    (!title.is_empty()).then(|| (level, title.to_string()))
}

fn split_by_chars(value: &str, max_chars: usize) -> Vec<String> {
    if value.chars().count() <= max_chars {
        return vec![value.to_string()];
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;
    for ch in value.chars() {
        current.push(ch);
        current_len += 1;
        if current_len == max_chars {
            parts.push(std::mem::take(&mut current));
            current_len = 0;
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cjk_by_char_without_exceeding_limit() {
        let chunks = ContextualChunker::new(64).chunk_text(&"知".repeat(150), "上下文");
        assert_eq!(chunks.len(), 3);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.content.chars().count() <= 64));
    }

    #[test]
    fn preserves_heading_and_line_ranges() {
        let text = "# Runtime\n\nfirst paragraph\ncontinued\n\n## Store\n\nsecond paragraph";
        let chunks = ContextualChunker::new(80).chunk_text(text, "document");
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line, 8);
        assert_eq!(
            chunks.last().unwrap().heading_path,
            vec!["Runtime", "Store"]
        );
    }

    #[test]
    fn pdf_pages_never_merge_citations() {
        let pages = vec!["page one".to_string(), "page two".to_string()];
        let chunks = ContextualChunker::new(64).chunk_pages(&pages, "pdf");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_page, Some(1));
        assert_eq!(chunks[1].start_page, Some(2));
    }
}
