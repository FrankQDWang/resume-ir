use search_planner::{plan_search, SearchPlan};

#[test]
fn exposes_search_planner_crate_identity() {
    assert_eq!(search_planner::crate_name(), "search-planner");
}

#[test]
fn plans_mixed_query_without_echoing_raw_query_in_debug() {
    let plan = plan_search(" Java  支付  ", 25).unwrap();

    assert_eq!(plan.query_text(), "Java 支付");
    assert_eq!(plan.limit(), 25);
    assert_eq!(plan.terms(), &["Java", "支付"]);
    assert!(!format!("{plan:?}").contains("Java 支付"));
}

#[test]
fn canonicalizes_execution_query_without_silently_dropping_terms() {
    let plan = plan_search("  ｒｕｓｔ  rust  and  ", 25).unwrap();

    assert_eq!(plan.query_text(), "rust and");
    assert_eq!(plan.terms(), &["rust", "and"]);
}

#[test]
fn preserves_explicit_or_and_phrase() {
    let plan = plan_search("rust OR “distributed   systems”", 25).unwrap();

    assert_eq!(plan.query_text(), "rust OR \"distributed systems\"");
}

#[test]
fn rejects_query_and_term_bounds_before_index_access() {
    let too_many_terms = (0..17)
        .map(|index| format!("term{index}"))
        .collect::<Vec<_>>()
        .join(" ");

    assert!(plan_search(&too_many_terms, 10).is_err());
    assert!(plan_search(&"a".repeat(257), 10).is_err());
    assert!(plan_search(&"a".repeat(4097), 10).is_err());
}

#[test]
fn rejects_empty_or_too_broad_queries_before_index_access() {
    assert!(plan_search("   ", 10).is_err());
}

#[test]
fn clamps_limit_to_safe_top_n_bound() {
    let plan = plan_search("Rust", 10_000).unwrap();

    assert_eq!(plan.limit(), SearchPlan::MAX_LIMIT);
}
