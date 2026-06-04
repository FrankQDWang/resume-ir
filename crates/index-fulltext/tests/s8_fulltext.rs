use std::collections::BTreeSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use index_fulltext::{
    inspect_snapshot_root, publish_incremental_snapshot, publish_snapshot, redact_contact_values,
    FullTextIndex, IndexDocument, IndexSection, SearchQuery, SnapshotReadTarget, SnapshotRootState,
};
use tantivy::collector::TopDocs;
use tantivy::query::AllQuery;
use tantivy::schema::{TantivyDocument, Value};
use tantivy::Index;

const SNAPSHOT_TEST_WRITE_RETRY_ATTEMPTS: usize = 100;
const SNAPSHOT_TEST_WRITE_RETRY_DELAY: Duration = Duration::from_millis(50);

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
fn redaction_removes_common_local_path_shapes() {
    let text = "paths /Users/frank/private/resume.pdf file:///private/tmp/resume.pdf C:\\Users\\frank\\resume.pdf and email candidate@example.test";
    let redacted = redact_contact_values(text);

    assert!(!redacted.contains("/Users/frank"));
    assert!(!redacted.contains("file:///private"));
    assert!(!redacted.contains("C:\\Users\\frank"));
    assert!(!redacted.contains("candidate@example.test"));
    assert!(redacted.contains("<redacted-path>"));
    assert!(redacted.contains("<redacted-email>"));
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

#[test]
fn published_snapshot_encrypts_payload_at_rest() {
    let index_root = temp_dir("published-encrypted-snapshot");
    let snapshot_name = "fulltext-1800003000-1-0-0";
    let private_payload = "PRIVATE_FULLTEXT_PAYLOAD_SECRET_1800003000";

    publish_snapshot(
        &index_root,
        snapshot_name,
        [IndexDocument {
            doc_id: "doc_private_fulltext".to_string(),
            version_id: "ver_private_fulltext".to_string(),
            file_name: "synthetic-private-fulltext.pdf".to_string(),
            clean_text: format!("Rust local search {private_payload}"),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: format!("Search evidence {private_payload}"),
            }],
            is_deleted: false,
        }],
    )
    .unwrap();

    let snapshot_dir = index_root.join("snapshots").join(snapshot_name);
    let envelope = fs::read(snapshot_dir.join("fulltext.snapshot.enc")).unwrap();
    assert!(envelope.starts_with(b"resume-ir-fulltext-snapshot-encrypted-v1\n"));
    assert!(!snapshot_dir.join("meta.json").exists());
    let snapshot_bytes = recursive_bytes(&snapshot_dir);
    assert!(!String::from_utf8_lossy(&snapshot_bytes).contains(private_payload));

    let reopened = FullTextIndex::open_active(&index_root).unwrap().unwrap();
    let hits = reopened
        .search(SearchQuery::new("Rust local search").with_limit(5))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc_private_fulltext");
    assert!(hits[0].snippet.contains("Rust"));

    remove_dir(&index_root);
}

#[test]
fn incremental_snapshot_inherits_replaces_and_excludes_documents() {
    let index_root = temp_dir("incremental-snapshot");

    publish_snapshot(
        &index_root,
        "fulltext-1800004000-1-0-0",
        [
            java_payment_document(false),
            IndexDocument {
                doc_id: "doc_backend".to_string(),
                version_id: "ver_backend_old".to_string(),
                file_name: "synthetic-backend-old.pdf".to_string(),
                clean_text: "Rust backend retiredtoken".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Rust backend retiredtoken".to_string(),
                }],
                is_deleted: false,
            },
        ],
    )
    .unwrap();

    publish_incremental_snapshot(
        &index_root,
        "fulltext-1800005000-1-0-0",
        [
            IndexDocument {
                doc_id: "doc_backend".to_string(),
                version_id: "ver_backend_new".to_string(),
                file_name: "synthetic-backend-new.pdf".to_string(),
                clean_text: "Go backend updated snapshot token".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Go backend updated".to_string(),
                }],
                is_deleted: false,
            },
            IndexDocument {
                doc_id: "doc_python".to_string(),
                version_id: "ver_python_new".to_string(),
                file_name: "synthetic-python-new.pdf".to_string(),
                clean_text: "Python ranking new snapshot token".to_string(),
                sections: vec![IndexSection {
                    section_type: "skill".to_string(),
                    text: "Python ranking".to_string(),
                }],
                is_deleted: false,
            },
        ],
        &BTreeSet::from(["doc_java_payment".to_string()]),
    )
    .unwrap();

    let inspection = inspect_snapshot_root(&index_root).unwrap();
    assert_eq!(inspection.state(), SnapshotRootState::Ready);
    assert_eq!(
        inspection.active_snapshot(),
        Some("fulltext-1800005000-1-0-0")
    );

    let index = FullTextIndex::open_active(&index_root).unwrap().unwrap();
    assert!(index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap()
        .is_empty());
    assert!(index
        .search(SearchQuery::new("retiredtoken").with_limit(5))
        .unwrap()
        .is_empty());

    let updated_hits = index
        .search(SearchQuery::new("Go backend").with_limit(5))
        .unwrap();
    assert_eq!(updated_hits.len(), 1);
    assert_eq!(updated_hits[0].doc_id, "doc_backend");
    assert_eq!(updated_hits[0].version_id, "ver_backend_new");

    let new_hits = index
        .search(SearchQuery::new("Python ranking").with_limit(5))
        .unwrap();
    assert_eq!(new_hits.len(), 1);
    assert_eq!(new_hits[0].doc_id, "doc_python");

    remove_dir(&index_root);
}

#[test]
fn active_snapshot_corruption_falls_back_to_last_good_snapshot() {
    let index_root = temp_dir("snapshot-fallback");
    publish_snapshot(
        &index_root,
        "fulltext-1800001000-1-0-0",
        [java_payment_document(false)],
    )
    .unwrap();
    publish_snapshot(
        &index_root,
        "fulltext-1800002000-1-0-0",
        [IndexDocument {
            doc_id: "doc_rust_snapshot".to_string(),
            version_id: "ver_rust_snapshot".to_string(),
            file_name: "synthetic-rust-snapshot.pdf".to_string(),
            clean_text: "Rust snapshot that will be corrupted".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Rust snapshot".to_string(),
            }],
            is_deleted: false,
        }],
    )
    .unwrap();
    write_snapshot_test_file_with_retry(
        &index_root
            .join("snapshots")
            .join("fulltext-1800002000-1-0-0")
            .join("fulltext.snapshot.enc"),
        b"not a valid active snapshot",
    )
    .unwrap();

    let inspection = inspect_snapshot_root(&index_root).unwrap();
    assert_eq!(inspection.state(), SnapshotRootState::Recovered);
    assert_eq!(
        inspection.read_target(),
        Some(SnapshotReadTarget::PublishedSnapshot)
    );
    assert_eq!(
        inspection.active_snapshot(),
        Some("fulltext-1800002000-1-0-0")
    );
    assert_eq!(
        inspection.fallback_snapshot(),
        Some("fulltext-1800001000-1-0-0")
    );

    let index = FullTextIndex::open_active(&index_root).unwrap().unwrap();
    let recovered_hits = index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap();
    assert_eq!(recovered_hits.len(), 1);
    assert_eq!(recovered_hits[0].doc_id, "doc_java_payment");
    assert!(index
        .search(SearchQuery::new("corrupted").with_limit(5))
        .unwrap()
        .is_empty());

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

fn write_snapshot_test_file_with_retry(path: &Path, bytes: &[u8]) -> io::Result<()> {
    for attempt in 0..SNAPSHOT_TEST_WRITE_RETRY_ATTEMPTS {
        match fs::write(path, bytes) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < SNAPSHOT_TEST_WRITE_RETRY_ATTEMPTS
                    && is_transient_snapshot_test_write_error(&error) =>
            {
                thread::sleep(SNAPSHOT_TEST_WRITE_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::other("snapshot test write retry exhausted"))
}

fn is_transient_snapshot_test_write_error(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        ErrorKind::Interrupted | ErrorKind::PermissionDenied | ErrorKind::WouldBlock
    ) {
        return true;
    }

    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33 | 145)) {
        return true;
    }

    let diagnostic = error.to_string().to_ascii_lowercase();
    diagnostic.contains("os error 5")
        || diagnostic.contains("os error 32")
        || diagnostic.contains("os error 33")
        || diagnostic.contains("os error 145")
        || diagnostic.contains("access is denied")
        || diagnostic.contains("permission denied")
        || diagnostic.contains("being used by another process")
        || diagnostic.contains("locked a portion of the file")
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

fn recursive_bytes(root: &Path) -> Vec<u8> {
    let mut output = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            output.extend(recursive_bytes(&path));
        } else {
            output.extend(fs::read(path).unwrap());
        }
    }
    output
}
