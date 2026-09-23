pub mod extractors;
pub mod runtime;
pub mod types;

pub use runtime::{shared_runtime, KnowledgeCorpusRuntime};
pub use types::{
    IngestDisposition, IngestReceipt, KnowledgeCorpusError, KnowledgeHit, RebuildReceipt,
};

#[cfg(test)]
pub(crate) use extractors::{parse_docx_xml, DocxDocumentAdapter, DocumentExtractor, PdfDocumentAdapter};
#[cfg(test)]
pub(crate) use types::ExtractedContent;

#[cfg(test)]
mod tests;
