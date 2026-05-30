use index_fulltext::{FullTextIndex, IndexDocument};

#[test]
fn committed_documents_are_searchable_after_reader_reload() {
    let index = FullTextIndex::create_in_memory().expect("index");
    index
        .index_batch(vec![IndexDocument::searchable(
            "doc_java",
            "ver_java",
            "java_resume.pdf",
            "Java payment gateway engineer",
            "experience",
        )])
        .expect("index batch");
    index.commit().expect("commit");

    let hits = index.search("Java payment", 10).expect("search");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rank, 1);
    assert_eq!(hits[0].doc_id, "doc_java");
    assert_eq!(hits[0].file_name, "java_resume.pdf");
    assert!(hits[0].snippet.contains("Java"));
}

#[test]
fn deleted_documents_are_hidden_by_default() {
    let index = FullTextIndex::create_in_memory().expect("index");
    index
        .index_batch(vec![
            IndexDocument::searchable("doc_live", "ver_live", "live.pdf", "Java backend", "skill"),
            IndexDocument::deleted(
                "doc_deleted",
                "ver_deleted",
                "deleted.pdf",
                "Java backend",
                "skill",
            ),
        ])
        .expect("index batch");
    index.commit().expect("commit");

    let hits = index.search("Java", 10).expect("search");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc_live");
}

#[test]
fn snippets_are_generated_only_for_returned_top_n() {
    let index = FullTextIndex::create_in_memory().expect("index");
    index
        .index_batch(vec![
            IndexDocument::searchable(
                "doc_one",
                "ver_one",
                "one.pdf",
                "Java Spring Cloud",
                "skill",
            ),
            IndexDocument::searchable("doc_two", "ver_two", "two.pdf", "Java MySQL", "skill"),
        ])
        .expect("index batch");
    index.commit().expect("commit");

    let hits = index.search("Java", 1).expect("search");

    assert_eq!(hits.len(), 1);
    assert!(!hits[0].snippet.is_empty());
}
