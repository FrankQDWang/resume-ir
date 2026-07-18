#!/usr/bin/env python3
"""Enforce the single-owner boundary for production search snapshot reads."""

from __future__ import annotations

import argparse
import copy
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
from collections.abc import Mapping, Sequence


ROOT = pathlib.Path(__file__).resolve().parents[2]
FACADE_PACKAGE = "search-runtime"
PRODUCTION_CONSUMERS = ("resume-daemon", "resume-cli")
FORBIDDEN_DIRECT_DEPENDENCIES = {"index-fulltext", "index-vector"}
PRODUCTION_SOURCE_ROOTS = (
    pathlib.Path("crates/daemon/src"),
    pathlib.Path("crates/cli/src"),
)
FORBIDDEN_SOURCE_PATTERNS = (
    ("SnapshotReadLease", re.compile(r"\bSnapshotReadLease\b")),
    ("FullTextIndex::open_*", re.compile(r"\bFullTextIndex\s*::\s*open_[A-Za-z0-9_]*\b")),
    ("VectorSnapshotRoot", re.compile(r"\bVectorSnapshotRoot\b")),
    ("VectorSnapshotReader", re.compile(r"\bVectorSnapshotReader\b")),
    ("PersistentVectorSearchIndex", re.compile(r"\bPersistentVectorSearchIndex\b")),
    ("PersistentVectorIndex", re.compile(r"\bPersistentVectorIndex\b")),
    ("with_search_metadata_snapshot", re.compile(r"\bwith_search_metadata_snapshot\b")),
    ("validated_active_projections", re.compile(r"\bvalidated_active_projections\b")),
    (
        "latest_visible_resume_version_for_document",
        re.compile(r"\blatest_visible_resume_version_for_document\b"),
    ),
)
FORBIDDEN_SOURCE_FIXTURES = (
    ("SnapshotReadLease", "fn bypass() { let _ = SnapshotReadLease; }"),
    (
        "FullTextIndex::open_*",
        'fn bypass() { let _ = FullTextIndex::open_snapshot("root", "generation"); }',
    ),
    ("VectorSnapshotRoot", "fn bypass() { let _ = VectorSnapshotRoot; }"),
    ("VectorSnapshotReader", "fn bypass() { let _ = VectorSnapshotReader; }"),
    (
        "PersistentVectorSearchIndex",
        "fn bypass() { let _ = PersistentVectorSearchIndex; }",
    ),
    ("PersistentVectorIndex", "fn bypass() { let _ = PersistentVectorIndex; }"),
    (
        "with_search_metadata_snapshot",
        "fn bypass(store: &Store) { store.with_search_metadata_snapshot(|_| ()); }",
    ),
    (
        "validated_active_projections",
        "fn bypass(store: &Store) { store.validated_active_projections(); }",
    ),
    (
        "latest_visible_resume_version_for_document",
        "fn bypass(store: &Store) { store.latest_visible_resume_version_for_document(); }",
    ),
)


class BoundaryViolation(ValueError):
    """One or more search-runtime ownership rules were violated."""


def require_mapping(value: object, context: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise BoundaryViolation(f"{context}: expected object")
    return value


def require_sequence(value: object, context: str) -> Sequence[object]:
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes, bytearray)):
        raise BoundaryViolation(f"{context}: expected array")
    return value


def workspace_packages(metadata: Mapping[str, object]) -> dict[str, Mapping[str, object]]:
    packages = require_sequence(metadata.get("packages"), "cargo metadata.packages")
    workspace_member_ids = metadata.get("workspace_members")
    members = (
        set(require_sequence(workspace_member_ids, "cargo metadata.workspace_members"))
        if workspace_member_ids is not None
        else None
    )
    by_name: dict[str, Mapping[str, object]] = {}
    for index, raw_package in enumerate(packages):
        package = require_mapping(raw_package, f"cargo metadata.packages[{index}]")
        package_id = package.get("id")
        if members is not None and package_id not in members:
            continue
        name = package.get("name")
        if not isinstance(name, str) or not name:
            raise BoundaryViolation(f"cargo metadata.packages[{index}].name: expected string")
        if name in by_name:
            raise BoundaryViolation(f"cargo metadata: duplicate workspace package {name!r}")
        by_name[name] = package
    return by_name


def dependency_names(package: Mapping[str, object]) -> tuple[set[str], set[str]]:
    raw_dependencies = require_sequence(
        package.get("dependencies"),
        f"cargo metadata package {package.get('name')!r}.dependencies",
    )
    all_dependencies: set[str] = set()
    normal_dependencies: set[str] = set()
    for index, raw_dependency in enumerate(raw_dependencies):
        dependency = require_mapping(
            raw_dependency,
            f"cargo metadata package {package.get('name')!r}.dependencies[{index}]",
        )
        name = dependency.get("name")
        if not isinstance(name, str) or not name:
            raise BoundaryViolation(
                f"cargo metadata package {package.get('name')!r}: dependency name is invalid"
            )
        all_dependencies.add(name)
        if dependency.get("kind") is None:
            normal_dependencies.add(name)
    return all_dependencies, normal_dependencies


def metadata_violations(metadata: Mapping[str, object]) -> list[str]:
    packages = workspace_packages(metadata)
    violations: list[str] = []
    if FACADE_PACKAGE not in packages:
        violations.append(f"workspace is missing required package {FACADE_PACKAGE!r}")
    for consumer in PRODUCTION_CONSUMERS:
        package = packages.get(consumer)
        if package is None:
            violations.append(f"workspace is missing production consumer {consumer!r}")
            continue
        _, normal_dependencies = dependency_names(package)
        if FACADE_PACKAGE not in normal_dependencies:
            violations.append(
                f"{consumer}: must have a normal dependency on {FACADE_PACKAGE!r}"
            )
        forbidden = sorted(normal_dependencies & FORBIDDEN_DIRECT_DEPENDENCIES)
        if forbidden:
            violations.append(
                f"{consumer}: forbidden direct search-index dependencies: {', '.join(forbidden)}"
            )
    return violations


def is_test_source(path: pathlib.Path) -> bool:
    return path.name == "tests.rs" or path.name.endswith("_tests.rs")


def source_violations(root: pathlib.Path) -> list[str]:
    violations: list[str] = []
    for relative_root in PRODUCTION_SOURCE_ROOTS:
        source_root = root / relative_root
        if not source_root.is_dir():
            violations.append(f"missing production source root: {relative_root.as_posix()}")
            continue
        for path in sorted(source_root.rglob("*.rs")):
            if is_test_source(path):
                continue
            relative_path = path.relative_to(root).as_posix()
            for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                for symbol, pattern in FORBIDDEN_SOURCE_PATTERNS:
                    if pattern.search(line):
                        violations.append(
                            f"{relative_path}:{line_number}: forbidden direct search owner API {symbol}"
                        )
    return violations


def validate(root: pathlib.Path, metadata: Mapping[str, object]) -> None:
    violations = metadata_violations(metadata) + source_violations(root)
    if violations:
        details = "\n".join(f"- {violation}" for violation in sorted(violations))
        raise BoundaryViolation(f"search-runtime boundary violations:\n{details}")


def fixture_metadata() -> dict[str, object]:
    def dependency(name: str, kind: str | None = None) -> dict[str, object]:
        return {"name": name, "kind": kind}

    def package(name: str, dependencies: list[dict[str, object]]) -> dict[str, object]:
        return {
            "id": f"path+file:///fixture/{name}#0.1.0",
            "name": name,
            "dependencies": dependencies,
        }

    packages = [
        package(FACADE_PACKAGE, [dependency("index-fulltext"), dependency("index-vector")]),
        package("resume-daemon", [dependency(FACADE_PACKAGE)]),
        package("resume-cli", [dependency(FACADE_PACKAGE)]),
        package("import-pipeline", [dependency("index-fulltext"), dependency("index-vector")]),
        package("benchmark-runner", [dependency("index-fulltext"), dependency("index-vector")]),
    ]
    return {
        "packages": packages,
        "workspace_members": [package["id"] for package in packages],
    }


def write_fixture_sources(root: pathlib.Path) -> None:
    for crate in ("daemon", "cli"):
        source_root = root / "crates" / crate / "src"
        source_root.mkdir(parents=True)
        (source_root / "main.rs").write_text(
            "use search_runtime::QueryCoordinator;\nfn main() {}\n",
            encoding="utf-8",
        )
        (source_root / "query_tests.rs").write_text(
            "use index_fulltext::SnapshotReadLease;\n",
            encoding="utf-8",
        )
        integration_root = root / "crates" / crate / "tests"
        integration_root.mkdir(parents=True)
        (integration_root / "fixture.rs").write_text(
            "use index_vector::PersistentVectorIndex;\n",
            encoding="utf-8",
        )


def expect_failure(action, expected_text: str) -> None:
    try:
        action()
    except BoundaryViolation as error:
        if expected_text not in str(error):
            raise AssertionError(
                f"expected failure containing {expected_text!r}, got {error!s}"
            ) from error
    else:
        raise AssertionError(f"expected boundary failure containing {expected_text!r}")


def run_self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="resume-ir-search-boundary-") as directory:
        root = pathlib.Path(directory)
        write_fixture_sources(root)
        legal_metadata = fixture_metadata()
        validate(root, legal_metadata)

        for forbidden_dependency in sorted(FORBIDDEN_DIRECT_DEPENDENCIES):
            illegal_dependency = copy.deepcopy(legal_metadata)
            daemon = next(
                package
                for package in illegal_dependency["packages"]
                if package["name"] == "resume-daemon"
            )
            daemon["dependencies"].append({"name": forbidden_dependency, "kind": None})
            expect_failure(
                lambda: validate(root, illegal_dependency),
                "resume-daemon: forbidden direct search-index dependencies: "
                f"{forbidden_dependency}",
            )

        test_only_indexes = copy.deepcopy(legal_metadata)
        daemon = next(
            package
            for package in test_only_indexes["packages"]
            if package["name"] == "resume-daemon"
        )
        daemon["dependencies"].extend(
            {"name": dependency, "kind": "dev"}
            for dependency in sorted(FORBIDDEN_DIRECT_DEPENDENCIES)
        )
        validate(root, test_only_indexes)

        dev_only_facade = copy.deepcopy(legal_metadata)
        cli = next(
            package
            for package in dev_only_facade["packages"]
            if package["name"] == "resume-cli"
        )
        cli_facade = next(
            dependency
            for dependency in cli["dependencies"]
            if dependency["name"] == FACADE_PACKAGE
        )
        cli_facade["kind"] = "dev"
        expect_failure(
            lambda: validate(root, dev_only_facade),
            "resume-cli: must have a normal dependency on 'search-runtime'",
        )

        illegal_symbol = root / "crates" / "cli" / "src" / "main.rs"
        for symbol, fixture_line in FORBIDDEN_SOURCE_FIXTURES:
            illegal_symbol.write_text(
                f"use search_runtime::QueryCoordinator;\n{fixture_line}\n",
                encoding="utf-8",
            )
            expect_failure(
                lambda: validate(root, legal_metadata),
                f"forbidden direct search owner API {symbol}",
            )
        illegal_symbol.write_text(
            "use search_runtime::QueryCoordinator;\nfn main() {}\n",
            encoding="utf-8",
        )

        missing_facade = copy.deepcopy(legal_metadata)
        missing_facade["packages"] = [
            package
            for package in missing_facade["packages"]
            if package["name"] != FACADE_PACKAGE
        ]
        missing_facade["workspace_members"] = [
            package["id"] for package in missing_facade["packages"]
        ]
        expect_failure(
            lambda: validate(root, missing_facade),
            "workspace is missing required package 'search-runtime'",
        )


def cargo_metadata(root: pathlib.Path, cargo: str) -> Mapping[str, object]:
    completed = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        stderr = completed.stderr.strip()
        if len(stderr) > 4096:
            stderr = f"{stderr[:4096]}..."
        raise BoundaryViolation(f"cargo metadata failed: {stderr}")
    try:
        return require_mapping(json.loads(completed.stdout), "cargo metadata")
    except json.JSONDecodeError as error:
        raise BoundaryViolation(f"cargo metadata returned invalid JSON: {error}") from error


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run only the synthetic legal and illegal boundary fixtures",
    )
    parser.add_argument(
        "--cargo",
        default=os.environ.get("CARGO", "cargo"),
        help="cargo executable used for workspace metadata",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        run_self_test()
        if args.self_test:
            print("search-runtime boundary self-test passed")
            return 0
        validate(ROOT, cargo_metadata(ROOT, args.cargo))
    except (BoundaryViolation, AssertionError) as error:
        print(error, file=sys.stderr)
        return 1
    print("search-runtime boundary check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
