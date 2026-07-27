#!/usr/bin/env python3
"""Public contract gate for writer-contract upgrade installed acceptance.

Validates matrix fields, runbook presence, and the redacted attestation receipt
schema fixture. Private exact-main DMG + v29 APFS/COW execution remains gated on
authorized roots and is never implied by a green public CI check.
"""

from __future__ import annotations

import json
import os
import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
MATRIX = ROOT / "perf" / "acceptance-matrix.toml"
RUNBOOK = ROOT / "docs" / "runbooks" / "writer-contract-upgrade-installed-acceptance.md"
ATTESTATION_FIXTURE = (
    ROOT
    / "perf"
    / "fixtures"
    / "writer-contract-upgrade"
    / "attestation-receipt.json"
)

REQUIRED_MATRIX_FIELDS = (
    "macos_installed_acceptance_pre_upgrade_canary",
    "macos_installed_acceptance_target_contract_attestation",
)

REQUIRED_RECEIPT_KEYS = (
    "schema_version",
    "privacy_boundary",
    "contains_raw_resume_text",
    "contains_queries",
    "contains_resume_paths",
    "contains_candidate_results",
    "phase",
    "schema_current",
    "writer_state",
    "opaque_transition_id",
    "target_contract_bound",
    "hard_cut_count",
    "task_purge_count",
    "target_commit_count",
)


def fail(message: str) -> None:
    print(f"writer-contract upgrade acceptance check failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def check_matrix() -> None:
    data = tomllib.loads(MATRIX.read_text())
    train = data.get("train_final") or data.get("current") or data
    # Walk nested tables for the required keys.
    flat: dict[str, object] = {}

    def walk(node: object, prefix: str = "") -> None:
        if isinstance(node, dict):
            for key, value in node.items():
                path = f"{prefix}.{key}" if prefix else str(key)
                if isinstance(value, dict):
                    walk(value, path)
                else:
                    flat[str(key)] = value
                    flat[path] = value

    walk(data)
    for field in REQUIRED_MATRIX_FIELDS:
        if field not in flat:
            fail(f"missing acceptance-matrix field {field}")


def check_runbook() -> None:
    if not RUNBOOK.is_file():
        fail(f"missing runbook {RUNBOOK.relative_to(ROOT)}")
    text = RUNBOOK.read_text()
    for needle in (
        "Pre-upgrade canary",
        "Target-contract attestation",
        "Restart idempotence",
        "schema current = 34",
        "opaque transition id",
    ):
        if needle not in text:
            fail(f"runbook missing required section text: {needle}")


def check_attestation_fixture() -> None:
    if not ATTESTATION_FIXTURE.is_file():
        fail(f"missing fixture {ATTESTATION_FIXTURE.relative_to(ROOT)}")
    receipt = json.loads(ATTESTATION_FIXTURE.read_text())
    for key in REQUIRED_RECEIPT_KEYS:
        if key not in receipt:
            fail(f"attestation fixture missing key {key}")
    for key in (
        "contains_raw_resume_text",
        "contains_queries",
        "contains_resume_paths",
        "contains_candidate_results",
    ):
        if receipt[key] is not False:
            fail(f"attestation fixture privacy field {key} must be false")
    if receipt["schema_version"] != "resume-ir.writer-contract-upgrade-attestation.v1":
        fail("unexpected attestation schema_version")
    if receipt["schema_current"] != 34:
        fail("attestation fixture schema_current must be 34")
    if receipt["hard_cut_count"] != 0 or receipt["task_purge_count"] != 0:
        fail("attestation fixture must record zero hard-cut/task-purge")
    opaque = receipt["opaque_transition_id"]
    if opaque is not None:
        if not isinstance(opaque, str) or not opaque.startswith("sha256:") or len(opaque) != 71:
            fail("opaque_transition_id must be a 71-char sha256 digest or null")


def private_gate_status() -> str:
    if os.environ.get("RESUME_IR_RUN_PRIVATE_WRITER_UPGRADE_ACCEPTANCE") != "1":
        return "blocked_private_root"
    root = os.environ.get("RESUME_IR_INSTALLED_ACCEPTANCE_V29_ROOT")
    if not root:
        return "blocked_private_root"
    path = pathlib.Path(root)
    if not path.is_dir():
        fail("RESUME_IR_INSTALLED_ACCEPTANCE_V29_ROOT is not a directory")
    # Private execution is operator-driven via the macOS installed acceptance
    # harness; this gate only confirms authorization surface is present.
    return "private_root_authorized"


def main() -> int:
    check_matrix()
    check_runbook()
    check_attestation_fixture()
    status = private_gate_status()
    print(
        "writer-contract upgrade acceptance public contract check passed "
        f"({status})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
