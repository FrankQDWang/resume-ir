#!/usr/bin/env python3
"""Guard public contract files against private evidence leakage."""

from __future__ import annotations

import json
import pathlib
import re
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


FORBIDDEN_PATH_SNIPPETS = ["/Users/frankqdwang", "~/Agents", "~/MLE"]
RAW_PRIVATE_TRUE_PATTERNS = [
    re.compile(r'(?m)"contains_raw_resume_text"\s*:\s*true\b'),
    re.compile(r'(?m)"contains_raw_query_text"\s*:\s*true\b'),
    re.compile(r'(?m)"contains_candidate_results"\s*:\s*true\b'),
    re.compile(r'(?m)"contains_local_paths"\s*:\s*true\b'),
    re.compile(r'(?m)"contains_tokens"\s*:\s*true\b'),
    re.compile(r'(?m)"contains_diagnostics_package"\s*:\s*true\b'),
    re.compile(r'(?m)contains_raw_resume_text\s*=\s*true\b'),
    re.compile(r'(?m)contains_raw_query_text\s*=\s*true\b'),
    re.compile(r'(?m)contains_candidate_results\s*=\s*true\b'),
    re.compile(r'(?m)contains_local_paths\s*=\s*true\b'),
    re.compile(r'(?m)contains_tokens\s*=\s*true\b'),
    re.compile(r'(?m)contains_diagnostics_package\s*=\s*true\b'),
    re.compile(r'(?m)contains_model_cache\s*=\s*true\b'),
]


def files_to_scan() -> list[pathlib.Path]:
    paths = [
        ROOT / "ACTIVE_GOAL.toml",
        ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md",
    ]
    paths.extend(sorted((ROOT / ".github" / "ISSUE_TEMPLATE").glob("*.md")))
    paths.extend(sorted((ROOT / "perf").glob("*.json")))
    paths.extend(sorted((ROOT / "perf").glob("*.toml")))
    paths.extend(sorted((ROOT / "perf" / "fixtures").glob("**/*.json")))
    return paths


def check_file(path: pathlib.Path) -> None:
    text = path.read_text(encoding="utf-8")
    rel = path.relative_to(ROOT)
    if "query_set_hash" in text:
        fail(f"{rel}: forbidden query_set_hash field name")
    for snippet in FORBIDDEN_PATH_SNIPPETS:
        if snippet in text:
            fail(f"{rel}: forbidden private path snippet {snippet}")
    for pattern in RAW_PRIVATE_TRUE_PATTERNS:
        if pattern.search(text):
            fail(f"{rel}: raw private data marker must not be true")


def main() -> int:
    for path in files_to_scan():
        if path.exists():
            check_file(path)

    print("check-private-evidence-redaction.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-private-evidence-redaction.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
