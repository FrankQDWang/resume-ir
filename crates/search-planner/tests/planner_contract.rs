//! Search planner contract tests.

use search_planner::{plan_snippets_for_top_results, PlannerCandidate, SearchOptions};

#[test]
fn snippets_are_generated_only_for_top_n_results() {
    let candidates = vec![
        candidate(1, "doc-a"),
        candidate(2, "doc-b"),
        candidate(3, "doc-c"),
    ];
    let mut generated_for = Vec::new();

    let hits = plan_snippets_for_top_results(
        candidates,
        "Java 支付",
        SearchOptions {
            top_k: 2,
            snippet_max_chars: 24,
            include_deleted: false,
        },
        |query, text, max_chars| {
            generated_for.push(text.to_string());
            format!("{query}:{max_chars}")
        },
    );

    assert_eq!(hits.len(), 2);
    assert_eq!(generated_for.len(), 2);
    assert!(generated_for.iter().any(|text| text.contains("doc-a")));
    assert!(generated_for.iter().any(|text| text.contains("doc-b")));
    assert!(!generated_for.iter().any(|text| text.contains("doc-c")));
}

fn candidate(rank: usize, doc_id: &str) -> PlannerCandidate {
    PlannerCandidate {
        rank,
        score: 1.0,
        doc_id: doc_id.to_string(),
        file_name: format!("{doc_id}.pdf"),
        clean_text: format!("Java 支付 synthetic text for {doc_id}"),
    }
}
