#!/usr/bin/env python3
"""Validate autonomous benchmark lane contracts."""

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


REQUIRED_LANES = {
    "first_searchable",
    "full_import_ocr_backlog",
    "query_hot_path",
    "agent_query_replay",
    "repeat_amplification_control",
}

REQUIRED_COMPLETION_CELLS = REQUIRED_LANES | {
    "w0_docs",
    "d10k_private_calibration",
    "d100k_weak_host",
    "d1m_scale",
    "soak_fault",
    "gui_manual",
}


def lower_layer_closure_paths(value: object, path: str = "") -> list[str]:
    matches: list[str] = []
    if isinstance(value, dict):
        for key, nested in value.items():
            nested_path = f"{path}.{key}" if path else key
            if key == "lower_layer_closes_higher_layer_blocker" and nested is True:
                matches.append(nested_path)
            matches.extend(lower_layer_closure_paths(nested, nested_path))
    elif isinstance(value, list):
        for index, nested in enumerate(value):
            matches.extend(lower_layer_closure_paths(nested, f"{path}[{index}]"))
    return matches


def check_no_lower_layer_closure_misuse() -> None:
    paths = sorted((ROOT / "perf").glob("*.json"))
    paths.extend(sorted((ROOT / "perf" / "fixtures" / "valid").glob("*.json")))
    for path in paths:
        value = load_json(path)
        matches = lower_layer_closure_paths(value)
        if matches:
            rel = path.relative_to(ROOT)
            fail(f"{rel}: lower-layer closure misuse is forbidden at {', '.join(matches)}")


def main() -> int:
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    lanes = matrix.get("autonomous_delivery_lanes")
    if not isinstance(lanes, dict):
        fail("perf/acceptance-matrix.toml: missing [autonomous_delivery_lanes]")
    lane_names = set(lanes)
    if lane_names != REQUIRED_LANES:
        fail(f"autonomous_delivery_lanes: expected {sorted(REQUIRED_LANES)}, got {sorted(lane_names)}")

    completion = matrix.get("completion")
    if not isinstance(completion, dict):
        fail("perf/acceptance-matrix.toml: missing [completion]")
    cells = completion.get("goal_complete_requires")
    if not isinstance(cells, list):
        fail("completion.goal_complete_requires: expected list")
    missing_cells = REQUIRED_COMPLETION_CELLS - set(cells)
    if missing_cells:
        fail(f"completion.goal_complete_requires missing {sorted(missing_cells)}")
    if completion.get("task6_guard_must_use_goal_complete_requires") is not True:
        fail("completion.task6_guard_must_use_goal_complete_requires: expected true")

    required_lanes = completion.get("required_autonomous_delivery_lanes")
    if set(required_lanes or []) != REQUIRED_LANES:
        fail("completion.required_autonomous_delivery_lanes mismatch")

    cannot_claim = lanes["agent_query_replay"].get("cannot_claim")
    if not isinstance(cannot_claim, list):
        fail("autonomous_delivery_lanes.agent_query_replay.cannot_claim: expected list")
    for token in ["d1m_real_distribution_quality", "ocr_completion_performance"]:
        if token not in cannot_claim:
            fail(f"agent_query_replay.cannot_claim missing {token}")
    if "D1M_real_distribution_quality" in cannot_claim:
        fail("agent_query_replay.cannot_claim uses non-canonical D1M_real_distribution_quality")

    check_no_lower_layer_closure_misuse()

    print("check-benchmark-lanes.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-benchmark-lanes.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
