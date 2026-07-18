use std::collections::{BTreeMap, BTreeSet};

use core_domain::{normalize_query_set_query, QuerySetSampleShape};

use super::{document_terms, query, CYCLE_QUERY_COUNT};

#[test]
fn frozen_cycle_has_unique_normalized_queries_and_exact_buckets() {
    let queries = (0..CYCLE_QUERY_COUNT).map(query).collect::<Vec<_>>();
    let normalized = queries
        .iter()
        .map(|query| normalize_query_set_query(query).unwrap())
        .collect::<BTreeSet<_>>();
    let mut counts = BTreeMap::new();
    for query in &queries {
        *counts
            .entry(QuerySetSampleShape::from_query(query).bucket())
            .or_insert(0) += 1;
    }

    assert_eq!(normalized.len(), 500);
    assert_eq!(counts["single_term"], 50);
    assert_eq!(counts["and_2"], 75);
    assert_eq!(counts["and_3_5"], 150);
    assert_eq!(counts["and_6_16"], 50);
    assert_eq!(counts["field_filter"], 75);
    assert_eq!(counts["hybrid"], 75);
    assert_eq!(counts["semantic"], 25);
}

#[test]
fn every_document_cycle_contains_its_query_terms_without_boolean_syntax() {
    for index in 0..CYCLE_QUERY_COUNT {
        let terms = document_terms(index);
        assert!(!terms.contains('"'));
        assert!(!terms.split_whitespace().any(|term| term == "OR"));
        assert!(!terms.is_empty());
    }
}
