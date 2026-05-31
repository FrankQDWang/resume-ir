use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection, SearchQuery};

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
