use std::fs;
use std::path::{Path, PathBuf};

use crate::private_query_support::{
    private_query_test_shape_for_bucket, temp_dir,
    write_private_query_set_summary_with_split_and_source_kind,
};

pub(crate) fn private_query_set_file_with_bucket_queries(
    label: &str,
    bucket_queries: &[(&str, &str)],
) -> PathBuf {
    let path = temp_dir(label).join("private-query-set.jsonl");
    let mut lines = String::new();
    let mut bucket_counts = Vec::<(&str, usize)>::new();
    for (index, (bucket, query)) in bucket_queries.iter().enumerate() {
        lines.push_str(
            &serde_json::json!({
                "schema_version": "resume-ir.query-set.jsonl.v2",
                "sample_id": format!("private-query-sample-{index:06}"),
                "bucket": bucket,
                "query": query,
                "source_kind": "trace_source_search_v1",
                "query_shape": private_query_test_shape_for_bucket(bucket),
            })
            .to_string(),
        );
        lines.push('\n');
        if let Some((_, count)) = bucket_counts
            .iter_mut()
            .find(|(candidate, _)| candidate == bucket)
        {
            *count += 1;
        } else {
            bucket_counts.push((bucket, 1));
        }
    }
    fs::write(&path, lines).unwrap();
    write_private_query_set_summary(&path, bucket_queries.len(), &bucket_counts);
    path
}

pub(crate) fn private_query_set_file_with_custom_shape_query(
    label: &str,
    bucket: &str,
    query: &str,
    query_shape: serde_json::Value,
) -> PathBuf {
    let path = temp_dir(label).join("private-query-set.jsonl");
    fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::json!({
                "schema_version": "resume-ir.query-set.jsonl.v2",
                "sample_id": "private-query-sample-000000",
                "bucket": bucket,
                "query": query,
                "source_kind": "trace_source_search_v1",
                "query_shape": query_shape,
            })
        ),
    )
    .unwrap();
    write_private_query_set_summary(&path, 1, &[(bucket, 1)]);
    path
}

pub(crate) fn private_query_set_file_without_summary(label: &str, query_count: usize) -> PathBuf {
    let path = temp_dir(label).join("private-query-set.jsonl");
    let mut lines = String::new();
    for index in 0..query_count {
        lines.push_str(&format!(
            "{{\"schema_version\":\"resume-ir.query-set.jsonl.v2\",\"sample_id\":\"private-query-sample-{index:06}\",\"bucket\":\"and_3_5\",\"query\":\"REDACTION_SENTINEL_PRIVATE_QUERY backend search\",\"source_kind\":\"trace_source_search_v1\",\"query_shape\":{{\"term_count\":3,\"has_boolean\":false,\"has_location\":false,\"has_years\":false,\"has_degree\":false,\"has_skill\":true,\"has_phrase\":false}}}}\n"
        ));
    }
    fs::write(&path, lines).unwrap();
    path
}

pub(crate) fn legacy_private_query_set_file(label: &str, query_count: usize) -> PathBuf {
    let path = temp_dir(label).join("private-query-set.jsonl");
    let mut lines = String::new();
    for index in 0..query_count {
        lines.push_str(&format!(
            "{{\"sample_id\":\"private-query-sample-{index:06}\",\"query\":\"REDACTION_SENTINEL_PRIVATE_QUERY backend search {index}\"}}\n"
        ));
    }
    fs::write(&path, lines).unwrap();
    path
}

pub(crate) fn write_private_query_set_summary(
    query_set: &Path,
    query_count: usize,
    bucket_counts: &[(&str, usize)],
) {
    let (tune_query_count, holdout_query_count) = private_query_test_split_counts(query_count);
    write_private_query_set_summary_with_split(
        query_set,
        query_count,
        tune_query_count,
        holdout_query_count,
        bucket_counts,
    );
}

pub(crate) fn write_private_query_set_summary_with_split(
    query_set: &Path,
    query_count: usize,
    tune_query_count: usize,
    holdout_query_count: usize,
    bucket_counts: &[(&str, usize)],
) {
    write_private_query_set_summary_with_split_and_source_kind(
        query_set,
        query_count,
        tune_query_count,
        holdout_query_count,
        bucket_counts,
        "trace_source_search_v1",
    );
}

fn private_query_test_split_counts(query_count: usize) -> (usize, usize) {
    if query_count <= 1 {
        return (query_count, 0);
    }
    let holdout_query_count = (query_count / 5).max(1);
    (query_count - holdout_query_count, holdout_query_count)
}
