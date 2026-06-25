#!/usr/bin/env python3
"""Validate autonomous delivery permissions in ACTIVE_GOAL.toml."""

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


def require_bool(value: object, expected: bool, path: str) -> None:
    if value is not expected:
        fail(f"{path}: expected {expected}")


def require_string(value: object, expected: str, path: str) -> None:
    if value != expected:
        fail(f"{path}: expected {expected!r}")


def main() -> int:
    contracts = load_contracts_module()
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
    autonomous = active_goal.get("autonomous_delivery")
    if not isinstance(autonomous, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery]")

    permissions = autonomous.get("permissions")
    if not isinstance(permissions, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.permissions]")

    for key in [
        "production_code_allowed",
        "private_benchmark_allowed",
        "private_resume_root_read_allowed",
        "seektalent_artifacts_query_read_allowed",
        "github_issue_write_allowed",
        "github_pr_write_allowed",
        "commit_push_allowed",
        "auto_merge_allowed",
    ]:
        require_bool(permissions.get(key), True, f"autonomous_delivery.permissions.{key}")

    for key in [
        "direct_main_push_allowed",
        "admin_bypass_allowed",
        "raw_private_data_commit_allowed",
        "raw_query_commit_allowed",
        "gate_bypass_allowed",
        "threshold_relaxation_allowed",
    ]:
        require_bool(permissions.get(key), False, f"autonomous_delivery.permissions.{key}")

    current_pr = active_goal.get("scope", {}).get("current_pr", {})
    require_bool(current_pr.get("production_code_allowed"), False, "scope.current_pr.production_code_allowed")
    require_bool(current_pr.get("private_benchmark_allowed"), False, "scope.current_pr.private_benchmark_allowed")

    gui = active_goal.get("gui")
    if not isinstance(gui, dict):
        fail("ACTIVE_GOAL.toml: missing [gui]")
    require_string(gui.get("default_stack"), contracts.GUI_DEFAULT_STACK, "gui.default_stack")
    require_bool(gui.get("production_next_server_allowed"), False, "gui.production_next_server_allowed")
    require_bool(gui.get("toolkit_bakeoff_default_required"), False, "gui.toolkit_bakeoff_default_required")
    require_bool(gui.get("toolkit_bakeoff_requires_blocker_issue"), True, "gui.toolkit_bakeoff_requires_blocker_issue")
    require_string(gui.get("visual_reference_role"), contracts.GUI_REFERENCE_ROLE, "gui.visual_reference_role")
    require_string(gui.get("visual_reference"), "UI-reference/", "gui.visual_reference")

    platform_lanes = active_goal.get("platform_lanes")
    if not isinstance(platform_lanes, dict):
        fail("ACTIVE_GOAL.toml: missing [platform_lanes]")
    require_string(platform_lanes.get("primary_discovery"), contracts.PLATFORM_LANES[0], "platform_lanes.primary_discovery")
    require_string(platform_lanes.get("weak_host_validation"), contracts.PLATFORM_LANES[1], "platform_lanes.weak_host_validation")
    require_string(platform_lanes.get("ci_smoke"), contracts.PLATFORM_LANES[2], "platform_lanes.ci_smoke")
    require_bool(platform_lanes.get("macos_m4_can_close_windows_gate"), False, "platform_lanes.macos_m4_can_close_windows_gate")
    require_bool(
        platform_lanes.get("cross_os_ci_smoke_can_replace_weak_host_perf"),
        False,
        "platform_lanes.cross_os_ci_smoke_can_replace_weak_host_perf",
    )

    private_corpus_transfer = platform_lanes.get("private_corpus_transfer")
    if not isinstance(private_corpus_transfer, dict):
        fail("ACTIVE_GOAL.toml: missing [platform_lanes.private_corpus_transfer]")
    require_bool(
        private_corpus_transfer.get("runner_may_choose_transfer_to_windows"),
        True,
        "platform_lanes.private_corpus_transfer.runner_may_choose_transfer_to_windows",
    )
    require_bool(
        private_corpus_transfer.get("transfer_public_evidence_allowed"),
        False,
        "platform_lanes.private_corpus_transfer.transfer_public_evidence_allowed",
    )
    require_bool(
        private_corpus_transfer.get("raw_private_paths_public_allowed"),
        False,
        "platform_lanes.private_corpus_transfer.raw_private_paths_public_allowed",
    )
    require_string(
        private_corpus_transfer.get("public_source_name"),
        "$RESUME_IR_PRIVATE_RESUME_ROOT",
        "platform_lanes.private_corpus_transfer.public_source_name",
    )
    require_bool(
        private_corpus_transfer.get("windows_unavailable_starts_reconciliation"),
        True,
        "platform_lanes.private_corpus_transfer.windows_unavailable_starts_reconciliation",
    )

    activation = autonomous.get("activation")
    if not isinstance(activation, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.activation]")
    require_bool(activation.get("current_pr_contract_only"), True, "autonomous_delivery.activation.current_pr_contract_only")
    require_bool(
        activation.get("production_implementation_in_current_pr_allowed"),
        False,
        "autonomous_delivery.activation.production_implementation_in_current_pr_allowed",
    )
    require_bool(
        activation.get("private_benchmark_in_current_pr_allowed"),
        False,
        "autonomous_delivery.activation.private_benchmark_in_current_pr_allowed",
    )

    print("check-autonomous-goal.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-autonomous-goal.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
