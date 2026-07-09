#!/usr/bin/env python3
"""Validate redacted current-stage private query benchmark evidence."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
ACCEPTANCE_MATRIX = ROOT / "perf" / "acceptance-matrix.toml"
SCHEMA_VERSION = "benchmark.v1"
DATASET_KIND = "private-real-corpus"
PRIVACY_BOUNDARY = "redacted_local_aggregate"
SCOPE = "private local real-corpus query benchmark; aggregate redacted report only"
FULL_SCALE_GATE = "D10K_private_calibration"
SMOKE_MIN_DOCUMENTS = 1
SMOKE_MIN_QUERIES = 1
SMOKE_MIN_REQUEST_SAMPLES = 1
SMOKE_MIN_SAMPLES_PER_BUCKET = 0
FORBIDDEN_TEXT_MARKERS = (
    "PRIVATE-current-stage",
    "private fake query",
    "REDACTION_SENTINEL_PRIVATE_QUERY",
    "/Users/",
    "/private/",
    "\\Users\\",
)
STAGE_LATENCY_FIELDS = (
    "query_parse",
    "prefilter",
    "bm25",
    "ann",
    "fusion",
    "bulk_hydrate",
    "snippet",
)
STAGE_HISTOGRAM_BOUNDS_MS = (
    1.0,
    5.0,
    10.0,
    25.0,
    50.0,
    100.0,
    250.0,
    500.0,
    1000.0,
    2500.0,
    5000.0,
    10000.0,
    60000.0,
)
QUERY_BUCKETS = (
    "single_term",
    "and_2",
    "and_3_5",
    "and_6_16",
    "field_filter",
    "hybrid",
    "semantic",
)


def fail(message: str) -> None:
    raise SystemExit(message)


def load_full_scale_gate() -> dict[str, int]:
    try:
        with ACCEPTANCE_MATRIX.open("rb") as handle:
            matrix = tomllib.load(handle)
    except OSError:
        fail("acceptance matrix is unavailable")
    scale_gates = matrix.get("scale_gates")
    if not isinstance(scale_gates, dict):
        fail("acceptance matrix scale_gates is invalid")
    gate = scale_gates.get(FULL_SCALE_GATE)
    if not isinstance(gate, dict):
        fail("acceptance matrix D10K scale gate is missing")
    thresholds: dict[str, int] = {}
    for key in [
        "min_document_count",
        "min_searchable_document_count",
        "min_query_count",
        "min_request_sample_count",
        "min_samples_per_bucket",
    ]:
        value = gate.get(key)
        if not isinstance(value, int):
            fail(f"acceptance matrix D10K {key} is invalid")
        thresholds[key] = value
    return thresholds


FULL_SCALE_GATE_THRESHOLDS = load_full_scale_gate()


def read_json(path: Path) -> dict[str, Any]:
    try:
        with path.open(encoding="utf-8") as handle:
            document = json.load(handle)
    except OSError as error:
        fail(f"failed to read {path}: {error}")
    except json.JSONDecodeError as error:
        fail(f"invalid JSON in {path}: {error}")
    if not isinstance(document, dict):
        fail("private benchmark evidence must be a JSON object")
    return document


def reject_forbidden_text(value: Any, path: str = "$") -> None:
    if isinstance(value, str):
        for marker in FORBIDDEN_TEXT_MARKERS:
            if marker in value:
                fail(f"{path} contains forbidden private marker")
    elif isinstance(value, dict):
        for key, child in value.items():
            reject_forbidden_text(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_forbidden_text(child, f"{path}[{index}]")


def require_bool(document: dict[str, Any], field: str, expected: bool) -> None:
    if document.get(field) is not expected:
        fail(f"{field} must be {str(expected).lower()}")


def require_int(document: dict[str, Any], field: str) -> int:
    value = document.get(field)
    if not isinstance(value, int):
        fail(f"{field} must be an integer")
    return value


def require_number(document: dict[str, Any], field: str) -> float:
    value = document.get(field)
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        fail(f"{field} must be a number")
    return float(value)


def require_string(document: dict[str, Any], field: str, expected: str) -> None:
    if document.get(field) != expected:
        fail(f"{field} is invalid")


def require_sha256(document: dict[str, Any], field: str) -> None:
    value = document.get(field)
    if not isinstance(value, str) or len(value) != 64:
        fail(f"{field} must be a sha256 hex digest")
    try:
        int(value, 16)
    except ValueError:
        fail(f"{field} must be a sha256 hex digest")


def validate_latency_summary(
    document: dict[str, Any], field: str, expected_samples: int
) -> None:
    if not isinstance(document, dict):
        fail(f"{field} must be an object")
    samples = require_int(document, "samples")
    if samples != expected_samples:
        fail(f"{field} samples must match expected sample count")
    minimum = require_number(document, "min")
    mean = require_number(document, "mean")
    p50 = require_number(document, "p50")
    p95 = require_number(document, "p95")
    p99 = require_number(document, "p99")
    maximum = require_number(document, "max")
    if not (
        0 <= minimum <= mean <= maximum
        and minimum <= p50 <= p95 <= p99 <= maximum
    ):
        fail(f"{field} percentiles are inconsistent")


def validate_samples_per_bucket(
    value: Any, expected_samples: int, min_samples_per_bucket: int
) -> None:
    if not isinstance(value, dict):
        fail("samples_per_bucket must be an object")
    if set(value) != set(QUERY_BUCKETS):
        fail("samples_per_bucket fields are invalid")
    total = 0
    for bucket in QUERY_BUCKETS:
        count = require_int(value, bucket)
        if count < 0:
            fail(f"samples_per_bucket.{bucket} is inconsistent")
        if count < min_samples_per_bucket:
            fail(f"samples_per_bucket.{bucket} below current-stage floor")
        total += count
    if total != expected_samples:
        fail("samples_per_bucket must sum to request_sample_count")


def validate_bucket_count_map(value: Any, field: str) -> dict[str, int]:
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    if set(value) != set(QUERY_BUCKETS):
        fail(f"{field} fields are invalid")
    output = {}
    for bucket in QUERY_BUCKETS:
        count = value.get(bucket)
        if not isinstance(count, int) or count < 0:
            fail(f"{field}.{bucket} is inconsistent")
        output[bucket] = count
    return output


def validate_query_split_counts(document: dict[str, Any], query_count: int) -> None:
    bucket_counts = validate_bucket_count_map(
        document.get("bucket_counts"), "bucket_counts"
    )
    tune_bucket_counts = validate_bucket_count_map(
        document.get("tune_bucket_counts"), "tune_bucket_counts"
    )
    holdout_bucket_counts = validate_bucket_count_map(
        document.get("holdout_bucket_counts"), "holdout_bucket_counts"
    )
    if sum(bucket_counts.values()) != query_count:
        fail("bucket_counts must sum to query_count")
    if sum(tune_bucket_counts.values()) + sum(holdout_bucket_counts.values()) != query_count:
        fail("tune_bucket_counts and holdout_bucket_counts must sum to query_count")
    for bucket in QUERY_BUCKETS:
        if tune_bucket_counts[bucket] + holdout_bucket_counts[bucket] != bucket_counts[bucket]:
            fail(f"query split count mismatch for {bucket}")


def validate_latency_by_bucket(
    value: Any, samples_per_bucket: dict[str, Any], field: str
) -> None:
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    if not set(value).issubset(set(QUERY_BUCKETS)):
        fail(f"{field} fields are invalid")
    for bucket in QUERY_BUCKETS:
        sample_count = require_int(samples_per_bucket, bucket)
        if sample_count == 0:
            if bucket in value:
                fail(f"{field}.{bucket} is inconsistent")
            continue
        if bucket not in value:
            fail(f"{field}.{bucket} is missing")
        validate_latency_summary(value[bucket], f"{field}.{bucket}", sample_count)


def validate_query_latency_by_bucket(
    value: Any, samples_per_bucket: dict[str, Any]
) -> None:
    validate_latency_by_bucket(value, samples_per_bucket, "query_latency_by_bucket")


def validate_stage_latency(value: Any, field: str, expected_samples: int) -> None:
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    if set(value) != set(STAGE_LATENCY_FIELDS):
        fail(f"{field} fields are invalid")
    for stage in STAGE_LATENCY_FIELDS:
        validate_latency_summary(value[stage], f"{field}.{stage}", expected_samples)


def validate_stage_latency_by_bucket(
    value: Any, samples_per_bucket: dict[str, Any]
) -> None:
    if not isinstance(value, dict):
        fail("stage_latency_by_bucket_ms must be an object")
    if not set(value).issubset(set(QUERY_BUCKETS)):
        fail("stage_latency_by_bucket_ms fields are invalid")
    for bucket in QUERY_BUCKETS:
        sample_count = require_int(samples_per_bucket, bucket)
        if sample_count == 0:
            if bucket in value:
                fail(f"stage_latency_by_bucket_ms.{bucket} is inconsistent")
            continue
        if bucket not in value:
            fail(f"stage_latency_by_bucket_ms.{bucket} is missing")
        validate_stage_latency(
            value[bucket], f"stage_latency_by_bucket_ms.{bucket}", sample_count
        )


def validate_histogram_summary(value: Any, field: str, expected_samples: int) -> None:
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    samples = require_int(value, "samples")
    if samples != expected_samples:
        fail(f"{field} samples must match expected sample count")
    bins = value.get("bins")
    if not isinstance(bins, list) or len(bins) != len(STAGE_HISTOGRAM_BOUNDS_MS):
        fail(f"{field} bins are invalid")
    previous_count = 0
    for index, expected_le_ms in enumerate(STAGE_HISTOGRAM_BOUNDS_MS):
        bin_value = bins[index]
        if not isinstance(bin_value, dict):
            fail(f"{field} bins are invalid")
        le_ms = require_number(bin_value, "le_ms")
        count = require_int(bin_value, "count")
        if (
            abs(le_ms - expected_le_ms) > 0.000001
            or count < previous_count
            or count > samples
        ):
            fail(f"{field} bins are inconsistent")
        previous_count = count
    overflow_count = require_int(value, "overflow_count")
    if previous_count + overflow_count != samples:
        fail(f"{field} overflow_count is inconsistent")


def validate_stage_histogram(value: Any, field: str, expected_samples: int) -> None:
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    if set(value) != set(STAGE_LATENCY_FIELDS):
        fail(f"{field} fields are invalid")
    for stage in STAGE_LATENCY_FIELDS:
        validate_histogram_summary(value[stage], f"{field}.{stage}", expected_samples)


def validate_stage_histogram_by_bucket(
    value: Any, samples_per_bucket: dict[str, Any]
) -> None:
    if not isinstance(value, dict):
        fail("stage_histogram_by_bucket_ms must be an object")
    if not set(value).issubset(set(QUERY_BUCKETS)):
        fail("stage_histogram_by_bucket_ms fields are invalid")
    for bucket in QUERY_BUCKETS:
        sample_count = require_int(samples_per_bucket, bucket)
        if sample_count == 0:
            if bucket in value:
                fail(f"stage_histogram_by_bucket_ms.{bucket} is inconsistent")
            continue
        if bucket not in value:
            fail(f"stage_histogram_by_bucket_ms.{bucket} is missing")
        validate_stage_histogram(
            value[bucket], f"stage_histogram_by_bucket_ms.{bucket}", sample_count
        )


def validate_private_benchmark(document: dict[str, Any], validation_profile: str) -> None:
    reject_forbidden_text(document)

    require_string(document, "schema_version", SCHEMA_VERSION)
    require_string(document, "dataset_kind", DATASET_KIND)
    require_string(document, "corpus_origin", "private_local")
    require_string(document, "privacy_boundary", PRIVACY_BOUNDARY)
    require_string(document, "query_protocol", "resume-ir-query-v2")
    require_string(document, "query_source", "trace_source_search_v1")
    private_scale_gate = document.get("private_scale_gate")
    if private_scale_gate is not None and private_scale_gate not in {
        "D10K_private_calibration",
        "D100K_weak_host",
        "D1M_scale",
    }:
        fail("private_scale_gate is invalid")
    require_string(document, "query_runner", "resident-batch-command")
    require_string(document, "query_mode", "hybrid")
    require_string(document, "retrieval_layers", "fulltext+field+vector+rrf")
    require_string(
        document,
        "warm_or_cold_definition",
        "current_stage_single_resident_batch_no_extra_warmup",
    )
    require_string(
        document,
        "cache_state",
        "hot_index_fully_covered_resident_batch_os_cache_uncontrolled",
    )
    require_string(document, "query_embedding_runtime", "local-command")
    require_string(document, "scope", SCOPE)

    document_count = require_int(document, "document_count")
    searchable_count = require_int(document, "searchable_document_count")
    vector_count = require_int(document, "vector_indexed_document_count")
    query_count = require_int(document, "query_count")
    request_sample_count = require_int(document, "request_sample_count")
    query_embedding_invocations = require_int(
        document, "query_embedding_command_invocations"
    )
    zero_result_queries = require_int(document, "zero_result_queries")
    require_bool(document, "spawn_per_query", False)

    if validation_profile == "full":
        min_documents = FULL_SCALE_GATE_THRESHOLDS["min_document_count"]
        min_searchable_documents = FULL_SCALE_GATE_THRESHOLDS[
            "min_searchable_document_count"
        ]
        min_vector_indexed_documents = FULL_SCALE_GATE_THRESHOLDS[
            "min_searchable_document_count"
        ]
        min_queries = FULL_SCALE_GATE_THRESHOLDS["min_query_count"]
        min_request_samples = FULL_SCALE_GATE_THRESHOLDS["min_request_sample_count"]
        min_samples_per_bucket = FULL_SCALE_GATE_THRESHOLDS["min_samples_per_bucket"]
        if private_scale_gate != FULL_SCALE_GATE:
            fail("private_scale_gate must be D10K_private_calibration for full current-stage baseline")
    elif validation_profile == "smoke":
        min_documents = SMOKE_MIN_DOCUMENTS
        min_searchable_documents = SMOKE_MIN_DOCUMENTS
        min_vector_indexed_documents = SMOKE_MIN_DOCUMENTS
        min_queries = SMOKE_MIN_QUERIES
        min_request_samples = SMOKE_MIN_REQUEST_SAMPLES
        min_samples_per_bucket = SMOKE_MIN_SAMPLES_PER_BUCKET
    else:
        fail("validation_profile is invalid")

    if document_count < min_documents:
        fail("document_count below current-stage floor")
    if searchable_count > document_count:
        fail("searchable_document_count is inconsistent")
    if vector_count > searchable_count:
        fail("vector_indexed_document_count is inconsistent")
    if searchable_count < min_searchable_documents:
        fail("searchable_document_count below current-stage floor")
    if vector_count < min_vector_indexed_documents:
        fail("vector_indexed_document_count below current-stage floor")
    if query_count < min_queries:
        fail("query_count below current-stage floor")
    if request_sample_count < min_request_samples:
        fail("request_sample_count below current-stage floor")
    if request_sample_count < query_count:
        fail("request_sample_count is inconsistent")
    if query_embedding_invocations != request_sample_count:
        fail("query_embedding_command_invocations is inconsistent")
    if zero_result_queries > request_sample_count:
        fail("zero_result_queries is inconsistent")
    samples_per_bucket = document.get("samples_per_bucket")
    validate_samples_per_bucket(
        samples_per_bucket,
        request_sample_count,
        min_samples_per_bucket,
    )
    validate_query_split_counts(document, query_count)
    validate_query_latency_by_bucket(
        document.get("query_latency_by_bucket"), samples_per_bucket
    )

    validate_latency_summary(
        document.get("query_latency_ms"), "query_latency_ms", request_sample_count
    )
    validate_stage_latency(
        document.get("stage_latency_ms"), "stage_latency_ms", request_sample_count
    )
    validate_stage_latency_by_bucket(
        document.get("stage_latency_by_bucket_ms"), samples_per_bucket
    )
    validate_stage_histogram(
        document.get("stage_histogram_ms"),
        "stage_histogram_ms",
        request_sample_count,
    )
    validate_stage_histogram_by_bucket(
        document.get("stage_histogram_by_bucket_ms"), samples_per_bucket
    )
    validate_latency_summary(
        document.get("rss_delta_mb"), "rss_delta_mb", request_sample_count
    )
    validate_latency_by_bucket(
        document.get("rss_delta_mb_by_bucket"),
        samples_per_bucket,
        "rss_delta_mb_by_bucket",
    )

    require_bool(document, "million_scale_verified", False)
    require_bool(document, "hot_index", True)
    require_bool(document, "hot_path_ocr", False)
    require_bool(document, "hot_path_parsing", False)
    require_bool(document, "hot_path_heavy_model_inference", False)
    require_bool(document, "contains_raw_resume_text", False)
    require_bool(document, "contains_resume_paths", False)
    require_bool(document, "contains_queries", False)

    for field in (
        "dataset_manifest_sha256",
        "query_set_sha256",
        "tune_sha256",
        "holdout_sha256",
        "model_manifest_sha256",
        "corpus_summary_sha256",
    ):
        require_sha256(document, field)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate redacted current-stage private query benchmark evidence."
    )
    parser.add_argument(
        "--private-benchmark",
        type=Path,
        required=True,
        help="Path to private-benchmark-local.json.",
    )
    parser.add_argument(
        "--validation-profile",
        choices=("full", "smoke"),
        default="full",
        help="Current-stage validation profile; smoke permits partial hot-index coverage.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    validate_private_benchmark(read_json(args.private_benchmark), args.validation_profile)
    return 0


if __name__ == "__main__":
    sys.exit(main())
