#!/usr/bin/env python3
"""Validate autonomous delivery PR budget and template anchors."""

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


def require_value(value: object, expected: object, path: str) -> None:
    if value != expected:
        fail(f"{path}: expected {expected!r}, got {value!r}")


def main() -> int:
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
    pr_budget = active_goal.get("autonomous_delivery", {}).get("pr_budget")
    if not isinstance(pr_budget, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.pr_budget]")

    expected = {
        "max_commits": 5,
        "max_changed_files": 15,
        "max_net_lines": 800,
        "max_issue_count": 2,
        "require_single_primary_lane": True,
        "require_single_primary_hypothesis": True,
        "allow_scope_exception_auto_merge": False,
    }
    for key, value in expected.items():
        require_value(pr_budget.get(key), value, f"autonomous_delivery.pr_budget.{key}")

    template = (ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text(encoding="utf-8")
    for anchor in [
        "contract:scope",
        "contract:linked_issue",
        "contract:hypothesis_baseline",
        "contract:changes",
        "contract:out_of_scope",
        "contract:verification",
        "contract:performance_evidence",
        "contract:privacy_boundary",
        "contract:rollback_plan",
        "contract:merge_readiness",
        "contract:scope_exception",
    ]:
        if anchor not in template:
            fail(f".github/PULL_REQUEST_TEMPLATE.md: missing {anchor}")

    print("check-pr-budget.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-pr-budget.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
