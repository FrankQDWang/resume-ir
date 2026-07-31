#!/usr/bin/env python3
"""Guard public contract files against private evidence leakage."""

from __future__ import annotations

import json
import math
import pathlib
import re
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[2]
MAX_TRACKED_PERF_RUN_JSON_BYTES = 64 * 1024
PUBLIC_PRIVACY_FLAGS = {
    "contains_raw_resume_text",
    "contains_raw_query_text",
    "contains_candidate_results",
    "contains_local_paths",
    "contains_tokens",
    "contains_diagnostics_package",
}
REDACTED_AGGREGATE_SCHEMA_VERSION = "resume-ir.redacted-aggregate.v1"
REDACTED_AGGREGATE_PATH_PATTERN = re.compile(
    r"aggregate(?:-[A-Za-z0-9][A-Za-z0-9._-]{0,63})?\.json"
)
SAFE_AGGREGATE_KEY_PATTERN = re.compile(r"[a-z][a-z0-9_]{0,63}")
FORBIDDEN_PUBLIC_FIELD_NAMES = {
    "raw_resume_text",
    "raw_query_text",
    "raw_query",
    "raw_hash",
    "candidate_results",
    "resolved_path",
    "local_path",
    "file_path",
    "source_path",
    "filename",
    "file_name",
    "query",
    "query_text",
    "trace",
    "trace_path",
    "private_artifact",
}
EVENT_KEYS = {
    "schema_version",
    "run_id",
    "state_version",
    "expected_state_version",
    "previous_event_hash",
    "observed_at",
    "lease_owner",
    "lease_expires_at",
    "heartbeat_at",
    "action_id",
    "idempotency_key",
    "last_confirmed_side_effect",
    "next_wake_at",
    "transition",
    "result",
    "evidence_refs",
    "evidence",
    "observation",
    "verification",
    "privacy",
}


def fail(message: str) -> None:
    raise ValueError(message)


def load_json(path: pathlib.Path) -> object:
    with path.open("rb") as fh:
        return json.load(fh)


def load_toml(path: pathlib.Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


ACTUAL_PRIVATE_SNIPPETS = ["/Users/frankqdwang", "~/Agents", "~/MLE"]
ALLOWED_SYMBOLIC_SOURCES = {
    "$RESUME_IR_PRIVATE_RESUME_ROOT",
    "$RESUME_IR_QUERY_ARTIFACT_ROOT",
    "$RESUME_IR_LOCAL_EVIDENCE_DIR",
}
PROHIBITED_PUBLIC_PATH_PATTERNS = [
    (re.compile(r"/" + r"Users/[^\s`\"'|)]+"), "macOS user home path"),
    (re.compile(r"~" + r"/[^\s`\"'|)]+"), "tilde home path"),
    (re.compile(r"\$" + r"HOME/[^\s`\"'|)]+"), "HOME env user path"),
    (re.compile(r"C:" + r"(?:\\\\|/)Users(?:\\\\|/)[^\s`\"'|)]+"), "Windows user home path"),
    (re.compile(r"\\" + r"Users\\[^\s`\"'|)]+"), "bare Windows user home path"),
]
PATTERN_DEFINITION_TOKENS = [
    "ACTUAL_PRIVATE_SNIPPETS",
    "ALLOWED_SYMBOLIC_SOURCES",
    "PROHIBITED_PUBLIC_PATH_PATTERNS",
    "PATTERN_DEFINITION_TOKENS",
]
QUERY_SET_HASH_ALLOWED_GUARDS = [
    "不得使用 `query_set_hash`",
    "forbidden query_set_hash field name",
]
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


def tracked_ui_reference_files() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files", "UI-reference"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return [line for line in result.stdout.splitlines() if line]


def tracked_perf_run_json_files() -> list[pathlib.Path]:
    result = subprocess.run(
        ["git", "ls-files", "--", "perf/runs"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return [
        ROOT / line
        for line in result.stdout.splitlines()
        if line.startswith("perf/runs/") and line.endswith(".json")
    ]


def files_to_scan() -> list[pathlib.Path]:
    paths = [
        ROOT / "AGENTS.md",
        ROOT / "GOAL.md",
        ROOT / "MANIFEST.md",
        ROOT / "ACTIVE_GOAL.toml",
        ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md",
        ROOT / ".github" / "workflows" / "pr.yml",
    ]
    paths.extend(sorted((ROOT / ".github" / "ISSUE_TEMPLATE").glob("*.md")))
    paths.extend(sorted((ROOT / "docs" / "superpowers").glob("**/*.md")))
    paths.extend(sorted((ROOT / "03_next_goal_高性能本地检索GUI闭环").glob("**/*.md")))
    paths.extend(sorted((ROOT / "perf").glob("*.json")))
    paths.extend(sorted((ROOT / "perf").glob("*.toml")))
    paths.extend(sorted((ROOT / "perf" / "fixtures").glob("**/*.json")))
    paths.extend(tracked_perf_run_json_files())
    return paths


def validate_public_privacy_flags(value: object, path: str) -> None:
    if not isinstance(value, dict) or set(value) != PUBLIC_PRIVACY_FLAGS:
        fail(f"{path}: privacy flags shape mismatch")
    for key, flag in value.items():
        if flag is not False:
            fail(f"{path}.{key}: public privacy flag must be false")


def reject_forbidden_public_fields(value: object, path: str) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_PUBLIC_FIELD_NAMES:
                fail(f"{path}: forbidden public field {key!r}")
            if key != "privacy":
                reject_forbidden_public_fields(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_forbidden_public_fields(child, f"{path}[{index}]")


def validate_aggregate_map(value: object, path: str, maximum: int) -> None:
    if not isinstance(value, dict) or len(value) > 64:
        fail(f"{path}: expected at most 64 bounded aggregate entries")
    for key, number in value.items():
        if not isinstance(key, str) or SAFE_AGGREGATE_KEY_PATTERN.fullmatch(key) is None:
            fail(f"{path}: invalid aggregate key")
        if isinstance(number, bool) or not isinstance(number, (int, float)):
            fail(f"{path}.{key}: expected numeric aggregate")
        if not math.isfinite(number) or number < 0 or number > maximum:
            fail(f"{path}.{key}: aggregate value out of bounds")


def validate_redacted_aggregate(value: object, path: str) -> None:
    if not isinstance(value, dict) or set(value) != {"schema_version", "privacy", "aggregate"}:
        fail(f"{path}: redacted aggregate object shape mismatch")
    if value["schema_version"] != REDACTED_AGGREGATE_SCHEMA_VERSION:
        fail(f"{path}.schema_version: unsupported redacted aggregate schema")
    validate_public_privacy_flags(value["privacy"], f"{path}.privacy")
    aggregate = value["aggregate"]
    if not isinstance(aggregate, dict) or set(aggregate) != {"label", "counts", "durations_ms", "bytes"}:
        fail(f"{path}.aggregate: bounded aggregate shape mismatch")
    if aggregate["label"] != "import_attribution":
        fail(f"{path}.aggregate.label: milestone-specific reports are not authorized")
    validate_aggregate_map(aggregate["counts"], f"{path}.aggregate.counts", 1_000_000_000)
    validate_aggregate_map(aggregate["durations_ms"], f"{path}.aggregate.durations_ms", 1_000_000_000_000)
    validate_aggregate_map(aggregate["bytes"], f"{path}.aggregate.bytes", 1_000_000_000_000_000)


def validate_event_object(value: object, path: str) -> None:
    if not isinstance(value, dict):
        fail(f"{path}: event must be a JSON object")
    required = {
        "schema_version", "run_id", "state_version", "expected_state_version",
        "previous_event_hash", "transition", "result", "evidence_refs",
        "observation", "verification", "privacy", "observed_at", "lease_owner",
        "lease_expires_at", "heartbeat_at", "action_id", "idempotency_key",
        "last_confirmed_side_effect", "next_wake_at",
    }
    if not required <= set(value):
        fail(f"{path}: event shape missing required fields")
    if not set(value) <= EVENT_KEYS:
        fail(f"{path}: event contains unsupported fields")
    if value["schema_version"] not in {
        "resume-ir.loop-reconciliation-event.v1",
        "resume-ir.loop-reconciliation-event.v2",
    }:
        fail(f"{path}.schema_version: unsupported event schema")
    if (
        isinstance(value["state_version"], bool)
        or not isinstance(value["state_version"], int)
        or isinstance(value["expected_state_version"], bool)
        or not isinstance(value["expected_state_version"], int)
    ):
        fail(f"{path}: event versions must be integers")
    for key in ("run_id", "observed_at", "lease_owner", "lease_expires_at", "heartbeat_at", "action_id", "idempotency_key", "last_confirmed_side_effect", "next_wake_at"):
        if not isinstance(value[key], str) or not value[key] or len(value[key]) > 512:
            fail(f"{path}.{key}: expected bounded string")
    if not isinstance(value["previous_event_hash"], str) or re.fullmatch(r"[0-9a-f]{64}", value["previous_event_hash"]) is None:
        fail(f"{path}.previous_event_hash: expected SHA-256")
    if value["result"] != "passed" or not isinstance(value["evidence_refs"], list) or len(value["evidence_refs"]) > 64:
        fail(f"{path}: event result/evidence shape mismatch")
    if not all(isinstance(item, str) and item and len(item) <= 512 for item in value["evidence_refs"]):
        fail(f"{path}.evidence_refs: expected bounded strings")
    transition = value["transition"]
    if (
        not isinstance(transition, dict)
        or set(transition) != {"name", "from", "to"}
        or not all(isinstance(transition[key], str) and transition[key] and len(transition[key]) <= 512 for key in transition)
    ):
        fail(f"{path}: event transition/observation shape mismatch")
    if not isinstance(value["observation"], dict) or len(value["observation"]) > 64:
        fail(f"{path}.observation: expected bounded object")
    if "evidence" in value and (not isinstance(value["evidence"], dict) or len(value["evidence"]) > 64):
        fail(f"{path}.evidence: expected bounded object")
    if not isinstance(value["verification"], list):
        fail(f"{path}.verification: expected list")
    if len(value["verification"]) > 64:
        fail(f"{path}.verification: too many entries")
    validate_public_privacy_flags(value["privacy"], f"{path}.privacy")


def validate_perf_run_json_content(rel_path: str, raw: bytes) -> None:
    if len(raw) > MAX_TRACKED_PERF_RUN_JSON_BYTES:
        fail(f"{rel_path}: tracked perf/runs JSON exceeds {MAX_TRACKED_PERF_RUN_JSON_BYTES} bytes")
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as exc:
        fail(f"{rel_path}: invalid JSON: {exc}")
    text = raw.decode("utf-8")
    for snippet in ACTUAL_PRIVATE_SNIPPETS:
        if snippet in text:
            fail(f"{rel_path}: forbidden private path snippet {snippet}")
    for pattern, description in PROHIBITED_PUBLIC_PATH_PATTERNS:
        if pattern.search(text):
            fail(f"{rel_path}: forbidden public {description}")
    reject_forbidden_public_fields(value, rel_path)
    parts = pathlib.PurePosixPath(rel_path).parts
    if len(parts) < 4 or parts[0:2] != ("perf", "runs"):
        fail(f"{rel_path}: expected perf/runs JSON path")
    if parts[-2] == "events":
        validate_event_object(value, rel_path)
    elif parts[-2] == "redacted":
        if REDACTED_AGGREGATE_PATH_PATTERN.fullmatch(parts[-1]) is None:
            fail(f"{rel_path}: only bounded aggregate redacted artifacts are authorized")
        validate_redacted_aggregate(value, rel_path)
    else:
        fail(f"{rel_path}: unsupported perf/runs JSON directory")


def check_file(path: pathlib.Path) -> None:
    raw = path.read_bytes()
    text = raw.decode("utf-8")
    rel = path.relative_to(ROOT)
    for line_number, line in enumerate(text.splitlines(), start=1):
        if "query_set_hash" in line and not any(
            guard in line for guard in QUERY_SET_HASH_ALLOWED_GUARDS
        ):
            fail(f"{rel}:{line_number}: forbidden query_set_hash field name")
        if any(token in line for token in PATTERN_DEFINITION_TOKENS):
            continue
        for snippet in ACTUAL_PRIVATE_SNIPPETS:
            if snippet in line:
                fail(f"{rel}:{line_number}: forbidden private path snippet {snippet}")
        for pattern, description in PROHIBITED_PUBLIC_PATH_PATTERNS:
            if pattern.search(line):
                fail(f"{rel}:{line_number}: forbidden public {description}")
    for pattern in RAW_PRIVATE_TRUE_PATTERNS:
        if pattern.search(text):
            fail(f"{rel}: raw private data marker must not be true")
    if rel.as_posix().startswith("perf/runs/") and rel.suffix == ".json":
        validate_perf_run_json_content(rel.as_posix(), raw)


def self_test() -> None:
    valid_event = {
        "schema_version": "resume-ir.loop-reconciliation-event.v2",
        "run_id": "test-run",
        "state_version": 566,
        "expected_state_version": 565,
        "previous_event_hash": "a" * 64,
        "observed_at": "2026-07-31T00:00:00Z",
        "lease_owner": "test-owner",
        "lease_expires_at": "2026-07-31T00:01:00Z",
        "heartbeat_at": "2026-07-31T00:00:30Z",
        "action_id": "test-action",
        "idempotency_key": "test-key",
        "last_confirmed_side_effect": "test",
        "next_wake_at": "2026-07-31T00:02:00Z",
        "transition": {"name": "authorize_current_main_import_attribution", "from": "contract_conflict", "to": "goal_authorized"},
        "result": "passed",
        "evidence_refs": ["https://github.com/FrankQDWang/resume-ir/issues/270"],
        "observation": {"owner_issue": "#270", "active_prs": []},
        "verification": [],
        "privacy": {key: False for key in PUBLIC_PRIVACY_FLAGS},
    }
    valid_aggregate = {
        "schema_version": REDACTED_AGGREGATE_SCHEMA_VERSION,
        "privacy": {key: False for key in PUBLIC_PRIVACY_FLAGS},
        "aggregate": {
            "label": "import_attribution",
            "counts": {"documents": 3},
            "durations_ms": {"import": 12.5},
            "bytes": {"archive": 1024},
        },
    }
    validate_perf_run_json_content(
        "perf/runs/test-run/events/566.json", json.dumps(valid_event).encode()
    )
    validate_perf_run_json_content(
        "perf/runs/test-run/redacted/aggregate.json", json.dumps(valid_aggregate).encode()
    )
    failures = []
    for label, path, value in (
        ("oversized", "perf/runs/test-run/redacted/aggregate.json", b"0" * (MAX_TRACKED_PERF_RUN_JSON_BYTES + 1)),
        ("privacy true", "perf/runs/test-run/redacted/aggregate.json", json.dumps({**valid_aggregate, "privacy": {**valid_aggregate["privacy"], "contains_local_paths": True}}).encode()),
        ("local path field", "perf/runs/test-run/redacted/aggregate.json", json.dumps({**valid_aggregate, "aggregate": {**valid_aggregate["aggregate"], "counts": {"resolved_path": 1}}}).encode()),
        ("milestone artifact", "perf/runs/test-run/redacted/embedding-complete.json", json.dumps(valid_aggregate).encode()),
    ):
        try:
            validate_perf_run_json_content(path, value)
        except ValueError:
            continue
        failures.append(label)
    if failures:
        fail(f"private evidence self-test accepted {failures}")


def main() -> int:
    if sys.argv[1:]:
        if sys.argv[1:] == ["--self-test"]:
            self_test()
            print("check-private-evidence-redaction.py self-test passed")
            return 0
        fail("usage: check-private-evidence-redaction.py [--self-test]")
    ui_reference_files = tracked_ui_reference_files()
    if ui_reference_files:
        fail(
            "UI-reference/ contains tracked local visual reference assets; "
            "remove these files from git tracking: "
            + ", ".join(ui_reference_files)
        )

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
