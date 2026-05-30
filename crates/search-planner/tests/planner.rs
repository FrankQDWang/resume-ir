use search_planner::{SearchRequest, plan_search};

#[test]
fn planner_clamps_top_k_and_keeps_snippets_on() {
    let plan = plan_search(SearchRequest {
        query: "Java 支付".to_owned(),
        top_k: 500,
    });

    assert_eq!(plan.fulltext_query, "Java 支付");
    assert_eq!(plan.top_k, 100);
    assert!(plan.include_snippet);
}

#[test]
fn planner_uses_default_top_k_for_zero() {
    let plan = plan_search(SearchRequest {
        query: "Rust".to_owned(),
        top_k: 0,
    });

    assert_eq!(plan.top_k, 20);
}
