#!/usr/bin/env python3
"""Build a redacted current-stage handoff summary from local validation output."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


HANDOFF_SCHEMA = "resume-ir.current-stage-handoff.v1"
HANDOFF_PRIVACY_BOUNDARY = "local_only_redacted_handoff"
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
PRIVATE_MARKER = re.compile(r"PRIVATE-|/Users/|/home/|[A-Za-z]:\\")


def fail(message: str) -> None:
    print(f"current-stage handoff blocked: {message}", file=sys.stderr)
    raise SystemExit(2)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize redacted current-stage validation evidence."
    )
    parser.add_argument("--input", required=True, help="redacted summary/evidence JSON")
    parser.add_argument("--out", required=True, help="handoff JSON output path")
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


def observability(document: dict[str, Any]) -> dict[str, Any]:
    value = document.get("corpus_summary_observability", {})
    if value is None:
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
    }
    return {key: value[key] for key in sorted(allowed) if key in value}


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

    return {
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
        "observability": observability(document),
        "completed_steps": completed_steps(document),
        "blocked_or_not_complete": not_complete_items(document),
        "must_not_upload": must_not_upload(document),
    }


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
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
