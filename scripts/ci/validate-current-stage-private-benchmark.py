#!/usr/bin/env python3
"""Validate redacted current-stage private query benchmark evidence."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


SCHEMA_VERSION = "benchmark.v1"
DATASET_KIND = "private-real-corpus"
PRIVACY_BOUNDARY = "redacted_local_aggregate"
SCOPE = "private local real-corpus query benchmark; aggregate redacted report only"
FULL_MIN_DOCUMENTS = 8000
FULL_MIN_QUERIES = 500
SMOKE_MIN_DOCUMENTS = 1
SMOKE_MIN_QUERIES = 1
FORBIDDEN_TEXT_MARKERS = (
    "PRIVATE-current-stage",
    "private fake query",
    "REDACTION_SENTINEL_PRIVATE_QUERY",
    "/Users/",
    "/private/",
    "\\Users\\",
)


def fail(message: str) -> None:
    raise SystemExit(message)


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


def validate_private_benchmark(document: dict[str, Any], validation_profile: str) -> None:
    reject_forbidden_text(document)

    require_string(document, "schema_version", SCHEMA_VERSION)
    require_string(document, "dataset_kind", DATASET_KIND)
    require_string(document, "corpus_origin", "private_local")
    require_string(document, "privacy_boundary", PRIVACY_BOUNDARY)
    require_string(document, "query_protocol", "resume-ir-query-v1")
    require_string(document, "query_mode", "hybrid")
    require_string(document, "retrieval_layers", "fulltext+field+vector+rrf")
    require_string(document, "query_embedding_runtime", "local-command")
    require_string(document, "scope", SCOPE)

    document_count = require_int(document, "document_count")
    searchable_count = require_int(document, "searchable_document_count")
    vector_count = require_int(document, "vector_indexed_document_count")
    query_count = require_int(document, "query_count")
    query_embedding_invocations = require_int(
        document, "query_embedding_command_invocations"
    )
    zero_result_queries = require_int(document, "zero_result_queries")

    if validation_profile == "full":
        min_documents = FULL_MIN_DOCUMENTS
        min_queries = FULL_MIN_QUERIES
        require_full_hot_index = True
    elif validation_profile == "smoke":
        min_documents = SMOKE_MIN_DOCUMENTS
        min_queries = SMOKE_MIN_QUERIES
        require_full_hot_index = False
    else:
        fail("validation_profile is invalid")

    if document_count < min_documents:
        fail("document_count below current-stage floor")
    if searchable_count > document_count:
        fail("searchable_document_count is inconsistent")
    if vector_count > searchable_count:
        fail("vector_indexed_document_count is inconsistent")
    if require_full_hot_index:
        if searchable_count < min_documents:
            fail("searchable_document_count is inconsistent")
        if vector_count < min_documents:
            fail("vector_indexed_document_count is inconsistent")
    else:
        if searchable_count < 1:
            fail("searchable_document_count is inconsistent")
        if vector_count < 1:
            fail("vector_indexed_document_count is inconsistent")
    if query_count < min_queries:
        fail("query_count below current-stage floor")
    if query_embedding_invocations != query_count:
        fail("query_embedding_command_invocations is inconsistent")
    if zero_result_queries > query_count:
        fail("zero_result_queries is inconsistent")

    latency = document.get("query_latency_ms")
    if not isinstance(latency, dict):
        fail("query_latency_ms must be an object")
    samples = require_int(latency, "samples")
    if samples < min_queries:
        fail("query latency samples below current-stage floor")
    p50 = require_number(latency, "p50")
    p95 = require_number(latency, "p95")
    p99 = require_number(latency, "p99")
    if not 0 <= p50 <= p95 <= p99:
        fail("query latency percentiles are inconsistent")

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
