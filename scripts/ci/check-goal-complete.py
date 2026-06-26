#!/usr/bin/env python3
"""Validate the stricter evidence contract for goal_complete loop state."""

from __future__ import annotations

import json
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


def ref_exists(ref: str) -> bool:
    completed = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", ref],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return completed.returncode == 0


def main_ref() -> str:
    for candidate in ["origin/main", "main"]:
        if ref_exists(candidate):
            return candidate
    fail("goal_complete requires origin/main or main ref for reachability check")


def require_commit_reachable_from_main(commit: str, path: str, main: str) -> None:
    completed = subprocess.run(
        ["git", "merge-base", "--is-ancestor", commit, main],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if completed.returncode != 0:
        fail(f"{path}: commit is not reachable from {main}")


def main() -> int:
    state = load_json(ROOT / "perf" / "current-loop-state.json")
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    if not isinstance(state, dict):
        fail("perf/current-loop-state.json: expected object")
    if state.get("workflow_state") != "goal_complete":
        print(f"check-goal-complete.py passed (workflow_state={state.get('workflow_state')}, not goal_complete)")
        return 0

    if state.get("experiment_state") != "complete":
        fail("goal_complete requires experiment_state=complete")
    verification = state.get("verification")
    if not isinstance(verification, dict):
        fail("verification: expected object")
    if verification.get("claim") != "pass":
        fail("goal_complete requires verification.claim=pass")
    if verification.get("all_required_commands_ran") is not True:
        fail("goal_complete requires verification.all_required_commands_ran=true")
    commands = verification.get("commands")
    if not isinstance(commands, list) or not commands:
        fail("goal_complete requires non-empty verification.commands")

    github_ledger = state.get("github_ledger")
    if not isinstance(github_ledger, dict):
        fail("goal_complete requires github_ledger")
    if github_ledger.get("active_prs") != []:
        fail("goal_complete requires github_ledger.active_prs=[]")
    if github_ledger.get("open_blockers") != []:
        fail("goal_complete requires github_ledger.open_blockers=[]")

    completion = matrix.get("completion")
    if not isinstance(completion, dict):
        fail("completion: expected object")
    required_cells = completion.get("goal_complete_requires")
    if not isinstance(required_cells, list) or not required_cells:
        fail("completion.goal_complete_requires: expected non-empty list")

    evidence_cells = state.get("evidence_cells")
    if not isinstance(evidence_cells, list):
        fail("goal_complete requires evidence_cells list")
    cells_by_name: dict[str, dict] = {}
    for index, cell in enumerate(evidence_cells):
        if not isinstance(cell, dict):
            fail(f"evidence_cells[{index}]: expected object")
        name = cell.get("cell")
        if not isinstance(name, str) or not name:
            fail(f"evidence_cells[{index}].cell: expected non-empty string")
        cells_by_name[name] = cell

    missing = sorted(set(required_cells) - set(cells_by_name))
    if missing:
        fail(f"evidence_cells missing {missing}")
    main = main_ref()
    for cell_name in required_cells:
        commit = cells_by_name[cell_name].get("main_reachable_commit")
        if not isinstance(commit, str) or not commit or commit == "working-tree":
            fail(f"evidence_cells[{cell_name}].main_reachable_commit: expected main-reachable commit")
        require_commit_reachable_from_main(
            commit,
            f"evidence_cells[{cell_name}].main_reachable_commit",
            main,
        )

    print("check-goal-complete.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-goal-complete.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
