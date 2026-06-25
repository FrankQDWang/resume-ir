#!/usr/bin/env python3
"""Validate autonomous delivery PR budget and template anchors."""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
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


def git(args: list[str]) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        fail(f"git {' '.join(args)} failed: {completed.stderr.strip()}")
    return completed.stdout.strip()


def ref_exists(ref: str) -> bool:
    completed = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", ref],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return completed.returncode == 0


def fetch_base_ref(base: str) -> None:
    subprocess.run(
        ["git", "fetch", "--no-tags", "--depth=1", "origin", f"{base}:refs/remotes/origin/{base}"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )


def select_base_ref() -> str:
    base = os.environ.get("GITHUB_BASE_REF") or "main"
    candidates = [f"origin/{base}", base, "origin/main", "main"]
    for candidate in candidates:
        if ref_exists(candidate):
            return candidate
    if os.environ.get("GITHUB_ACTIONS") == "true":
        fetch_base_ref(base)
        for candidate in candidates:
            if ref_exists(candidate):
                return candidate
    fail("unable to find base ref for PR budget check")


def actual_pr_budget(base_ref: str) -> dict[str, int]:
    merge_base = git(["merge-base", base_ref, "HEAD"])
    commits = int(git(["rev-list", "--count", f"{merge_base}..HEAD"]) or "0")
    changed_files_raw = git(["diff", "--name-only", f"{merge_base}...HEAD"])
    changed_files = 0 if not changed_files_raw else len(changed_files_raw.splitlines())
    additions = 0
    deletions = 0
    numstat = git(["diff", "--numstat", f"{merge_base}...HEAD"])
    for line in numstat.splitlines():
        fields = line.split("\t")
        if len(fields) < 3:
            continue
        added, deleted = fields[0], fields[1]
        if added != "-":
            additions += int(added)
        if deleted != "-":
            deletions += int(deleted)
    return {
        "commits": commits,
        "changed_files": changed_files,
        "net_lines": additions + deletions,
    }


def validate_actual_budget(active_goal: dict, pr_budget: dict) -> None:
    scope_current_pr = active_goal.get("scope", {}).get("current_pr", {})
    scope_exception = scope_current_pr.get("scope_exception") is True
    if scope_exception:
        if scope_current_pr.get("scope_exception_auto_merge_allowed") is not False:
            fail("scope.current_pr.scope_exception_auto_merge_allowed: expected false")
        if not scope_current_pr.get("scope_exception_reason"):
            fail("scope.current_pr.scope_exception_reason: required for budget exception")
    base_ref = select_base_ref()
    actual = actual_pr_budget(base_ref)
    violations = []
    if actual["commits"] > pr_budget["max_commits"]:
        violations.append(f"commits {actual['commits']} > {pr_budget['max_commits']}")
    if actual["changed_files"] > pr_budget["max_changed_files"]:
        violations.append(f"changed_files {actual['changed_files']} > {pr_budget['max_changed_files']}")
    if actual["net_lines"] > pr_budget["max_net_lines"]:
        violations.append(f"net_lines {actual['net_lines']} > {pr_budget['max_net_lines']}")
    if violations and not scope_exception:
        fail("PR budget exceeded without scope exception: " + ", ".join(violations))
    if violations and pr_budget.get("allow_scope_exception_auto_merge") is not False:
        fail("scope exception PRs must not auto-merge")


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

    validate_actual_budget(active_goal, pr_budget)

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
