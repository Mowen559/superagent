use super::*;

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
