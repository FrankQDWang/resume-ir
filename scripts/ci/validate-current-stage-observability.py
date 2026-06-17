#!/usr/bin/env python3
"""Validate redacted current-stage corpus observability evidence."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


CURRENT_STAGE_DOCUMENT_FLOOR = 8000
OBSERVABILITY_FIELD = "corpus_summary_observability"
PRIVACY_BOUNDARY = "redacted_local_aggregate"
REQUIRED_MAP_FIELDS = (
    "document_status_counts",
    "ingest_job_status_counts",
    "ingest_job_kind_status_counts",
    "ingest_job_failure_counts",
)
FORBIDDEN_FIELDS = (
    "resume_root",
    "data_dir",
    "out_dir",
    "raw_resume_text",
    "raw_query_text",
    "sample_ids",
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
        fail("evidence must be a JSON object")
    return document


def require_int(value: Any, field: str) -> int:
    if not isinstance(value, int):
        fail(f"{field} must be an integer")
    return value


def validate_full_evidence_observability(document: dict[str, Any]) -> None:
    observability = document.get(OBSERVABILITY_FIELD)
    if not isinstance(observability, dict):
        fail(f"missing {OBSERVABILITY_FIELD}")

    if observability.get("privacy_boundary") != PRIVACY_BOUNDARY:
        fail("invalid privacy boundary")

    document_count = require_int(observability.get("document_count"), "document_count")
    searchable_count = require_int(
        observability.get("searchable_document_count"),
        "searchable_document_count",
    )
    vector_count = require_int(
        observability.get("vector_indexed_document_count"),
        "vector_indexed_document_count",
    )

    if document_count < CURRENT_STAGE_DOCUMENT_FLOOR:
        fail("document_count below current-stage floor")
    if not 0 <= searchable_count <= document_count:
        fail("searchable_document_count is inconsistent")
    if not 0 <= vector_count <= searchable_count:
        fail("vector_indexed_document_count is inconsistent")
    if observability.get("hot_index_fully_covered") is not True:
        fail("hot index coverage must be true for full evidence")

    for field in REQUIRED_MAP_FIELDS:
        if not isinstance(observability.get(field), dict):
            fail(f"{field} must be an object")

    for field in FORBIDDEN_FIELDS:
        if field in observability:
            fail(f"forbidden observability field: {field}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate redacted current-stage corpus observability evidence."
    )
    parser.add_argument(
        "--full-evidence",
        type=Path,
        required=True,
        help="Path to current-stage-validation-evidence.json.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    validate_full_evidence_observability(read_json(args.full_evidence))
    return 0


if __name__ == "__main__":
    sys.exit(main())
