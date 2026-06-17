#!/usr/bin/env python3
"""Validate redacted current-stage local-safe fault-suite evidence."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


SCHEMA_VERSION = "fault-simulation-suite.v1"
SUITE = "local_safe"
PATHS_SENTINEL = "<redacted>"
EVIDENCE_LEVEL = "local_synthetic_fault_suite"
RELEASE_HARDWARE_DRILLS = "blocked"
REQUIRED_FAULTS = (
    "disk_space_low",
    "permission_denied",
    "file_lock",
    "index_snapshot_corrupt",
    "metadata_migration",
    "model_checksum",
    "daemon_kill",
    "ocr_crash",
    "battery_mode",
    "external_drive_disconnect",
)
ALLOWED_STATUSES = {"reproduced", "blocked_by_host"}
FORBIDDEN_TEXT_MARKERS = (
    "PRIVATE-current-stage",
    "PRIVATE_OCR_CRASH",
    "SYNTHETIC OCR CRASH PROBE BYTES",
    "SYNTHETIC MODEL CHECKSUM PROBE",
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
        fail("fault-suite evidence must be a JSON object")
    return document


def require_bool(value: Any, field: str) -> bool:
    if not isinstance(value, bool):
        fail(f"{field} must be a boolean")
    return value


def require_int(value: Any, field: str) -> int:
    if not isinstance(value, int):
        fail(f"{field} must be an integer")
    return value


def require_object(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    return value


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


def validate_case(case: Any, index: int) -> tuple[str, str]:
    case_path = f"cases[{index}]"
    case_obj = require_object(case, case_path)
    fault = case_obj.get("fault")
    if not isinstance(fault, str):
        fail(f"{case_path}.fault must be a string")
    status = case_obj.get("status")
    if not isinstance(status, str):
        fail(f"{case_path}.status must be a string")
    if status not in ALLOWED_STATUSES:
        fail(f"{case_path}.status is not allowed: {status}")
    if require_bool(case_obj.get("redacted"), f"{case_path}.redacted") is not True:
        fail(f"{case_path}.redacted must be true")
    if case_obj.get("paths") != PATHS_SENTINEL:
        fail(f"{case_path}.paths must be redacted")
    require_object(case_obj.get("details"), f"{case_path}.details")
    return fault, status


def require_reproduced_case(
    cases_by_fault: dict[str, dict[str, Any]],
    fault: str,
) -> dict[str, Any]:
    case = cases_by_fault.get(fault)
    if case is None:
        fail(f"missing required fault case: {fault}")
    if case.get("status") != "reproduced":
        fail(f"{fault} must be reproduced when current-stage supplies local fixtures")
    return case


def validate_local_safe_suite(document: dict[str, Any]) -> None:
    reject_forbidden_text(document)

    if document.get("schema_version") != SCHEMA_VERSION:
        fail("invalid fault-suite schema_version")
    if document.get("suite") != SUITE:
        fail("invalid fault-suite suite")
    if require_bool(document.get("redacted"), "redacted") is not True:
        fail("redacted must be true")
    if document.get("paths") != PATHS_SENTINEL:
        fail("paths must be redacted")
    if document.get("evidence_level") != EVIDENCE_LEVEL:
        fail("invalid evidence_level")
    if document.get("release_hardware_drills") != RELEASE_HARDWARE_DRILLS:
        fail("release_hardware_drills must remain blocked")

    summary = require_object(document.get("summary"), "summary")
    cases = document.get("cases")
    if not isinstance(cases, list):
        fail("cases must be an array")

    total_cases = require_int(summary.get("total_cases"), "summary.total_cases")
    reproduced_cases = require_int(
        summary.get("reproduced_cases"),
        "summary.reproduced_cases",
    )
    blocked_by_host_cases = require_int(
        summary.get("blocked_by_host_cases"),
        "summary.blocked_by_host_cases",
    )
    failed_cases = require_int(summary.get("failed_cases"), "summary.failed_cases")
    if require_bool(
        summary.get("release_blockers_cleared"),
        "summary.release_blockers_cleared",
    ):
        fail("local-safe fault suite must not clear release blockers")

    if total_cases != len(cases):
        fail("summary.total_cases does not match cases length")
    if total_cases < len(REQUIRED_FAULTS):
        fail("fault-suite is missing required local-safe coverage")
    if failed_cases != 0:
        fail("fault-suite has failed cases")

    cases_by_fault: dict[str, dict[str, Any]] = {}
    status_counts = {"reproduced": 0, "blocked_by_host": 0}
    for index, case in enumerate(cases):
        fault, status = validate_case(case, index)
        if fault in cases_by_fault:
            fail(f"duplicate fault case: {fault}")
        cases_by_fault[fault] = case
        status_counts[status] += 1

    if reproduced_cases != status_counts["reproduced"]:
        fail("summary.reproduced_cases does not match cases")
    if blocked_by_host_cases != status_counts["blocked_by_host"]:
        fail("summary.blocked_by_host_cases does not match cases")

    for fault in REQUIRED_FAULTS:
        if fault not in cases_by_fault:
            fail(f"missing required fault case: {fault}")

    daemon_case = require_reproduced_case(cases_by_fault, "daemon_kill")
    daemon_details = require_object(daemon_case.get("details"), "daemon_kill.details")
    if daemon_details.get("restart_check") != "passed":
        fail("daemon_kill restart_check must be passed")

    ocr_case = require_reproduced_case(cases_by_fault, "ocr_crash")
    ocr_details = require_object(ocr_case.get("details"), "ocr_crash.details")
    if ocr_details.get("ocr_command") != "failed":
        fail("ocr_crash ocr_command must be failed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate redacted current-stage local-safe fault-suite evidence."
    )
    parser.add_argument(
        "--local-safe-suite",
        type=Path,
        required=True,
        help="Path to fault-simulation-suite-local-safe.json.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    validate_local_safe_suite(read_json(args.local_safe_suite))
    return 0


if __name__ == "__main__":
    sys.exit(main())
