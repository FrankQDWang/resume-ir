use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn private_query_set_file(label: &str, query_count: usize) -> PathBuf {
    private_query_set_file_with_buckets(label, &[("and_3_5", query_count)])
}

pub(crate) fn private_query_set_file_with_buckets(
    label: &str,
    bucket_counts: &[(&str, usize)],
) -> PathBuf {
    private_query_set_file_with_buckets_and_source_kind(
        label,
        bucket_counts,
        "trace_source_search_v1",
    )
}

pub(crate) fn private_query_set_file_with_buckets_and_source_kind(
    label: &str,
    bucket_counts: &[(&str, usize)],
    source_kind: &str,
) -> PathBuf {
    let query_count = bucket_counts.iter().map(|(_, count)| count).sum::<usize>();
    let path = temp_dir(label).join("private-query-set.jsonl");
    let mut lines = String::new();
    let mut index = 0_usize;
    for (bucket, count) in bucket_counts {
        for _ in 0..*count {
            let query = private_query_test_query_for_bucket(bucket, index);
            lines.push_str(
                &serde_json::json!({
                    "schema_version": "resume-ir.query-set.jsonl.v2",
                    "sample_id": format!("private-query-sample-{index:06}"),
                    "bucket": bucket,
                    "query": query,
                    "source_kind": source_kind,
                    "query_shape": private_query_test_shape_for_bucket(bucket),
                })
                .to_string(),
            );
            lines.push('\n');
            index += 1;
        }
    }
    fs::write(&path, lines).unwrap();
    write_private_query_set_summary_with_source_kind(
        &path,
        query_count,
        bucket_counts,
        source_kind,
    );
    path
}

fn write_private_query_set_summary_with_source_kind(
    query_set: &Path,
    query_count: usize,
    bucket_counts: &[(&str, usize)],
    source_kind: &str,
) {
    let (tune_query_count, holdout_query_count) =
        private_query_test_split_counts_for_buckets(bucket_counts);
    write_private_query_set_summary_with_split_and_source_kind(
        query_set,
        query_count,
        tune_query_count,
        holdout_query_count,
        bucket_counts,
        source_kind,
    );
}

pub(crate) fn write_private_query_set_summary_with_split_and_source_kind(
    query_set: &Path,
    query_count: usize,
    tune_query_count: usize,
    holdout_query_count: usize,
    bucket_counts: &[(&str, usize)],
    source_kind: &str,
) {
    fs::write(
        private_query_set_summary_path(query_set),
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"resume-ir.query-set-summary.v2\",",
                "\"privacy_boundary\":\"redacted_local_aggregate\",",
                "\"query_source\":\"{source_kind}\",",
                "\"query_count\":{query_count},",
                "\"tune_query_count\":{tune_query_count},",
                "\"holdout_query_count\":{holdout_query_count},",
                "\"bucket_counts\":{bucket_counts_json},",
                "\"tune_bucket_counts\":{tune_bucket_counts_json},",
                "\"holdout_bucket_counts\":{holdout_bucket_counts_json},",
                "\"candidate_queries_sampled\":{query_count},",
                "\"zero_hit_queries_dropped\":0,",
                "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
                "\"tune_sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\",",
                "\"holdout_sha256\":\"3333333333333333333333333333333333333333333333333333333333333333\",",
                "\"hmac_split\":true,",
                "\"contains_raw_query_text\":false,",
                "\"contains_raw_resume_text\":false,",
                "\"contains_candidate_results\":false,",
                "\"contains_local_paths\":false",
                "}}\n"
            ),
            query_count = query_count,
            tune_query_count = tune_query_count,
            holdout_query_count = holdout_query_count,
            bucket_counts_json = private_query_test_bucket_counts_json(bucket_counts),
            tune_bucket_counts_json =
                private_query_test_split_bucket_counts_json(bucket_counts, holdout_query_count).0,
            holdout_bucket_counts_json =
                private_query_test_split_bucket_counts_json(bucket_counts, holdout_query_count).1,
            source_kind = source_kind
        ),
    )
    .unwrap();
}

pub(crate) fn private_query_set_summary_path(query_set: &Path) -> PathBuf {
    let file_name = query_set.file_name().unwrap().to_str().unwrap();
    let base_name = file_name.strip_suffix(".local.jsonl").unwrap_or(file_name);
    query_set.with_file_name(format!("{base_name}.summary.json"))
}

pub(crate) fn private_query_corpus_summary_json(document_count: usize, hot_index: bool) -> Vec<u8> {
    let searchable_count = if hot_index {
        document_count
    } else {
        document_count.saturating_sub(1)
    };
    let vector_count = if hot_index {
        document_count
    } else {
        document_count.saturating_sub(2)
    };
    format!(
        concat!(
            "{{",
            "\"schema_version\":\"benchmark-corpus-summary.v1\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"document_count\":{},",
            "\"searchable_document_count\":{},",
            "\"vector_indexed_document_count\":{},",
            "\"active_vector_document_count\":{},",
            "\"vector_count\":{},",
            "\"vector_deleted_count\":0,",
            "\"vector_index_state\":\"available\",",
            "\"vector_search_backend\":\"hnsw_ann\",",
            "\"hot_index_fully_covered\":{},",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_queries\":false,",
            "\"contains_sample_ids\":false",
            "}}"
        ),
        document_count, searchable_count, vector_count, vector_count, vector_count, hot_index
    )
    .into_bytes()
}

pub(crate) fn private_query_test_query_for_bucket(bucket: &str, index: usize) -> String {
    let id = private_query_test_alpha_id(index);
    match bucket {
        "single_term" => format!("REDACTION_SENTINEL_PRIVATE_QUERY_SINGLE{id}"),
        "and_2" => format!("REDACTION_SENTINEL_PRIVATE_QUERY_ANDTWO{id} search"),
        "and_3_5" => format!("REDACTION_SENTINEL_PRIVATE_QUERY backend search {id}"),
        "and_6_16" => {
            format!("REDACTION_SENTINEL_PRIVATE_QUERY backend search ranking index systems {id}")
        }
        "field_filter" => format!("REDACTION_SENTINEL_PRIVATE_QUERY shanghai {id}"),
        "hybrid" => format!("REDACTION_SENTINEL_PRIVATE_QUERY AND search {id}"),
        "semantic" => format!("\"REDACTION_SENTINEL_PRIVATE_QUERY semantic {id}\""),
        _ => format!("REDACTION_SENTINEL_PRIVATE_QUERY backend search {id}"),
    }
}

pub(crate) fn private_query_test_shape_for_bucket(bucket: &str) -> serde_json::Value {
    match bucket {
        "single_term" => private_query_test_shape(1, false, false, false, false, true, false),
        "and_2" => private_query_test_shape(2, false, false, false, false, true, false),
        "and_3_5" => private_query_test_shape(4, false, false, false, false, true, false),
        "and_6_16" => private_query_test_shape(7, false, false, false, false, true, false),
        "field_filter" => private_query_test_shape(3, false, true, false, false, true, false),
        "hybrid" => private_query_test_shape(4, true, false, false, false, true, false),
        "semantic" => private_query_test_shape(1, false, false, false, false, true, true),
        _ => private_query_test_shape(4, false, false, false, false, true, false),
    }
}

pub(crate) fn private_query_test_alpha_id(mut value: usize) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push(char::from(b'a' + (value % 26) as u8));
        value /= 26;
        if value == 0 {
            break;
        }
    }
    chars.iter().rev().collect()
}

pub(crate) fn private_query_test_shape(
    term_count: usize,
    has_boolean: bool,
    has_location: bool,
    has_years: bool,
    has_degree: bool,
    has_skill: bool,
    has_phrase: bool,
) -> serde_json::Value {
    serde_json::json!({
        "term_count": term_count,
        "has_boolean": has_boolean,
        "has_location": has_location,
        "has_years": has_years,
        "has_degree": has_degree,
        "has_skill": has_skill,
        "has_phrase": has_phrase,
    })
}

fn private_query_test_split_counts_for_buckets(bucket_counts: &[(&str, usize)]) -> (usize, usize) {
    let query_count = bucket_counts.iter().map(|(_, count)| count).sum::<usize>();
    let (_, holdout_counts_json) =
        private_query_test_split_bucket_counts_json(bucket_counts, query_count / 5);
    let holdout_value: serde_json::Value = serde_json::from_str(&holdout_counts_json).unwrap();
    let holdout_query_count = holdout_value
        .as_object()
        .unwrap()
        .values()
        .map(|value| value.as_u64().unwrap() as usize)
        .sum::<usize>();
    (query_count - holdout_query_count, holdout_query_count)
}

pub(crate) fn private_query_test_buckets() -> &'static [&'static str] {
    &[
        "single_term",
        "and_2",
        "and_3_5",
        "and_6_16",
        "field_filter",
        "hybrid",
        "semantic",
    ]
}

fn private_query_test_bucket_counts_json(bucket_counts: &[(&str, usize)]) -> String {
    let fields = private_query_test_buckets()
        .iter()
        .map(|bucket| {
            let count = bucket_counts
                .iter()
                .filter(|(candidate, _)| candidate == bucket)
                .map(|(_, count)| *count)
                .sum::<usize>();
            format!("\"{bucket}\":{count}")
        })
        .collect::<Vec<_>>();
    format!("{{{}}}", fields.join(","))
}

fn private_query_test_split_bucket_counts_json(
    bucket_counts: &[(&str, usize)],
    holdout_query_count: usize,
) -> (String, String) {
    let min_holdout = bucket_counts.iter().filter(|(_, count)| *count > 1).count();
    let mut holdout_remaining = holdout_query_count.max(min_holdout);
    let mut tune_counts = Vec::new();
    let mut holdout_counts = Vec::new();
    for bucket in private_query_test_buckets() {
        let count = bucket_counts
            .iter()
            .filter(|(candidate, _)| candidate == bucket)
            .map(|(_, count)| *count)
            .sum::<usize>();
        let holdout_count = usize::from(count > 1 && holdout_remaining > 0);
        holdout_remaining = holdout_remaining.saturating_sub(holdout_count);
        tune_counts.push((bucket, count - holdout_count));
        holdout_counts.push((bucket, holdout_count));
    }
    for ((_, tune_count), (_, holdout_count)) in
        tune_counts.iter_mut().zip(holdout_counts.iter_mut())
    {
        if holdout_remaining == 0 {
            break;
        }
        let max_moved = if *tune_count > 1 {
            *tune_count - 1
        } else if *holdout_count == 0 {
            *tune_count
        } else {
            0
        };
        let moved = holdout_remaining.min(max_moved);
        *tune_count -= moved;
        *holdout_count += moved;
        holdout_remaining -= moved;
    }
    let tune_json = tune_counts
        .iter()
        .map(|(bucket, count)| format!("\"{bucket}\":{count}"))
        .collect::<Vec<_>>()
        .join(",");
    let holdout_json = holdout_counts
        .iter()
        .map(|(bucket, count)| format!("\"{bucket}\":{count}"))
        .collect::<Vec<_>>()
        .join(",");
    (format!("{{{tune_json}}}"), format!("{{{holdout_json}}}"))
}

pub(crate) fn assert_private_query_stage_latency(
    report: &serde_json::Value,
    expected_samples: u64,
) {
    let stages = report["stage_latency_ms"]
        .as_object()
        .expect("stage_latency_ms should be an object");
    assert_eq!(stages.len(), 7);
    for stage in [
        "query_parse",
        "prefilter",
        "bm25",
        "ann",
        "fusion",
        "bulk_hydrate",
        "snippet",
    ] {
        let summary = stages
            .get(stage)
            .and_then(serde_json::Value::as_object)
            .unwrap_or_else(|| panic!("{stage} stage latency should be an object"));
        assert_eq!(summary["samples"], expected_samples);
        let min = summary["min"]
            .as_f64()
            .expect("stage min should be numeric");
        let mean = summary["mean"]
            .as_f64()
            .expect("stage mean should be numeric");
        let p50 = summary["p50"]
            .as_f64()
            .expect("stage p50 should be numeric");
        let p95 = summary["p95"]
            .as_f64()
            .expect("stage p95 should be numeric");
        let p99 = summary["p99"]
            .as_f64()
            .expect("stage p99 should be numeric");
        let max = summary["max"]
            .as_f64()
            .expect("stage max should be numeric");
        assert!(min <= mean);
        assert!(mean <= max);
        assert!(min <= p50);
        assert!(p50 <= p95);
        assert!(p95 <= p99);
        assert!(p99 <= max);
    }
}

pub(crate) fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s17-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}
