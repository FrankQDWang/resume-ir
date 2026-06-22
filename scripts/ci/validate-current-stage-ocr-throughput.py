#!/usr/bin/env python3
"""Validate redacted current-stage private OCR throughput evidence."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


SCHEMA_VERSION = "ocr-throughput.v1"
DATASET_KIND = "private-real-corpus"
PRIVACY_BOUNDARY = "redacted_local_aggregate"
SCOPE = "private real-corpus OCR throughput benchmark; aggregate redacted report only"
MIN_DOCUMENTS = 8000
MIN_PAGES = 500
FORBIDDEN_TEXT_MARKERS = (
    "PRIVATE-current-stage",
    "PRIVATE_OCR",
    "SYNTHETIC OCR",
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
        fail("OCR throughput evidence must be a JSON object")
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


def require_string(document: dict[str, Any], field: str, expected: str | None = None) -> str:
    value = document.get(field)
    if not isinstance(value, str):
        fail(f"{field} must be a string")
    if expected is not None and value != expected:
        fail(f"{field} is invalid")
    return value


def require_sha256(document: dict[str, Any], field: str) -> None:
    value = require_string(document, field)
    if len(value) != 64:
        fail(f"{field} must be a sha256 hex digest")
    try:
        int(value, 16)
    except ValueError:
        fail(f"{field} must be a sha256 hex digest")


def validate_ocr_throughput(document: dict[str, Any]) -> None:
    reject_forbidden_text(document)

    require_string(document, "schema_version", SCHEMA_VERSION)
    require_string(document, "dataset_kind", DATASET_KIND)
    require_string(document, "target_claim", "ocr_throughput_baseline_observed")
    require_string(document, "corpus_origin", "private_local")
    require_string(document, "privacy_boundary", PRIVACY_BOUNDARY)
    require_string(document, "scope", SCOPE)
    require_string(document, "run_id")
    require_string(document, "platform")
    require_string(document, "engine_kind")

    document_count = require_int(document, "document_count")
    scanned_document_count = require_int(document, "scanned_document_count")
    page_count = require_int(document, "page_count")
    failed_document_count = require_int(document, "failed_document_count")
    render_failure_count = require_int(document, "render_failure_count")
    ocr_failure_count = require_int(document, "ocr_failure_count")
    total_ms = require_number(document, "total_ms")
    pages_per_second = require_number(document, "pages_per_second")

    if document_count < MIN_DOCUMENTS:
        fail("document_count below current-stage floor")
    if scanned_document_count <= 0 or scanned_document_count > document_count:
        fail("scanned_document_count is inconsistent")
    if page_count < MIN_PAGES or page_count < scanned_document_count:
        fail("page_count is inconsistent")
    if failed_document_count > document_count:
        fail("failed_document_count is inconsistent")
    if render_failure_count + ocr_failure_count != failed_document_count:
        fail("OCR failure counts are inconsistent")
    if total_ms <= 0:
        fail("total_ms must be positive")
    expected_pages_per_second = page_count / (total_ms / 1000.0)
    if abs(pages_per_second - expected_pages_per_second) > 0.001:
        fail("pages_per_second is inconsistent")

    latency = document.get("page_latency_ms")
    if not isinstance(latency, dict):
        fail("page_latency_ms must be an object")
    samples = require_int(latency, "samples")
    p50 = require_number(latency, "p50")
    p95 = require_number(latency, "p95")
    p99 = require_number(latency, "p99")
    if samples != page_count:
        fail("page latency samples must match page_count")
    if not 0 <= p50 <= p95 <= p99:
        fail("page latency percentiles are inconsistent")

    require_bool(document, "run_budget_exhausted", False)
    require_bool(document, "contains_raw_ocr_text", False)
    require_bool(document, "contains_page_images", False)
    require_bool(document, "contains_resume_paths", False)
    require_bool(document, "contains_document_ids", False)
    require_bool(document, "contains_page_ids", False)
    require_bool(document, "contains_command_paths", False)

    for field in (
        "dataset_manifest_sha256",
        "ocr_runtime_manifest_sha256",
        "renderer_manifest_sha256",
        "language_pack_manifest_sha256",
    ):
        require_sha256(document, field)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate redacted current-stage private OCR throughput evidence."
    )
    parser.add_argument(
        "--ocr-throughput",
        type=Path,
        required=True,
        help="Path to private-ocr-throughput.json.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    validate_ocr_throughput(read_json(args.ocr_throughput))
    return 0


if __name__ == "__main__":
    sys.exit(main())
