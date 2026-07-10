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


def require_issue_ref(value: object, path: str) -> str:
    if not isinstance(value, str) or not value.startswith("#") or not value[1:].isdigit():
        fail(f"{path}: expected issue ref like '#123'")
    return value


def validate_github_ledger(state: dict) -> None:
    github_ledger = require_mapping(state.get("github_ledger"), "perf/current-loop-state.json.github_ledger")
    primary_issue = require_issue_ref(
        github_ledger.get("primary_issue"),
        "perf/current-loop-state.json.github_ledger.primary_issue",
    )
    active_prs = github_ledger.get("active_prs")
    if not isinstance(active_prs, list):
        fail("perf/current-loop-state.json.github_ledger.active_prs: expected list")
    for index, ref in enumerate(active_prs):
        require_issue_ref(ref, f"perf/current-loop-state.json.github_ledger.active_prs[{index}]")
    open_blockers = github_ledger.get("open_blockers")
    if not isinstance(open_blockers, list):
        fail("perf/current-loop-state.json.github_ledger.open_blockers: expected list")
    for index, ref in enumerate(open_blockers):
        require_issue_ref(ref, f"perf/current-loop-state.json.github_ledger.open_blockers[{index}]")

    if state.get("workflow_state") == "blocked_permission":
        if active_prs != []:
            fail("perf/current-loop-state.json.github_ledger.active_prs: expected [] while workflow_state=blocked_permission")
        if not open_blockers:
            fail("perf/current-loop-state.json.github_ledger.open_blockers: expected non-empty list while workflow_state=blocked_permission")
        if primary_issue not in open_blockers:
            fail("perf/current-loop-state.json.github_ledger.primary_issue: expected to appear in open_blockers while workflow_state=blocked_permission")


def validate_synthetic_smoke_snapshot(state: dict, matrix: dict) -> None:
    if state.get("workflow_state") != "baseline_captured":
        return
    if state.get("experiment_state") != "baseline_validated" or state.get("evidence_lane") != "smoke":
        return

    smoke_baseline = require_mapping(
        matrix.get("synthetic_smoke_baseline"),
        "perf/acceptance-matrix.toml.synthetic_smoke_baseline",
    )
    required_commands = smoke_baseline.get("required_commands")
    if not isinstance(required_commands, list) or not required_commands:
        fail("perf/acceptance-matrix.toml.synthetic_smoke_baseline.required_commands: expected non-empty list")

    verification = require_mapping(state.get("verification"), "perf/current-loop-state.json.verification")
    commands = verification.get("commands")
    if not isinstance(commands, list):
        fail("perf/current-loop-state.json.verification.commands: expected list")
    observed: dict[str, int] = {}
    for index, command in enumerate(commands):
        entry = require_mapping(command, f"perf/current-loop-state.json.verification.commands[{index}]")
        command_text = entry.get("command")
        if not isinstance(command_text, str) or not command_text:
            fail(f"perf/current-loop-state.json.verification.commands[{index}].command: expected non-empty string")
        exit_code = entry.get("exit_code")
        if not isinstance(exit_code, int) or isinstance(exit_code, bool):
            fail(f"perf/current-loop-state.json.verification.commands[{index}].exit_code: expected integer")
        observed[command_text] = exit_code

    missing = sorted(set(required_commands) - set(observed))
    if missing:
        fail(f"perf/current-loop-state.json.verification.commands missing synthetic smoke required commands: {missing}")
    failed = sorted(command for command in required_commands if observed.get(command) != 0)
    if failed:
        fail(f"perf/current-loop-state.json.verification.commands non-zero synthetic smoke commands: {failed}")
    if verification.get("claim") != "partial":
        fail("perf/current-loop-state.json.verification.claim: baseline_captured smoke snapshot must remain partial")
    require_bool(
        verification.get("all_required_commands_ran"),
        True,
        "perf/current-loop-state.json.verification.all_required_commands_ran",
    )


def main() -> int:
    state = load_json(ROOT / "perf" / "current-loop-state.json")
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
    contracts = load_contracts_module()
    contracts.validate_loop_state(state, matrix, "perf/current-loop-state.json")
    contracts.validate_current_loop_contract_pins(state)

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

    validate_github_ledger(state)

    active_slice = require_mapping(
        active_goal.get("scope", {}).get("active_slice"),
        "ACTIVE_GOAL.toml.scope.active_slice",
    )
    active_issue = require_issue_ref(
        active_slice.get("issue"),
        "ACTIVE_GOAL.toml.scope.active_slice.issue",
    )
    github_ledger = require_mapping(
        state.get("github_ledger"),
        "perf/current-loop-state.json.github_ledger",
    )
    require_string(
        github_ledger.get("primary_issue"),
        active_issue,
        "perf/current-loop-state.json.github_ledger.primary_issue",
    )
    current_slice = state.get("current_slice")
    if not isinstance(current_slice, str) or not current_slice.startswith(f"{active_issue} "):
        fail(
            "perf/current-loop-state.json.current_slice: expected prefix "
            f"{active_issue!r}"
        )
    if active_issue == "#145":
        for key, expected in {
            "workflow_state": "pr_opened",
            "experiment_state": "contract_locked",
            "evidence_lane": "w0_docs",
            "current_slice": "#145 atomic bootstrap parse-result cancel-poll recovery",
        }.items():
            require_string(state.get(key), expected, f"perf/current-loop-state.json.{key}")
        active_prs = github_ledger.get("active_prs")
        if active_prs != ["#142", "#144"]:
            fail(
                "perf/current-loop-state.json.github_ledger.active_prs: "
                "#145 bootstrap snapshot must keep open PRs #142 and #144"
            )
        open_blockers = github_ledger.get("open_blockers")
        if open_blockers != ["#37", "#140", "#143", "#145"]:
            fail(
                "perf/current-loop-state.json.github_ledger.open_blockers: "
                "#145 bootstrap snapshot must equal #37/#140/#143/#145"
            )
        transition = {
            "from": "contract_conflict",
            "to": "pr_opened",
            "evidence_ref": "https://github.com/FrankQDWang/resume-ir/issues/145#issuecomment-4932578248",
        }
        if transition not in state.get("transition_history", []):
            fail("perf/current-loop-state.json.transition_history: missing atomic bootstrap intent transition")

    active_loop = active_goal.get("loop")
    if not isinstance(active_loop, dict):
        fail("ACTIVE_GOAL.toml: missing [loop]")
    for stale_key in ("workflow_state", "experiment_state"):
        if stale_key in active_loop:
            fail(
                f"ACTIVE_GOAL.toml.loop.{stale_key}: current state belongs in "
                "perf/current-loop-state.json, not ACTIVE_GOAL.toml"
            )

    validate_synthetic_smoke_snapshot(state, matrix)

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
