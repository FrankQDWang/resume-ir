//! Full-text Tantivy contract tests.

use index_fulltext::{FullTextError, FullTextIndexReader, FullTextIndexWriter, IndexDocument};
use search_planner::SearchOptions;
use tantivy::schema::TantivyDocument;
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{Index, TantivyError};

#[test]
fn commit_makes_new_documents_searchable_after_reader_reload(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let mut writer = FullTextIndexWriter::open_or_create(temp_dir.path())?;
    let reader = FullTextIndexReader::open_existing(temp_dir.path())?;

    assert!(reader
        .search("Java 支付", SearchOptions::default())?
        .is_empty());

    writer.add_document(IndexDocument {
        doc_id: "doc-visible".to_string(),
        version_id: "ver-visible".to_string(),
        file_name: "synthetic-a.pdf".to_string(),
        clean_text: "Java 支付 platform synthetic project text".to_string(),
        section_type: "experience".to_string(),
        is_deleted: false,
    })?;
    writer.commit()?;

    let hits = reader.search("Java 支付", SearchOptions::default())?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rank, 1);
    assert_eq!(hits[0].doc_id, "doc-visible");
    assert_eq!(hits[0].file_name, "synthetic-a.pdf");
    assert!(hits[0].snippet.contains("Java") || hits[0].snippet.contains("支付"));
    Ok(())
}

#[test]
fn deleted_marker_is_hidden_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let mut writer = FullTextIndexWriter::open_or_create(temp_dir.path())?;

    writer.add_document(IndexDocument {
        doc_id: "doc-visible".to_string(),
        version_id: "ver-visible".to_string(),
        file_name: "synthetic-visible.pdf".to_string(),
        clean_text: "Java 支付 visible synthetic text".to_string(),
        section_type: "experience".to_string(),
        is_deleted: false,
    })?;
    writer.add_document(IndexDocument {
        doc_id: "doc-deleted".to_string(),
        version_id: "ver-deleted".to_string(),
        file_name: "synthetic-deleted.pdf".to_string(),
        clean_text: "Java 支付 deleted synthetic text".to_string(),
        section_type: "experience".to_string(),
        is_deleted: true,
    })?;
    writer.commit()?;

    let reader = FullTextIndexReader::open_existing(temp_dir.path())?;
    let hits = reader.search("Java 支付", SearchOptions::default())?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc-visible");

    let all_hits = reader.search(
        "Java 支付",
        SearchOptions {
            include_deleted: true,
            ..SearchOptions::default()
        },
    )?;

    assert_eq!(all_hits.len(), 2);
    assert!(all_hits.iter().any(|hit| hit.doc_id == "doc-deleted"));
    Ok(())
}

#[test]
fn adding_new_version_for_same_document_replaces_old_search_hit(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let mut writer = FullTextIndexWriter::open_or_create(temp_dir.path())?;

    writer.add_document(IndexDocument {
        doc_id: "doc-stable".to_string(),
        version_id: "ver-old".to_string(),
        file_name: "synthetic-old.pdf".to_string(),
        clean_text: "Java old synthetic text".to_string(),
        section_type: "experience".to_string(),
        is_deleted: false,
    })?;
    writer.commit()?;

    writer.add_document(IndexDocument {
        doc_id: "doc-stable".to_string(),
        version_id: "ver-new".to_string(),
        file_name: "synthetic-new.pdf".to_string(),
        clean_text: "Java new synthetic text".to_string(),
        section_type: "experience".to_string(),
        is_deleted: false,
    })?;
    writer.commit()?;

    let reader = FullTextIndexReader::open_existing(temp_dir.path())?;
    let hits = reader.search("Java", SearchOptions::default())?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc-stable");
    assert_eq!(hits[0].file_name, "synthetic-new.pdf");
    assert!(hits[0].snippet.contains("new synthetic"));
    Ok(())
}

#[test]
fn debug_output_redacts_text_file_name_and_snippet() {
    let document = IndexDocument {
        doc_id: "doc-debug".to_string(),
        version_id: "ver-debug".to_string(),
        file_name: "synthetic-debug.pdf".to_string(),
        clean_text: "sensitive synthetic full text should not appear".to_string(),
        section_type: "skill".to_string(),
        is_deleted: false,
    };

    let debug_text = format!("{document:?}");

    assert!(debug_text.contains("doc-debug"));
    assert!(!debug_text.contains("synthetic-debug.pdf"));
    assert!(!debug_text.contains("sensitive synthetic full text"));
}

#[test]
fn error_debug_redacts_filesystem_details() {
    let redacted_path = "/synthetic/redacted/index-path";
    let err = FullTextIndexReader::open_existing(redacted_path)
        .err()
        .map(|error| format!("{error:?}"));

    assert_eq!(err.as_deref(), Some("FullTextError::MissingIndex"));
    assert!(err
        .as_deref()
        .is_some_and(|debug| !debug.contains(redacted_path)));
}

#[test]
fn malformed_stored_document_errors_without_public_empty_hit(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let mut writer = FullTextIndexWriter::open_or_create(temp_dir.path())?;
    writer.commit()?;
    drop(writer);
    add_malformed_matching_document(temp_dir.path())?;

    let reader = FullTextIndexReader::open_existing(temp_dir.path())?;
    let err = reader
        .search("Java 支付", SearchOptions::default())
        .err()
        .ok_or("malformed document should not search")?;

    assert!(matches!(err, FullTextError::MalformedDocument));
    let debug = format!("{err:?}");
    assert_eq!(debug, "FullTextError::MalformedDocument");
    assert!(!debug.contains("Java"));
    assert!(!debug.contains("synthetic"));
    Ok(())
}

fn add_malformed_matching_document(path: &std::path::Path) -> Result<(), TantivyError> {
    let index = Index::open_in_dir(path)?;
    let tokenizer = TextAnalyzer::builder(NgramTokenizer::all_ngrams(1, 24)?)
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("resume_cjk_ngram", tokenizer);
    let schema = index.schema();
    let clean_text = schema.get_field("clean_text")?;
    let is_deleted = schema.get_field("is_deleted")?;
    let mut writer = index.writer(50_000_000)?;
    let mut document = TantivyDocument::default();
    document.add_text(clean_text, "Java 支付 synthetic malformed text");
    document.add_bool(is_deleted, false);
    writer.add_document(document)?;
    writer.commit()?;
    Ok(())
}
