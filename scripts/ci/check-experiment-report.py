#!/usr/bin/env python3
"""Validate experiment report fixtures independently from loop-state fixtures."""

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


def is_experiment_report(value: object) -> bool:
    return isinstance(value, dict) and value.get("schema_version") == "resume-ir.experiment-report.v2"


def validate_no_goal_complete_claim(value: object, path: pathlib.Path) -> None:
    if isinstance(value, dict) and value.get("claim") == "goal_complete":
        fail(f"{path.relative_to(ROOT)}: experiment report must not claim goal_complete")


def validate_w1_contract_fields(value: object, contracts: object, path: pathlib.Path) -> None:
    if not isinstance(value, dict) or value.get("evidence_lane") != "w1_private":
        return
    rel = str(path.relative_to(ROOT))
    contracts.validate_optimization(value.get("optimization"), f"{rel}.optimization")
    contracts.validate_workload_manifest(value.get("workload_manifest"), f"{rel}.workload_manifest")
    contracts.validate_platform_evidence(value.get("platform_evidence"), f"{rel}.platform_evidence")


def main() -> int:
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    contracts = load_contracts_module()
    valid_count = 0
    invalid_count = 0

    for path in sorted((ROOT / "perf" / "fixtures" / "valid").glob("*.json")):
        value = load_json(path)
        if not is_experiment_report(value):
            continue
        valid_count += 1
        validate_no_goal_complete_claim(value, path)
        validate_w1_contract_fields(value, contracts, path)
        contracts.validate_experiment_report(value, matrix, str(path.relative_to(ROOT)))

    for path in sorted((ROOT / "perf" / "fixtures" / "invalid").glob("*.json")):
        value = load_json(path)
        if not is_experiment_report(value):
            continue
        invalid_count += 1
        try:
            validate_no_goal_complete_claim(value, path)
            validate_w1_contract_fields(value, contracts, path)
            contracts.validate_experiment_report(value, matrix, str(path.relative_to(ROOT)))
        except ValueError:
            continue
        fail(f"{path.relative_to(ROOT)}: invalid experiment fixture unexpectedly passed")

    if valid_count == 0:
        fail("no valid experiment report fixtures found")
    if invalid_count == 0:
        fail("no invalid experiment report fixtures found")

    print("check-experiment-report.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-experiment-report.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
