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
REQUIRED_FALSE_FIELDS = (
    "contains_raw_resume_text",
    "contains_resume_paths",
    "contains_queries",
    "contains_sample_ids",
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


def require_false(value: Any, field: str) -> None:
    if value is not False:
        fail(f"{field} must be false")


def validate_observability(
    document: dict[str, Any],
    require_hot_index: bool,
    min_documents: int,
    field: str,
) -> None:
    observability = document.get(field)
    if not isinstance(observability, dict):
        fail(f"missing {field}")

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

    if document_count < min_documents:
        fail("document_count below current-stage floor")
    if not 0 <= searchable_count <= document_count:
        fail("searchable_document_count is inconsistent")
    if not 0 <= vector_count <= searchable_count:
        fail("vector_indexed_document_count is inconsistent")
    hot_index_fully_covered = observability.get("hot_index_fully_covered")
    if not isinstance(hot_index_fully_covered, bool):
        fail("hot_index_fully_covered must be a boolean")
    if require_hot_index and hot_index_fully_covered is not True:
        fail("hot index coverage must be true for full evidence")

    for field in REQUIRED_MAP_FIELDS:
        if not isinstance(observability.get(field), dict):
            fail(f"{field} must be an object")

    for field in REQUIRED_FALSE_FIELDS:
        require_false(observability.get(field), field)

    for field in FORBIDDEN_FIELDS:
        if field in observability:
            fail(f"forbidden observability field: {field}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate redacted current-stage corpus observability evidence."
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--full-evidence",
        type=Path,
        help="Path to current-stage-validation-evidence.json.",
    )
    mode.add_argument(
        "--summary",
        type=Path,
        help="Path to a current-stage smoke or blocked summary JSON.",
    )
    parser.add_argument(
        "--min-documents",
        type=int,
        default=CURRENT_STAGE_DOCUMENT_FLOOR,
        help="Minimum document_count required for the observability payload.",
    )
    parser.add_argument(
        "--field",
        default=OBSERVABILITY_FIELD,
        help="Top-level field containing the observability payload.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.min_documents < 1:
        fail("min-documents must be positive")
    if args.full_evidence is not None:
        validate_observability(
            read_json(args.full_evidence),
            require_hot_index=True,
            min_documents=args.min_documents,
            field=args.field,
        )
    else:
        validate_observability(
            read_json(args.summary),
            require_hot_index=False,
            min_documents=args.min_documents,
            field=args.field,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
