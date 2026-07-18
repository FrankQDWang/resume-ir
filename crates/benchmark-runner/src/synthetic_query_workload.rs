use std::collections::BTreeMap;

use core_domain::QuerySetSampleShape;

pub(crate) const VERSION: &str = "resume-ir.public-synthetic-query-hot-path.v1";
pub(crate) const CANONICAL_DOCUMENT_COUNT: usize = 10_000;
pub(crate) const CYCLE_QUERY_COUNT: usize = 500;

const BUCKETS: [&str; 7] = [
    "single_term",
    "and_2",
    "and_3_5",
    "and_6_16",
    "field_filter",
    "hybrid",
    "semantic",
];

pub(crate) fn query(index: usize) -> String {
    let index = index % CYCLE_QUERY_COUNT;
    match index {
        0..50 => format!("single{}", alphabetic_id(index)),
        50..125 => {
            let id = alphabetic_id(index - 50);
            format!("twoa{id} twob{id}")
        }
        125..275 => {
            let local = index - 125;
            joined_terms("three", &alphabetic_id(local), 3 + local % 3)
        }
        275..325 => {
            let local = index - 275;
            joined_terms("many", &alphabetic_id(local), 6 + local % 11)
        }
        325..400 => format!("field{} shanghai", alphabetic_id(index - 325)),
        400..475 => {
            let id = alphabetic_id(index - 400);
            format!("hybrida{id} OR hybridb{id}")
        }
        475..500 => format!(
            "\"semantic{} distributed systems\"",
            alphabetic_id(index - 475)
        ),
        _ => unreachable!("query index is reduced to the fixed cycle"),
    }
}

pub(crate) fn document_terms(index: usize) -> String {
    query(index)
        .replace('"', "")
        .split_whitespace()
        .filter(|term| !matches!(*term, "AND" | "OR" | "NOT"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn redacted_contract_json() -> String {
    let counts = bucket_counts();
    let buckets = BUCKETS
        .into_iter()
        .map(|bucket| format!("\"{bucket}\":{}", counts[bucket]))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        concat!(
            "{{",
            "\"version\":\"{}\",",
            "\"canonical_document_count\":{},",
            "\"cycle_query_count\":{},",
            "\"cycle_unique_query_count\":{},",
            "\"bucket_counts\":{{{}}}",
            "}}"
        ),
        VERSION,
        CANONICAL_DOCUMENT_COUNT,
        CYCLE_QUERY_COUNT,
        unique_query_count(),
        buckets,
    )
}

fn unique_query_count() -> usize {
    (0..CYCLE_QUERY_COUNT)
        .map(query)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

fn bucket_counts() -> BTreeMap<&'static str, usize> {
    let mut counts = BUCKETS
        .into_iter()
        .map(|bucket| (bucket, 0))
        .collect::<BTreeMap<_, _>>();
    for query in (0..CYCLE_QUERY_COUNT).map(query) {
        *counts
            .get_mut(QuerySetSampleShape::from_query(&query).bucket())
            .expect("query shape must use a declared public workload bucket") += 1;
    }
    counts
}

fn joined_terms(prefix: &str, id: &str, count: usize) -> String {
    (0..count)
        .map(|position| format!("{prefix}{}{id}", alphabetic_id(position)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn alphabetic_id(mut value: usize) -> String {
    let mut reversed = Vec::new();
    loop {
        reversed.push((b'a' + (value % 26) as u8) as char);
        value /= 26;
        if value == 0 {
            break;
        }
    }
    reversed.into_iter().rev().collect()
}

#[cfg(test)]
#[path = "synthetic_query_workload_tests.rs"]
mod tests;
