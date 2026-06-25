#!/usr/bin/env python3
"""Validate the focused current loop-state contract."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[2]


def fail(message: str) -> None:
    raise ValueError(message)


def load_json(path: pathlib.Path) -> object:
    with path.open("rb") as fh:
        return json.load(fh)


def load_toml(path: pathlib.Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def load_contracts_module():
    path = ROOT / "scripts" / "ci" / "check-performance-contracts.py"
    spec = importlib.util.spec_from_file_location("performance_contracts", path)
    if spec is None or spec.loader is None:
        fail(f"{path}: unable to load aggregate contract module")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> int:
    state = load_json(ROOT / "perf" / "current-loop-state.json")
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    contracts = load_contracts_module()
    contracts.validate_loop_state(state, matrix, "perf/current-loop-state.json")

    if not isinstance(state, dict):
        fail("perf/current-loop-state.json: expected object")
    if state.get("schema_version") != "resume-ir.loop-state-report.v2":
        fail("schema_version mismatch")
    if state.get("goal_id") != "resume-ir.performance-gui-loop.2026-06":
        fail("goal_id mismatch")

    privacy = state.get("privacy")
    if not isinstance(privacy, dict):
        fail("privacy: expected object")
    for field in contracts.PRIVACY_FALSE_FIELDS:
        if privacy.get(field) is not False:
            fail(f"privacy.{field}: expected false")

    allowed_paths = state.get("allowed_paths")
    if not isinstance(allowed_paths, list) or not allowed_paths:
        fail("allowed_paths: expected non-empty list")

    verification = state.get("verification")
    if not isinstance(verification, dict):
        fail("verification: expected object")
    if verification.get("claim") not in {"pass", "fail", "blocked", "partial"}:
        fail("verification.claim invalid")

    experiment_state = state.get("experiment_state")
    if experiment_state in {"hypothesis_registered", "accepted", "reverted", "complete"}:
        hypothesis = state.get("hypothesis")
        if not isinstance(hypothesis, dict):
            fail("hypothesis: expected object")
        for key in ["id", "acceptance_cell", "expected_effect", "before_measurement_ref"]:
            if not hypothesis.get(key):
                fail(f"hypothesis.{key}: required")
        if experiment_state in {"accepted", "reverted", "complete"}:
            for key in ["after_measurement_ref", "reprofile_ref", "decision"]:
                if not hypothesis.get(key):
                    fail(f"hypothesis.{key}: required")

    print("check-loop-state.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-loop-state.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
