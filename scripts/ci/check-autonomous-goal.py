#!/usr/bin/env python3
"""Validate autonomous delivery permissions in ACTIVE_GOAL.toml."""

from __future__ import annotations

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


def require_bool(value: object, expected: bool, path: str) -> None:
    if value is not expected:
        fail(f"{path}: expected {expected}")


def main() -> int:
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
