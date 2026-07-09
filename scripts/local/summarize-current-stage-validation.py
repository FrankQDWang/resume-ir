#!/usr/bin/env python3
"""Build a redacted current-stage handoff summary from local validation output."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
ACCEPTANCE_MATRIX = ROOT / "perf" / "acceptance-matrix.toml"
HANDOFF_SCHEMA = "resume-ir.current-stage-handoff.v1"
HANDOFF_PRIVACY_BOUNDARY = "local_only_redacted_handoff"
D10K_PRIVATE_SCALE_GATE = "D10K_private_calibration"
SUPPORTED_SCHEMAS = {
    "resume-ir.current-stage-smoke-summary.v1",
    "resume-ir.current-stage-blocked-summary.v1",
    "resume-ir.current-stage-validation-evidence.v1",
}
EXPECTED_SOURCE_PRIVACY_BOUNDARIES = {
    "resume-ir.current-stage-smoke-summary.v1": "local_only_redacted_aggregate_summary",
    "resume-ir.current-stage-blocked-summary.v1": "local_only_redacted_blocked_summary",
    "resume-ir.current-stage-validation-evidence.v1": "local_only_redacted_evidence_manifest",
}
PRIVATE_QUERY_BASELINE_COPY_KEYS = (
    "privacy_boundary",
    "dataset_kind",
    "document_count",
    "searchable_document_count",
    "vector_indexed_document_count",
    "query_count",
    "request_sample_count",
    "query_source",
    "private_scale_gate",
    "query_set_sha256",
    "tune_sha256",
    "holdout_sha256",
    "bucket_counts",
    "tune_bucket_counts",
    "holdout_bucket_counts",
    "samples_per_bucket",
    "query_latency_ms",
    "query_latency_by_bucket",
    "stage_latency_p95_ms",
    "stage_latency_by_bucket_p95_ms",
    "rss_delta_mb",
    "rss_delta_mb_by_bucket",
    "zero_result_queries",
    "query_runner",
    "query_mode",
    "retrieval_layers",
    "warm_or_cold_definition",
    "cache_state",
    "percentile_confidence",
    "spawn_per_query",
    "hot_index",
    "hot_path_ocr",
    "hot_path_parsing",
    "hot_path_heavy_model_inference",
    "contains_raw_resume_text",
    "contains_resume_paths",
    "contains_queries",
)
BASELINE_ARTIFACT_REF_FIELDS = {
    "private-benchmark-local.json": "benchmark_report_hash",
    "private-query-set.summary.json": "query_set_summary_hash",
}
BLOCKED_ARTIFACT_REF_FILES = {
    "query-set-trace-preflight.local.json",
}
PRIVATE_MARKER = re.compile(r"PRIVATE-|/Users/|/home/|[A-Za-z]:\\")


def fail(message: str) -> None:
    print(f"current-stage handoff blocked: {message}", file=sys.stderr)
    raise SystemExit(2)


def load_d10k_scale_gate() -> dict[str, int]:
    try:
        with ACCEPTANCE_MATRIX.open("rb") as handle:
            matrix = tomllib.load(handle)
    except OSError:
        fail("acceptance matrix is unavailable")
    scale_gates = matrix.get("scale_gates")
    if not isinstance(scale_gates, dict):
        fail("acceptance matrix scale_gates is invalid")
    gate = scale_gates.get(D10K_PRIVATE_SCALE_GATE)
    if not isinstance(gate, dict):
        fail("acceptance matrix D10K scale gate is missing")
    thresholds: dict[str, int] = {}
    for key in [
        "min_document_count",
        "min_searchable_document_count",
        "min_query_count",
        "min_request_sample_count",
    ]:
        value = gate.get(key)
        if not isinstance(value, int):
            fail(f"acceptance matrix D10K {key} is invalid")
        thresholds[key] = value
    return thresholds


D10K_SCALE_GATE = load_d10k_scale_gate()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize redacted current-stage validation evidence."
    )
    parser.add_argument("--input", required=True, help="redacted summary/evidence JSON")
    parser.add_argument("--out", required=True, help="handoff JSON output path")
    parser.add_argument(
        "--issue-comment-out",
        help="optional redacted GitHub issue comment body output path",
    )
    return parser.parse_args()


def load_json(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as handle:
            value = json.load(handle)
    except OSError:
        fail("input is unavailable")
    except json.JSONDecodeError:
        fail("input is not valid JSON")
    if not isinstance(value, dict):
        fail("input must be a JSON object")
    return value


def reject_private_markers(value: Any) -> None:
    if isinstance(value, str):
        if PRIVATE_MARKER.search(value):
            fail("input contains a private marker")
        return
    if isinstance(value, list):
        for item in value:
            reject_private_markers(item)
        return
    if isinstance(value, dict):
        for key, item in value.items():
            reject_private_markers(key)
            reject_private_markers(item)
        return


def bool_field(document: dict[str, Any], name: str) -> bool:
    value = document.get(name)
    if not isinstance(value, bool):
        fail(f"missing boolean field: {name}")
    return value


def string_field(document: dict[str, Any], name: str) -> str:
    value = document.get(name)
    if not isinstance(value, str) or not value:
        fail(f"missing string field: {name}")
    return value


def optional_string(document: dict[str, Any], name: str) -> str | None:
    value = document.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        fail(f"invalid string field: {name}")
    return value


def source_status(document: dict[str, Any], schema: str) -> str:
    if schema == "resume-ir.current-stage-blocked-summary.v1":
        return "blocked"
    if schema == "resume-ir.current-stage-smoke-summary.v1":
        return "smoke_satisfied" if bool_field(document, "smoke_satisfied") else "smoke_failed"
    return "full_evidence_ready"


def validation_profile(document: dict[str, Any], schema: str) -> str:
    value = document.get("validation_profile")
    if isinstance(value, str) and value:
        return value
    if schema == "resume-ir.current-stage-validation-evidence.v1":
        return "full"
    fail("validation profile is missing")


def completed_steps(document: dict[str, Any]) -> list[dict[str, str]]:
    steps = document.get("steps", [])
    if not isinstance(steps, list):
        fail("steps must be an array")
    completed: list[dict[str, str]] = []
    for step in steps:
        if not isinstance(step, dict):
            fail("step must be an object")
        step_id = step.get("id")
        status = step.get("status")
        if not isinstance(step_id, str) or not isinstance(status, str):
            fail("step id/status must be strings")
        if status in {"success", "smoke_success", "expected_blocked"}:
            completed.append({"id": step_id, "status": status})
    return completed


def not_complete_items(document: dict[str, Any]) -> list[dict[str, str]]:
    items = document.get("not_completed", [])
    if items is None:
        items = []
    if not isinstance(items, list):
        fail("not_completed must be an array")
    output = []
    for item in items:
        if not isinstance(item, str) or not item:
            fail("not_completed entries must be strings")
        output.append({"kind": "not_complete", "item": item})
    if document.get("schema_version") == "resume-ir.current-stage-blocked-summary.v1":
        output.insert(
            0,
            {
                "kind": "blocked",
                "step": string_field(document, "blocked_step"),
                "category": string_field(document, "blocked_category"),
                "reason": string_field(document, "blocked_reason"),
            },
        )
    return output


def source_requires_observability(document: dict[str, Any], schema: str) -> bool:
    if schema in {
        "resume-ir.current-stage-smoke-summary.v1",
        "resume-ir.current-stage-validation-evidence.v1",
    }:
        return True
    if schema == "resume-ir.current-stage-blocked-summary.v1":
        steps = document.get("steps", [])
        if not isinstance(steps, list):
            fail("steps must be an array")
        return any(
            isinstance(step, dict)
            and step.get("id") == "corpus_summary"
            and step.get("status") == "success"
            for step in steps
        )
    return False


def observability(document: dict[str, Any], schema: str) -> dict[str, Any]:
    value = document.get("corpus_summary_observability")
    if value is None:
        if source_requires_observability(document, schema):
            fail("corpus_summary_observability is required for this handoff source")
        value = {}
    if not isinstance(value, dict):
        fail("corpus_summary_observability must be an object")
    allowed = {
        "privacy_boundary",
        "document_count",
        "searchable_document_count",
        "vector_indexed_document_count",
        "hot_index_fully_covered",
        "document_status_counts",
        "ingest_job_status_counts",
        "ingest_job_kind_status_counts",
        "ingest_job_failure_counts",
        "contains_raw_resume_text",
        "contains_resume_paths",
        "contains_queries",
        "contains_sample_ids",
    }
    return {key: value[key] for key in sorted(allowed) if key in value}


def histogram_stage_shape(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        return {
            "stage_count": 0,
            "stage_names": [],
            "histogram_bin_count": 0,
            "samples": 0,
            "overflow_included": False,
        }
    max_bin_count = 0
    max_samples = 0
    overflow_included = False
    stage_names: list[str] = []
    for stage, histogram in value.items():
        if not isinstance(histogram, dict):
            continue
        if isinstance(stage, str) and stage:
            stage_names.append(stage)
        bins = histogram.get("bins")
        if isinstance(bins, list):
            max_bin_count = max(max_bin_count, len(bins))
        sample_count = histogram.get("samples")
        if isinstance(sample_count, int) and sample_count > max_samples:
            max_samples = sample_count
        overflow_included = overflow_included or "overflow_count" in histogram
    return {
        "stage_count": len(stage_names),
        "stage_names": sorted(stage_names),
        "histogram_bin_count": max_bin_count,
        "samples": max_samples,
        "overflow_included": overflow_included,
    }


def stage_histogram_summary(value: dict[str, Any]) -> dict[str, Any]:
    global_shape = histogram_stage_shape(value.get("stage_histogram_ms"), "stage_histogram_ms")
    by_bucket = value.get("stage_histogram_by_bucket_ms")
    bucket_sample_counts: dict[str, int] = {}
    bucket_names: list[str] = []
    max_bin_count = global_shape["histogram_bin_count"]
    overflow_included = global_shape["overflow_included"]
    if isinstance(by_bucket, dict):
        for bucket, stages in by_bucket.items():
            if not isinstance(bucket, str) or not bucket:
                continue
            bucket_shape = histogram_stage_shape(stages, f"stage_histogram_by_bucket_ms.{bucket}")
            bucket_sample_counts[bucket] = bucket_shape["samples"]
            bucket_names.append(bucket)
            max_bin_count = max(max_bin_count, bucket_shape["histogram_bin_count"])
            overflow_included = overflow_included or bucket_shape["overflow_included"]
    return {
        "global_stage_count": global_shape["stage_count"],
        "global_stage_names": global_shape["stage_names"],
        "global_samples": global_shape["samples"],
        "bucket_count": len(bucket_names),
        "bucket_names": sorted(bucket_names),
        "bucket_sample_counts": bucket_sample_counts,
        "histogram_bin_count": max_bin_count,
        "overflow_included": overflow_included,
    }


def private_query_baseline_summary(document: dict[str, Any]) -> dict[str, Any] | None:
    value = document.get("private_query_observability")
    if value is None:
        return None
    if not isinstance(value, dict):
        fail("private_query_observability must be an object")
    if value.get("private_scale_gate") != D10K_PRIVATE_SCALE_GATE:
        fail("private_query_observability.private_scale_gate must be D10K_private_calibration")
    if int_at(value, "document_count") < D10K_SCALE_GATE["min_document_count"]:
        fail("private_query_observability.document_count is below D10K")
    if int_at(value, "searchable_document_count") < D10K_SCALE_GATE["min_searchable_document_count"]:
        fail("private_query_observability.searchable_document_count is below D10K")
    if int_at(value, "vector_indexed_document_count") < D10K_SCALE_GATE["min_searchable_document_count"]:
        fail("private_query_observability.vector_indexed_document_count is below D10K")
    if int_at(value, "query_count") < D10K_SCALE_GATE["min_query_count"]:
        fail("private_query_observability.query_count is below D10K")
    if int_at(value, "request_sample_count") < D10K_SCALE_GATE["min_request_sample_count"]:
        fail("private_query_observability.request_sample_count is below D10K")
    summary = {
        key: value[key]
        for key in PRIVATE_QUERY_BASELINE_COPY_KEYS
        if key in value
    }
    summary["stage_histogram_summary"] = stage_histogram_summary(value)
    return summary


def baseline_artifact_refs(document: dict[str, Any]) -> list[dict[str, str]]:
    value = document.get("redacted_outputs", [])
    if not isinstance(value, list):
        fail("redacted_outputs must be an array")
    refs: list[dict[str, str]] = []
    seen: set[str] = set()
    for item in value:
        if not isinstance(item, dict):
            fail("redacted_outputs entries must be objects")
        file_name = item.get("file")
        if file_name not in BASELINE_ARTIFACT_REF_FIELDS:
            continue
        sha256 = item.get("sha256")
        if not isinstance(sha256, str) or not sha256:
            fail(f"redacted_outputs.{file_name}.sha256 is missing")
        if file_name in seen:
            fail(f"redacted_outputs has duplicate baseline artifact: {file_name}")
        seen.add(file_name)
        refs.append({"file": file_name, "sha256": sha256})
    return sorted(refs, key=lambda ref: ref["file"])


def blocked_artifact_refs(document: dict[str, Any], schema: str) -> list[dict[str, str]]:
    if schema != "resume-ir.current-stage-blocked-summary.v1":
        return []
    value = document.get("redacted_outputs", [])
    if not isinstance(value, list):
        fail("redacted_outputs must be an array")
    refs: list[dict[str, str]] = []
    seen: set[str] = set()
    for item in value:
        if not isinstance(item, dict):
            fail("redacted_outputs entries must be objects")
        file_name = item.get("file")
        if file_name not in BLOCKED_ARTIFACT_REF_FILES:
            continue
        sha256 = item.get("sha256")
        if not isinstance(sha256, str) or not sha256:
            fail(f"redacted_outputs.{file_name}.sha256 is missing")
        if file_name in seen:
            fail(f"redacted_outputs has duplicate blocked artifact: {file_name}")
        seen.add(file_name)
        refs.append({"file": file_name, "sha256": sha256})
    return sorted(refs, key=lambda ref: ref["file"])


def scalar_text(value: Any, name: str) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return str(value)
    if isinstance(value, str) and value:
        return value
    fail(f"issue comment field is not scalar: {name}")


def metric_line(label: str, value: Any) -> str:
    return f"- {label}: {scalar_text(value, label)}"


def latency_summary_line(label: str, value: Any) -> str:
    if not isinstance(value, dict):
        fail(f"issue comment latency summary is invalid: {label}")
    parts = []
    for key in ("samples", "p50", "p95", "p99"):
        if key not in value:
            fail(f"issue comment latency summary is missing {label}.{key}")
        parts.append(f"{key}={scalar_text(value[key], f'{label}.{key}')}")
    return f"- {label}: {', '.join(parts)}"


def bucket_counts_line(label: str, value: Any) -> str:
    if not isinstance(value, dict):
        fail(f"issue comment bucket counts are invalid: {label}")
    parts = [f"{key}={scalar_text(value[key], f'{label}.{key}')}" for key in sorted(value)]
    return f"- {label}: {', '.join(parts)}"


def stage_p95_line(value: Any) -> str:
    if not isinstance(value, dict):
        fail("issue comment stage_latency_p95_ms is invalid")
    parts = [
        f"{key}={scalar_text(value[key], f'stage_latency_p95_ms.{key}')}"
        for key in sorted(value)
    ]
    return f"- stage_latency_p95_ms: {', '.join(parts)}"


def issue_comment_body(handoff: dict[str, Any]) -> str:
    summary = handoff.get("private_query_baseline_summary")
    if summary is None:
        blocked = handoff.get("blocked")
        if isinstance(blocked, dict):
            return blocked_issue_comment_body(handoff, blocked)
        fail("issue comment requires private_query_baseline_summary or blocked handoff")
    return private_query_issue_comment_body(handoff, summary)


def private_query_issue_comment_body(
    handoff: dict[str, Any], summary: dict[str, Any]
) -> str:
    if not isinstance(summary, dict):
        fail("issue comment requires private_query_baseline_summary")
    histogram = summary.get("stage_histogram_summary")
    if not isinstance(histogram, dict):
        fail("issue comment requires stage_histogram_summary")
    required_fields = (
        "query_set_sha256",
        "tune_sha256",
        "holdout_sha256",
        "query_source",
        "private_scale_gate",
        "query_count",
        "request_sample_count",
        "query_runner",
        "query_mode",
        "retrieval_layers",
        "warm_or_cold_definition",
        "cache_state",
        "percentile_confidence",
        "spawn_per_query",
        "hot_path_ocr",
        "hot_path_parsing",
        "hot_path_heavy_model_inference",
        "contains_raw_resume_text",
        "contains_resume_paths",
        "contains_queries",
    )
    lines = [
        "#53 Current-Stage Private Query Baseline Handoff",
        "",
        "This is redacted current-stage handoff evidence; not goal_complete; not a profile optimization issue closure.",
        "",
        "## Workload",
    ]
    for field in required_fields[:12]:
        lines.append(metric_line(field, summary.get(field)))
    lines.append(bucket_counts_line("bucket_counts", summary.get("bucket_counts")))
    lines.append(bucket_counts_line("samples_per_bucket", summary.get("samples_per_bucket")))
    lines.extend(
        [
            "",
            "## Latency And Stages",
            latency_summary_line("query_latency_ms", summary.get("query_latency_ms")),
            stage_p95_line(summary.get("stage_latency_p95_ms")),
            latency_summary_line("rss_delta_mb", summary.get("rss_delta_mb")),
            (
                "- stage_histogram_shape: "
                f"global_stages={scalar_text(histogram.get('global_stage_count'), 'global_stage_count')}, "
                f"buckets={scalar_text(histogram.get('bucket_count'), 'bucket_count')}, "
                f"bins={scalar_text(histogram.get('histogram_bin_count'), 'histogram_bin_count')}, "
                f"overflow={scalar_text(histogram.get('overflow_included'), 'overflow_included')}"
            ),
            "",
            "## Privacy Boundary",
        ]
    )
    for field in required_fields[12:]:
        lines.append(metric_line(field, summary.get(field)))
    artifact_refs = handoff.get("baseline_artifact_refs", [])
    if artifact_refs:
        if not isinstance(artifact_refs, list):
            fail("issue comment baseline_artifact_refs must be an array")
        lines.extend(["", "## Redacted Artifact Refs"])
        for item in artifact_refs:
            if not isinstance(item, dict):
                fail("issue comment baseline_artifact_refs entries must be objects")
            file_name = item.get("file")
            sha256 = item.get("sha256")
            field = BASELINE_ARTIFACT_REF_FIELDS.get(file_name)
            if field is None or not isinstance(sha256, str) or not sha256:
                fail("issue comment baseline artifact ref is invalid")
            lines.append(f"- {field}: {sha256} ({file_name})")
    lines.extend(
        [
            "",
            "Do not attach raw query sets, private benchmark reports, trace logs, candidate results, local paths, diagnostics packages, indexes, SQLite databases, model caches, or resume files.",
        ]
    )
    return "\n".join(lines) + "\n"


def blocked_issue_comment_body(
    handoff: dict[str, Any], blocked: dict[str, Any]
) -> str:
    next_action_value = handoff.get("next_action")
    if not isinstance(next_action_value, dict):
        fail("issue comment requires next_action")
    lines = [
        "#53 Current-Stage Blocked Handoff",
        "",
        "This is redacted current-stage blocked evidence; not goal_complete; not a profile optimization issue closure.",
        "",
        "## Blocker",
        metric_line("current_stage_status", handoff.get("current_stage_status")),
        metric_line("validation_profile", handoff.get("validation_profile")),
        metric_line("current_stage_target", handoff.get("current_stage_target")),
        metric_line("blocked_step", blocked.get("blocked_step")),
        metric_line("blocked_category", blocked.get("blocked_category")),
        metric_line("blocked_reason", blocked.get("blocked_reason")),
        metric_line("private_corpus_read", blocked.get("private_corpus_read")),
        "",
        "## Observability",
    ]
    observability_value = handoff.get("observability")
    if isinstance(observability_value, dict):
        for field in (
            "document_count",
            "searchable_document_count",
            "vector_indexed_document_count",
            "hot_index_fully_covered",
        ):
            if field in observability_value:
                lines.append(metric_line(field, observability_value.get(field)))
    else:
        lines.append("- observability: null")
    trace_preflight = handoff.get("query_set_trace_preflight")
    if isinstance(trace_preflight, dict):
        lines.extend(
            [
                "",
                "## Query Set Preflight",
                metric_line("query_source", trace_preflight.get("query_source")),
                metric_line("query_index_available", trace_preflight.get("query_index_available")),
                metric_line("target_query_count", trace_preflight.get("target_query_count")),
                metric_line("document_count", trace_preflight.get("document_count")),
                metric_line(
                    "searchable_document_count",
                    trace_preflight.get("searchable_document_count"),
                ),
                metric_line(
                    "vector_indexed_document_count",
                    trace_preflight.get("vector_indexed_document_count"),
                ),
                metric_line("d10k_corpus_ready", trace_preflight.get("d10k_corpus_ready")),
                bucket_counts_line(
                    "d10k_corpus_deficits",
                    trace_preflight.get("d10k_corpus_deficits"),
                ),
                metric_line("trace_logs", trace_preflight.get("trace_logs")),
                metric_line("source_search_lines", trace_preflight.get("source_search_lines")),
                metric_line("extracted_queries", trace_preflight.get("extracted_queries")),
                metric_line(
                    "duplicate_queries_dropped",
                    trace_preflight.get("duplicate_queries_dropped"),
                ),
                metric_line(
                    "candidate_queries_sampled",
                    trace_preflight.get("candidate_queries_sampled"),
                ),
                bucket_counts_line(
                    "candidate_bucket_counts",
                    trace_preflight.get("candidate_bucket_counts"),
                ),
                bucket_counts_line(
                    "candidate_bucket_deficits",
                    trace_preflight.get("candidate_bucket_deficits"),
                ),
                metric_line("corpus_valid_queries", trace_preflight.get("corpus_valid_queries")),
                bucket_counts_line(
                    "corpus_valid_bucket_counts",
                    trace_preflight.get("corpus_valid_bucket_counts"),
                ),
                bucket_counts_line(
                    "corpus_valid_bucket_deficits",
                    trace_preflight.get("corpus_valid_bucket_deficits"),
                ),
            ]
        )
    lines.extend(
        [
            "",
            "## Next Action",
            metric_line(
                "recommended_next_step",
                next_action_value.get("recommended_next_step"),
            ),
            metric_line("do_not_do", next_action_value.get("do_not_do")),
        ]
    )
    artifact_refs = handoff.get("blocked_artifact_refs", [])
    if artifact_refs:
        if not isinstance(artifact_refs, list):
            fail("issue comment blocked_artifact_refs must be an array")
        lines.extend(["", "## Redacted Artifact Refs"])
        for item in artifact_refs:
            if not isinstance(item, dict):
                fail("issue comment blocked_artifact_refs entries must be objects")
            file_name = item.get("file")
            sha256 = item.get("sha256")
            if file_name not in BLOCKED_ARTIFACT_REF_FILES or not isinstance(sha256, str) or not sha256:
                fail("issue comment blocked artifact ref is invalid")
            lines.append(f"- redacted_artifact_hash: {sha256} ({file_name})")
    lines.extend(
        [
            "",
            "Do not attach raw query sets, private benchmark reports, trace logs, candidate results, local paths, diagnostics packages, indexes, SQLite databases, model caches, or resume files.",
        ]
    )
    return "\n".join(lines) + "\n"


def int_at(value: Any, key: str) -> int:
    if not isinstance(value, dict):
        return 0
    item = value.get(key)
    if isinstance(item, int) and item > 0:
        return item
    return 0


def nested_int_at(value: Any, parent: str, key: str) -> int:
    if not isinstance(value, dict):
        return 0
    return int_at(value.get(parent), key)


def derived_blockers(observability_value: dict[str, Any]) -> list[dict[str, Any]]:
    blockers: list[dict[str, Any]] = []
    status_counts = observability_value.get("document_status_counts")
    job_kind_counts = observability_value.get("ingest_job_kind_status_counts")

    failed_permanent = int_at(status_counts, "failed_permanent")
    if failed_permanent > 0:
        blockers.append(
            {
                "kind": "derived_blocker",
                "category": "import/parser",
                "reason": "failed_permanent_documents_present",
                "count": failed_permanent,
                "next_action": "inspect redacted diagnostics and fix parser/import failure classes",
            }
        )

    ocr_required = int_at(status_counts, "ocr_required")
    queued_ocr = nested_int_at(job_kind_counts, "ocr_document", "queued")
    running_ocr = nested_int_at(job_kind_counts, "ocr_document", "running")
    if ocr_required > 0 or queued_ocr > 0 or running_ocr > 0:
        blockers.append(
            {
                "kind": "derived_blocker",
                "category": "ocr",
                "reason": "ocr_backlog_present",
                "document_count": ocr_required,
                "queued_jobs": queued_ocr,
                "running_jobs": running_ocr,
                "next_action": "continue bounded OCR validation or carry backlog to performance/runtime follow-up",
            }
        )

    searchable = int_at(observability_value, "searchable_document_count")
    vector_indexed = int_at(observability_value, "vector_indexed_document_count")
    if searchable > 0 and vector_indexed < searchable:
        blockers.append(
            {
                "kind": "derived_blocker",
                "category": "embedding",
                "reason": "vector_index_backlog_present",
                "searchable_document_count": searchable,
                "vector_indexed_document_count": vector_indexed,
                "next_action": "continue bounded embedding/index validation with reviewed local model runtime",
            }
        )

    if observability_value.get("hot_index_fully_covered") is False:
        blockers.append(
            {
                "kind": "derived_blocker",
                "category": "benchmark",
                "reason": "hot_index_not_fully_covered",
                "next_action": "do not claim full baseline; rerun full validation after OCR/vector coverage is sufficient",
            }
        )

    return blockers


def preflight_probes(document: dict[str, Any]) -> dict[str, str]:
    value = document.get("preflight_probes", {})
    if not isinstance(value, dict):
        fail("preflight_probes must be an object")
    probes: dict[str, str] = {}
    for key in ("ocr_runtime_probe", "embedding_protocol"):
        item = value.get(key)
        if isinstance(item, str):
            probes[key] = item
    return probes


def query_set_trace_preflight(document: dict[str, Any]) -> dict[str, Any] | None:
    value = document.get("query_set_trace_preflight")
    if value is None:
        return None
    if not isinstance(value, dict):
        fail("query_set_trace_preflight must be an object")
    if value.get("schema_version") != "resume-ir.query-set-trace-preflight.v1":
        fail("query_set_trace_preflight schema is invalid")
    output: dict[str, Any] = {
        "schema_version": "resume-ir.query-set-trace-preflight.v1",
    }
    query_source = value.get("query_source")
    if isinstance(query_source, str) and query_source:
        output["query_source"] = query_source
    query_index_available = value.get("query_index_available")
    if not isinstance(query_index_available, bool):
        fail("query_set_trace_preflight.query_index_available is invalid")
    output["query_index_available"] = query_index_available
    d10k_corpus_ready = value.get("d10k_corpus_ready")
    if not isinstance(d10k_corpus_ready, bool):
        fail("query_set_trace_preflight.d10k_corpus_ready is invalid")
    output["d10k_corpus_ready"] = d10k_corpus_ready
    for field in (
        "target_query_count",
        "document_count",
        "searchable_document_count",
        "vector_indexed_document_count",
        "d10k_min_document_count",
        "d10k_min_searchable_document_count",
        "d10k_min_vector_indexed_document_count",
        "trace_logs",
        "trace_lines",
        "source_search_lines",
        "extracted_queries",
        "normalization_rejected",
        "duplicate_queries_dropped",
        "candidate_queries_sampled",
        "zero_hit_queries_dropped",
        "corpus_valid_queries",
    ):
        item = value.get(field)
        if not isinstance(item, int) or item < 0:
            fail(f"query_set_trace_preflight.{field} is invalid")
        output[field] = item
    for field in (
        "d10k_corpus_deficits",
        "candidate_bucket_counts",
        "candidate_bucket_deficits",
        "corpus_valid_bucket_counts",
        "required_bucket_counts",
        "corpus_valid_bucket_deficits",
    ):
        item = value.get(field)
        if not isinstance(item, dict) or not item:
            fail(f"query_set_trace_preflight.{field} is invalid")
        checked: dict[str, int] = {}
        for key, count in item.items():
            if not isinstance(key, str) or not key or not isinstance(count, int) or count < 0:
                fail(f"query_set_trace_preflight.{field} entry is invalid")
            checked[key] = count
        output[field] = checked
    return output


def must_not_upload(document: dict[str, Any]) -> list[str]:
    value = document.get("must_not_upload", [])
    if not isinstance(value, list):
        fail("must_not_upload must be an array")
    output = []
    for item in value:
        if not isinstance(item, str) or not item:
            fail("must_not_upload entries must be strings")
        if item not in output:
            output.append(item)
    return output


def next_action(document: dict[str, Any], schema: str) -> dict[str, str]:
    if schema == "resume-ir.current-stage-blocked-summary.v1":
        category = string_field(document, "blocked_category")
        blocked_reason = string_field(document, "blocked_reason")
        if blocked_reason == "query_set_index_unavailable":
            return {
                "status": "blocked",
                "category": category,
                "blocked_step": string_field(document, "blocked_step"),
                "recommended_next_step": (
                    "prepare or reuse an indexed local data-dir, then rerun "
                    "current-stage validation with the static replay query-set freeze"
                ),
                "do_not_do": (
                    "do not run private-query baseline, D10K calibration, or "
                    "P95/P99 optimization until query-set freeze succeeds"
                ),
            }
        if blocked_reason == "query_set_corpus_or_trace_coverage_insufficient":
            return {
                "status": "blocked",
                "category": category,
                "blocked_step": string_field(document, "blocked_step"),
                "recommended_next_step": (
                    "prepare a D10K-shaped indexed local corpus and collect more "
                    "trace-derived source_search workload for deficient buckets, then rerun "
                    "current-stage validation with the static replay query-set freeze"
                ),
                "do_not_do": (
                    "do not run private-query baseline, D10K calibration, or P95/P99 "
                    "optimization until the static query set can freeze"
                ),
            }
        return {
            "status": "blocked",
            "category": category,
            "blocked_step": string_field(document, "blocked_step"),
            "recommended_next_step": (
                f"fix {category} blocker and rerun current-stage validation"
            ),
            "do_not_do": (
                "do not chase P95/P99 optimization or require million-resume "
                "validation in current stage"
            ),
        }
    if schema == "resume-ir.current-stage-smoke-summary.v1":
        return {
            "status": "smoke_only",
            "recommended_next_step": "run full current-stage validation when local runtime and corpus are ready",
            "do_not_do": "do not treat smoke handoff as release-readiness evidence",
        }
    return {
        "status": "full_evidence_ready",
        "recommended_next_step": "feed current-stage evidence into release-readiness and continue remaining release blockers",
        "do_not_do": "do not claim complete product while release-readiness remains blocked",
    }


def build_handoff(document: dict[str, Any]) -> dict[str, Any]:
    schema = string_field(document, "schema_version")
    if schema not in SUPPORTED_SCHEMAS:
        fail("unsupported current-stage evidence schema")
    if (
        string_field(document, "privacy_boundary")
        != EXPECTED_SOURCE_PRIVACY_BOUNDARIES[schema]
    ):
        fail("source privacy_boundary does not match schema")
    reject_private_markers(document)

    blocked = None
    if schema == "resume-ir.current-stage-blocked-summary.v1":
        blocked = {
            "blocked_step": string_field(document, "blocked_step"),
            "blocked_category": string_field(document, "blocked_category"),
            "blocked_reason": string_field(document, "blocked_reason"),
            "blocked_exit": document.get("blocked_exit"),
            "private_corpus_read": bool_field(document, "private_corpus_read"),
        }

    observability_value = observability(document, schema)

    handoff = {
        "schema_version": HANDOFF_SCHEMA,
        "privacy_boundary": HANDOFF_PRIVACY_BOUNDARY,
        "source_schema": schema,
        "current_stage_status": source_status(document, schema),
        "validation_profile": validation_profile(document, schema),
        "current_stage_target": optional_string(document, "current_stage_target"),
        "complete_product": False,
        "full_baseline_satisfied": bool_field(document, "full_baseline_satisfied"),
        "release_readiness_evidence": bool_field(document, "release_readiness_evidence"),
        "performance_optimization_deferred": bool_field(
            document, "performance_optimization_deferred"
        ),
        "preflight_probes": preflight_probes(document),
        "blocked": blocked,
        "next_action": next_action(document, schema),
        "observability": observability_value,
        "derived_blockers": derived_blockers(observability_value),
        "completed_steps": completed_steps(document),
        "blocked_or_not_complete": not_complete_items(document),
        "must_not_upload": must_not_upload(document),
    }
    trace_preflight = query_set_trace_preflight(document)
    if trace_preflight is not None:
        handoff["query_set_trace_preflight"] = trace_preflight
    query_baseline = private_query_baseline_summary(document)
    if query_baseline is not None:
        handoff["private_query_baseline_summary"] = query_baseline
        artifact_refs = baseline_artifact_refs(document)
        if artifact_refs:
            handoff["baseline_artifact_refs"] = artifact_refs
    artifact_refs = blocked_artifact_refs(document, schema)
    if artifact_refs:
        handoff["blocked_artifact_refs"] = artifact_refs
    return handoff


def main() -> int:
    args = parse_args()
    source = load_json(Path(args.input))
    handoff = build_handoff(source)
    reject_private_markers(handoff)
    out_path = Path(args.out)
    if out_path.parent and str(out_path.parent) != ".":
        out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(
        json.dumps(handoff, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    if args.issue_comment_out:
        comment = issue_comment_body(handoff)
        reject_private_markers(comment)
        comment_path = Path(args.issue_comment_out)
        if comment_path.parent and str(comment_path.parent) != ".":
            comment_path.parent.mkdir(parents=True, exist_ok=True)
        comment_path.write_text(comment, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
