#!/usr/bin/env python3
"""Validate experiment report fixtures independently from loop-state fixtures."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys
import tomllib
from collections.abc import Mapping


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


def is_synthetic_smoke_manifest(value: object) -> bool:
    return isinstance(value, dict) and value.get("schema_version") == "resume-ir.synthetic-smoke-artifact-manifest.v1"


def relative_path(path: pathlib.Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def validate_report_or_manifest(
    value: object,
    contracts: object,
    matrix: dict,
    path: pathlib.Path,
) -> None:
    if is_experiment_report(value):
        contracts.validate_experiment_report(value, matrix, relative_path(path))
    elif is_synthetic_smoke_manifest(value):
        contracts.validate_synthetic_smoke_artifact_manifest(value, relative_path(path))
    else:
        fail(f"{relative_path(path)}: unsupported report schema_version")


def validate_pairs(paths_and_values: list[tuple[pathlib.Path, object]], contracts: object, matrix: dict) -> int:
    reports: list[tuple[pathlib.Path, Mapping[str, object]]] = []
    manifests: list[tuple[pathlib.Path, Mapping[str, object]]] = []
    for path, value in paths_and_values:
        if is_experiment_report(value):
            if not isinstance(value, Mapping):
                fail(f"{relative_path(path)}: expected object")
            reports.append((path, value))
        elif is_synthetic_smoke_manifest(value):
            if not isinstance(value, Mapping):
                fail(f"{relative_path(path)}: expected object")
            manifests.append((path, value))
    for manifest_path, manifest in manifests:
        matches = []
        for report_path, report in reports:
            if manifest.get("report_sha256") == contracts.sha256_file(report_path):
                matches.append((report_path, report))
        if len(matches) != 1:
            fail(f"{relative_path(manifest_path)}: expected exactly one matching report")
        report_path, report = matches[0]
        contracts.validate_synthetic_smoke_report_manifest_pair(report, manifest, report_path, manifest_path, matrix)
    for report_path, report in reports:
        if report.get("evidence_lane") != "smoke" or "synthetic_smoke" not in report:
            continue
        matches = [
            manifest_path
            for manifest_path, manifest in manifests
            if manifest.get("report_sha256") == contracts.sha256_file(report_path)
        ]
        if len(matches) != 1:
            fail(f"{relative_path(report_path)}: synthetic smoke report requires exactly one matching manifest")
    return len(reports) + len(manifests)


def validate_explicit_paths(paths: list[pathlib.Path]) -> int:
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    contracts = load_contracts_module()
    paths_and_values = []
    for path in [path.resolve() for path in paths]:
        value = load_json(path)
        validate_report_or_manifest(value, contracts, matrix, path)
        paths_and_values.append((path, value))
    return validate_pairs(paths_and_values, contracts, matrix)


def main() -> int:
    if len(sys.argv) > 1:
        count = validate_explicit_paths([pathlib.Path(arg) for arg in sys.argv[1:]])
        print(f"check-experiment-report.py passed ({count} explicit report/manifest file(s))")
        return 0

    contracts = load_contracts_module()
    contracts.main()
    print("check-experiment-report.py passed (fixture sweep delegated to check-performance-contracts.py)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-experiment-report.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
