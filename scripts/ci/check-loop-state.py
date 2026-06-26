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


def require_mapping(value: object, path: str) -> dict:
    if not isinstance(value, dict):
        fail(f"{path}: expected object")
    return value


def require_bool(value: object, expected: bool, path: str) -> None:
    if value is not expected:
        fail(f"{path}: expected {expected}")


def require_string(value: object, expected: str, path: str) -> None:
    if value != expected:
        fail(f"{path}: expected {expected!r}")


def validate_runner_contracts(state: dict, active_goal: dict) -> None:
    autonomous = require_mapping(active_goal.get("autonomous_delivery"), "ACTIVE_GOAL.toml.autonomous_delivery")

    for key in ["goal_prompt", "event_log", "runtime_capability_attestation"]:
        expected = require_mapping(autonomous.get(key), f"ACTIVE_GOAL.toml.autonomous_delivery.{key}")
        observed = require_mapping(state.get(key), f"perf/current-loop-state.json.{key}")
        if observed != expected:
            fail(f"perf/current-loop-state.json.{key}: must match ACTIVE_GOAL.toml autonomous_delivery.{key}")

    recovery = require_mapping(state.get("runner_recovery"), "perf/current-loop-state.json.runner_recovery")
    for key in [
        "lease_required",
        "heartbeat_required",
        "cas_required",
        "idempotency_key_required",
        "intent_before_side_effect_required",
        "verify_after_side_effect_required",
        "one_transition_per_wake",
        "capability_attestation_required",
    ]:
        require_bool(recovery.get(key), True, f"perf/current-loop-state.json.runner_recovery.{key}")

    github_ledger = require_mapping(state.get("github_ledger"), "perf/current-loop-state.json.github_ledger")
    require_string(github_ledger.get("primary_issue"), "#10", "perf/current-loop-state.json.github_ledger.primary_issue")
    active_prs = github_ledger.get("active_prs")
    if active_prs != ["#10"]:
        fail("perf/current-loop-state.json.github_ledger.active_prs: expected ['#10']")
    open_blockers = github_ledger.get("open_blockers")
    if open_blockers != []:
        fail("perf/current-loop-state.json.github_ledger.open_blockers: expected empty list")


def main() -> int:
    state = load_json(ROOT / "perf" / "current-loop-state.json")
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
    contracts = load_contracts_module()
    contracts.validate_loop_state(state, matrix, "perf/current-loop-state.json")
    contracts.validate_current_loop_contract_pins(state)

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

    platform_lane = state.get("platform_lane")
    if platform_lane is not None and platform_lane not in contracts.PLATFORM_LANES:
        fail("platform_lane invalid")

    visual_reference = state.get("visual_reference")
    if visual_reference is not None:
        if not isinstance(visual_reference, dict):
            fail("visual_reference: expected object")
        if visual_reference.get("reference_role") != contracts.GUI_REFERENCE_ROLE:
            fail("visual_reference.reference_role mismatch")
        if visual_reference.get("default_stack") != contracts.GUI_DEFAULT_STACK:
            fail("visual_reference.default_stack mismatch")
        if visual_reference.get("production_next_server_allowed") is not False:
            fail("visual_reference.production_next_server_allowed: expected false")

    validate_runner_contracts(state, active_goal)

    experiment_state = state.get("experiment_state")
    if experiment_state in {"hypothesis_registered", "accepted", "reverted", "complete"}:
        hypothesis = state.get("hypothesis")
        if not isinstance(hypothesis, dict):
            fail("hypothesis: expected object")
        optimization_layer = hypothesis.get("optimization_layer")
        if optimization_layer is not None and optimization_layer not in contracts.OPTIMIZATION_LAYERS:
            fail("hypothesis.optimization_layer invalid")
        lower_layer_closure = hypothesis.get("lower_layer_closure")
        if lower_layer_closure is not None:
            if not isinstance(lower_layer_closure, dict):
                fail("hypothesis.lower_layer_closure: expected object")
            if lower_layer_closure.get("lower_layer_closes_higher_layer_blocker") is not False:
                fail("hypothesis.lower_layer_closure.lower_layer_closes_higher_layer_blocker: expected false")
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
