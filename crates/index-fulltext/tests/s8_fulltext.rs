use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{
    inspect_snapshot_root, publish_snapshot, FullTextIndex, IndexDocument, IndexSection,
    SearchQuery, SnapshotReadTarget, SnapshotRootState,
};
use tantivy::collector::TopDocs;
use tantivy::query::AllQuery;
use tantivy::schema::{TantivyDocument, Value};
use tantivy::Index;

#[test]
fn exposes_index_fulltext_crate_identity() {
    assert_eq!(index_fulltext::crate_name(), "index-fulltext");
}

#[test]
fn committed_documents_are_searchable_after_reader_reload() {
    let index_dir = temp_dir("commit-searchable");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    index
        .replace_documents([java_payment_document(false)])
        .unwrap();
    index.commit().unwrap();
    index.reload().unwrap();

    let hits = index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rank, 1);
    assert_eq!(hits[0].doc_id, "doc_java_payment");
    assert_eq!(hits[0].version_id, "ver_java_payment");
    assert_eq!(hits[0].file_name, "synthetic-java-payment.pdf");
    assert!(hits[0].snippet.contains("Java"));
    assert!(!format!("{:?}", hits[0]).contains("Java payment platform"));
    assert!(!format!("{:?}", java_payment_document(false)).contains("Java payment platform"));
    assert!(!format!("{:?}", SearchQuery::new("Java payment")).contains("Java payment"));

    remove_dir(&index_dir);
}

#[test]
fn deleted_documents_are_invisible_by_default() {
    let index_dir = temp_dir("deleted-hidden");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    index
        .replace_documents([
            java_payment_document(true),
            IndexDocument {
                doc_id: "doc_visible".to_string(),
                version_id: "ver_visible".to_string(),
                file_name: "visible-rust.pdf".to_string(),
                clean_text: "Rust local search implementation".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Rust local search".to_string(),
                }],
                is_deleted: false,
            },
        ])
        .unwrap();
    index.commit().unwrap();
    index.reload().unwrap();

    let hits = index
        .search(SearchQuery::new("Java payment").with_limit(10))
        .unwrap();

    assert!(hits.is_empty());
    remove_dir(&index_dir);
}

#[test]
fn top_n_snippets_are_generated_only_for_returned_hits() {
    let index_dir = temp_dir("topn-snippets");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    index
        .replace_documents([
            java_payment_document(false),
            IndexDocument {
                doc_id: "doc_java_backend".to_string(),
                version_id: "ver_java_backend".to_string(),
                file_name: "synthetic-java-backend.pdf".to_string(),
                clean_text: "Java backend search service".to_string(),
                sections: vec![IndexSection {
                    section_type: "skill".to_string(),
                    text: "Java backend".to_string(),
                }],
                is_deleted: false,
            },
        ])
        .unwrap();
    index.commit().unwrap();
    index.reload().unwrap();

    let hits = index
        .search(SearchQuery::new("Java").with_limit(1))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rank, 1);
    assert!(!hits[0].snippet.is_empty());
    remove_dir(&index_dir);
}

#[test]
fn duplicate_sections_do_not_hide_distinct_documents_at_top_n_boundary() {
    let index_dir = temp_dir("duplicate-sections");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    let mut section_heavy = java_payment_document(false);
    section_heavy.sections = (0..12)
        .map(|index| IndexSection {
            section_type: "experience".to_string(),
            text: format!("Java payment repeated section {index}"),
        })
        .collect();

    index
        .replace_documents([
            section_heavy,
            IndexDocument {
                doc_id: "doc_second_java".to_string(),
                version_id: "ver_second_java".to_string(),
                file_name: "synthetic-second-java.pdf".to_string(),
                clean_text: "Java payment migration".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Java payment migration".to_string(),
                }],
                is_deleted: false,
            },
        ])
        .unwrap();
    index.commit().unwrap();
    index.reload().unwrap();

    let hits = index
        .search(SearchQuery::new("Java").with_limit(2))
        .unwrap();

    assert_eq!(hits.len(), 2);
    assert!(hits.iter().any(|hit| hit.doc_id == "doc_java_payment"));
    assert!(hits.iter().any(|hit| hit.doc_id == "doc_second_java"));
    remove_dir(&index_dir);
}

#[test]
fn malformed_query_syntax_returns_safe_result_instead_of_error() {
    let index_dir = temp_dir("malformed-query");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    index
        .replace_documents([java_payment_document(false)])
        .unwrap();
    index.commit().unwrap();
    index.reload().unwrap();

    let hits = index
        .search(SearchQuery::new("Java \"").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    remove_dir(&index_dir);
}

#[test]
fn mixed_chinese_english_query_matches_clean_text() {
    let index_dir = temp_dir("mixed-query");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    index
        .replace_documents([IndexDocument {
            doc_id: "doc_java_pay_cn".to_string(),
            version_id: "ver_java_pay_cn".to_string(),
            file_name: "synthetic-java-pay-cn.pdf".to_string(),
            clean_text: "Java 支付平台 本地搜索".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Java 支付平台".to_string(),
            }],
            is_deleted: false,
        }])
        .unwrap();
    index.commit().unwrap();
    index.reload().unwrap();

    let hits = index
        .search(SearchQuery::new("Java 支付").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc_java_pay_cn");
    assert!(hits[0].snippet.contains("支付"));
    remove_dir(&index_dir);
}

#[test]
fn snippets_redact_contact_values_near_query_matches() {
    let index_dir = temp_dir("snippet-redaction");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    index
        .replace_documents([IndexDocument {
            doc_id: "doc_contact".to_string(),
            version_id: "ver_contact".to_string(),
            file_name: "synthetic-contact.pdf".to_string(),
            clean_text:
                "Built Java. Phone: +14155550132 Alt: 4155550132 Email: Shared.Candidate@Example.Test"
                    .to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Built Java ranking services".to_string(),
            }],
            is_deleted: false,
        }])
        .unwrap();
    index.commit().unwrap();
    index.reload().unwrap();

    let hits = index
        .search(SearchQuery::new("Java").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("Java"));
    assert!(hits[0].snippet.contains("<redacted-email>"));
    assert!(hits[0].snippet.contains("<redacted-phone>"));
    assert!(!hits[0].snippet.contains("Shared.Candidate"));
    assert!(!hits[0].snippet.contains("415"));
    remove_dir(&index_dir);
}

#[test]
fn stored_index_fields_redact_contact_values_before_commit() {
    let index_dir = temp_dir("stored-contact-redaction");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();

    index
        .replace_documents([IndexDocument {
            doc_id: "doc_stored_contact".to_string(),
            version_id: "ver_stored_contact".to_string(),
            file_name: "synthetic-Shared.Candidate@Example.Test.pdf".to_string(),
            clean_text: concat!(
                "Built Java systems. Email: Shared.Candidate@Example.Test ",
                "Phone: (415) 555-0132 Alt: (415)555-0132 Backup: +1(415)555-0132"
            )
            .to_string(),
            sections: vec![IndexSection {
                section_type: "contact".to_string(),
                text: "Contact +14155550132 and Shared.Candidate@Example.Test".to_string(),
            }],
            is_deleted: false,
        }])
        .unwrap();
    index.commit().unwrap();
    drop(index);

    let stored_text = stored_text_dump(&index_dir);
    assert!(stored_text.contains("Java"));
    assert!(stored_text.contains("<redacted-email>"));
    assert!(stored_text.contains("<redacted-phone>"));
    assert!(!stored_text.contains("Shared.Candidate"));
    assert!(!stored_text.contains("shared.candidate"));
    assert!(!stored_text.contains("415"));
    assert!(!stored_text.contains("+14155550132"));

    let reopened = FullTextIndex::open(&index_dir).unwrap();
    let hits = reopened
        .search(SearchQuery::new("Java systems").with_limit(5))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("Java"));

    for contact_query in [
        "Shared.Candidate@Example.Test",
        "(415) 555-0132",
        "(415)555-0132",
        "+1(415)555-0132",
        "+14155550132",
    ] {
        let contact_hits = reopened
            .search(SearchQuery::new(contact_query).with_limit(5))
            .unwrap();
        assert!(
            contact_hits.is_empty(),
            "query should not match: {contact_query}"
        );
    }
    remove_dir(&index_dir);
}

#[test]
fn published_snapshot_becomes_active_without_reading_staging_orphans() {
    let index_root = temp_dir("published-snapshot");

    publish_snapshot(
        &index_root,
        "fulltext-1800001000-1-0-0",
        [java_payment_document(false)],
    )
    .unwrap();
    fs::create_dir_all(index_root.join("staging").join("orphan-bad")).unwrap();
    fs::write(
        index_root
            .join("staging")
            .join("orphan-bad")
            .join("meta.json"),
        b"not a valid tantivy index",
    )
    .unwrap();

    let inspection = inspect_snapshot_root(&index_root).unwrap();
    assert_eq!(inspection.state(), SnapshotRootState::Ready);
    assert_eq!(
        inspection.read_target(),
        Some(SnapshotReadTarget::PublishedSnapshot)
    );
    assert_eq!(
        inspection.active_snapshot(),
        Some("fulltext-1800001000-1-0-0")
    );
    assert_eq!(inspection.staging_orphans(), 1);

    let index = FullTextIndex::open_active(&index_root).unwrap().unwrap();
    let hits = index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc_java_payment");

    remove_dir(&index_root);
}

fn java_payment_document(is_deleted: bool) -> IndexDocument {
    IndexDocument {
        doc_id: "doc_java_payment".to_string(),
        version_id: "ver_java_payment".to_string(),
        file_name: "synthetic-java-payment.pdf".to_string(),
        clean_text: "Built a Java payment platform with local search observability.".to_string(),
        sections: vec![
            IndexSection {
                section_type: "experience".to_string(),
                text: "Java payment platform".to_string(),
            },
            IndexSection {
                section_type: "skill".to_string(),
                text: "Java Rust SQLite".to_string(),
            },
        ],
        is_deleted,
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s8-index-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn stored_text_dump(index_dir: &Path) -> String {
    let index = Index::open_in_dir(index_dir).unwrap();
    let schema = index.schema();
    let reader = index.reader().unwrap();
    let searcher = reader.searcher();
    let fields = [
        schema.get_field("file_name").unwrap(),
        schema.get_field("clean_text").unwrap(),
        schema.get_field("all_sections").unwrap(),
        schema.get_field("section_text").unwrap(),
    ];

    let mut values = Vec::new();
    for (_, address) in searcher
        .search(&AllQuery, &TopDocs::with_limit(10).order_by_score())
        .unwrap()
    {
        let document = searcher.doc::<TantivyDocument>(address).unwrap();
        for field in fields {
            values.extend(
                document
                    .get_all(field)
                    .filter_map(|value| value.as_value().as_str().map(str::to_string)),
            );
        }
    }

    values.join("\n")
}
