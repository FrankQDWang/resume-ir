#!/usr/bin/env python3
"""Validate guard and merge-policy integrity for autonomous delivery."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[2]
ATOMIC_BASE_REF = "origin/main"
ATOMIC_BASE_SHA = "4afd254d9b7989108d726a737a3cc939c9f45deb"
ATOMIC_BASE_GOAL_SHA256 = "07cba3670294625aaee873ef1889008308051f14545904d09132edcf025d8214"
IMPORT_PIPELINE_SOURCE = "crates/import-pipeline/src/lib.rs"
IMPORT_PIPELINE_BASE_BLOB = "ebe399c1f99cdf09ba80b696519ced69053e604a"
IMPORT_PIPELINE_BASE_SHA256 = "7dbe7d72ed49d7062702a7d3e3d3ce98effad7fc028668870969f2f16d004609"
IMPORT_PIPELINE_FIX_BLOB = "282104f377b37615453702db8547010694ba157a"
IMPORT_PIPELINE_FIX_SHA256 = "061b7cbfe557ef4cddb4065daa35b45a966ad017325be020555d413b52a49b9c"
FORWARD_CONTRACT_PATHS = {
    "ACTIVE_GOAL.toml",
    "PROGRESS.md",
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-loop-state.py",
    "scripts/ci/check-gate-integrity.py",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md",
    "03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md",
    "03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md",
    "03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md",
    IMPORT_PIPELINE_SOURCE,
}
REVERSE_CONTRACT_PATHS = FORWARD_CONTRACT_PATHS - {
    "scripts/ci/check-loop-state.py", "scripts/ci/check-gate-integrity.py", IMPORT_PIPELINE_SOURCE,
}


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


def git(args: list[str]) -> str:
    completed = subprocess.run(
        ["git", "-c", "core.quotePath=false", *args],
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
    for candidate in [f"origin/{base}", base, "origin/main", "main"]:
        if ref_exists(candidate):
            return candidate
    if os.environ.get("GITHUB_ACTIONS") == "true":
        fetch_base_ref(base)
        for candidate in [f"origin/{base}", base, "origin/main", "main"]:
            if ref_exists(candidate):
                return candidate
    fail("unable to find base ref for gate integrity check")


def merge_base_and_changed_paths() -> tuple[str, set[str]]:
    base_ref = select_base_ref()
    merge_base = git(["merge-base", base_ref, "HEAD"])
    if git(["diff", "--name-only"]) or git(["ls-files", "--others", "--exclude-standard"]):
        fail("gate integrity requires the index and working tree to match with no untracked files")
    output = git(["diff", "--cached", "--name-only", merge_base])
    paths = {path for path in output.splitlines() if path}
    return merge_base, paths


def load_toml_at_revision(revision: str, path: str) -> dict:
    return tomllib.loads(git(["show", f"{revision}:{path}"]))


def source_sha256(source: bytes) -> str:
    return hashlib.sha256(source.replace(b"\r\n", b"\n")).hexdigest()


def git_blob_id(source: bytes) -> str:
    source = source.replace(b"\r\n", b"\n")
    return hashlib.sha1(b"blob " + str(len(source)).encode() + b"\0" + source, usedforsecurity=False).hexdigest()


def approved_import_pipeline_source(base_source: bytes) -> bytes:
    source = base_source.replace(b"\r\n", b"\n").decode("utf-8")
    replacements = [
        ("let (result_tx, result_rx) = mpsc::sync_channel(1);", "let (result_tx, result_rx) = mpsc::sync_channel(1);\n        let (release_tx, release_rx) = mpsc::sync_channel(1);"),
        ("thread::spawn(move || {\n            thread::sleep(Duration::from_millis(180));", "let sender = thread::spawn(move || {\n            release_rx.recv().unwrap();"),
        ("observed_cancel_polls.fetch_add(1, Ordering::SeqCst);", "let poll = observed_cancel_polls.fetch_add(1, Ordering::SeqCst) + 1;\n            if poll == 2 {\n                release_tx.send(()).unwrap();\n            }"),
    ]
    for old, new in replacements:
        if source.count(old) != 1:
            fail(f"{IMPORT_PIPELINE_SOURCE}: approved test-only source anchor mismatch")
        source = source.replace(old, new)
    old = "        .unwrap();\n\n        assert_eq!(result.index, 7);"
    new = "        .unwrap();\n        sender.join().unwrap();\n\n        assert_eq!(result.index, 7);"
    if source.count(old) != 1:
        fail(f"{IMPORT_PIPELINE_SOURCE}: approved sender join anchor mismatch")
    return source.replace(old, new).encode()


def require_exact_import_pipeline_fix(merge_base: str, changed: set[str]) -> None:
    if IMPORT_PIPELINE_SOURCE not in changed:
        fail(f"{IMPORT_PIPELINE_SOURCE}: exact forward transition is missing the Rust repair")
    base_source = subprocess.check_output(
        ["git", "show", f"{merge_base}:{IMPORT_PIPELINE_SOURCE}"], cwd=ROOT
    )
    head_source = (ROOT / IMPORT_PIPELINE_SOURCE).read_bytes()
    approved_source = approved_import_pipeline_source(base_source)
    actual = (git(["rev-parse", f"{merge_base}:{IMPORT_PIPELINE_SOURCE}"]), source_sha256(base_source), git_blob_id(approved_source), source_sha256(approved_source), git_blob_id(head_source), source_sha256(head_source))
    expected = (IMPORT_PIPELINE_BASE_BLOB, IMPORT_PIPELINE_BASE_SHA256, IMPORT_PIPELINE_FIX_BLOB, IMPORT_PIPELINE_FIX_SHA256, IMPORT_PIPELINE_FIX_BLOB, IMPORT_PIPELINE_FIX_SHA256)
    if actual != expected or head_source.replace(b"\r\n", b"\n") != approved_source:
        fail(f"{IMPORT_PIPELINE_SOURCE}: #145 Rust change must match the exact approved test-only repair")


def require_atomic_forward_candidate(merge_base: str, changed: set[str]) -> None:
    if not ref_exists(ATOMIC_BASE_REF) or merge_base != ATOMIC_BASE_SHA or git(["rev-parse", ATOMIC_BASE_REF]) != ATOMIC_BASE_SHA:
        fail(f"atomic bootstrap base/ref must both equal {ATOMIC_BASE_SHA}")
    base_goal = subprocess.check_output(["git", "show", f"{merge_base}:ACTIVE_GOAL.toml"], cwd=ROOT)
    if source_sha256(base_goal) != ATOMIC_BASE_GOAL_SHA256:
        fail("atomic bootstrap base ACTIVE_GOAL.toml SHA-256 mismatch")

    expected_entries = {path: "M" for path in changed}
    actual_entries: dict[str, str] = {}
    for line in git(["diff", "--cached", "--raw", "--no-abbrev", merge_base]).splitlines():
        header, path = line.split("\t", 1)
        old_mode, new_mode, _old_oid, _new_oid, status = header[1:].split()
        expected_old_mode = "000000" if status == "A" else "100644"
        if status not in {"A", "M"} or old_mode != expected_old_mode or new_mode != "100644":
            fail(f"atomic bootstrap path {path!r} has invalid status/mode")
        actual_entries[path] = status
    if actual_entries != expected_entries:
        fail(f"atomic bootstrap status/path set mismatch: {actual_entries!r}")

    commit_count = int(git(["rev-list", "--count", f"{merge_base}..HEAD"]) or "0")
    if commit_count > 5:
        fail(f"atomic bootstrap commit budget exceeded: {commit_count} > 5")
    stats = [line.split("\t", 2) for line in git(["diff", "--cached", "--numstat", merge_base]).splitlines()]
    if any(added == "-" or deleted == "-" for added, deleted, _path in stats):
        fail("atomic bootstrap candidate must contain only text files")
    changed_lines = sum(int(added) + int(deleted) for added, deleted, _path in stats)
    if changed_lines > 800:
        fail(f"atomic bootstrap changed-line budget exceeded: {changed_lines} > 800")
    require_exact_import_pipeline_fix(merge_base, changed)


def validate_transition_scope(base_goal: dict, head_goal: dict, merge_base: str, changed: set[str]) -> None:
    base_slice = base_goal.get("scope", {}).get("active_slice", {})
    head_slice = head_goal.get("scope", {}).get("active_slice", {})
    base_issue = base_slice.get("issue")
    head_issue = head_slice.get("issue")
    if base_issue == head_issue:
        if head_issue == "#145" and changed:
            fail("same-issue #145 changes are forbidden; use the exact #145 -> #140 restoration")
        return

    if (base_issue, head_issue) == ("#140", "#145"):
        require_bool(
            base_slice.get("contract_change_allowed"),
            True,
            "base.scope.active_slice.contract_change_allowed",
        )
        if changed != FORWARD_CONTRACT_PATHS:
            fail(f"#140 -> #145 path mismatch: expected {sorted(FORWARD_CONTRACT_PATHS)!r}, found {sorted(changed)!r}")
        require_atomic_forward_candidate(merge_base, changed)
        return

    if (base_issue, head_issue) == ("#145", "#140"):
        targets = base_slice.get("allowed_contract_transition_targets")
        if not isinstance(targets, list) or "#140" not in targets:
            fail("#145 contract does not authorize return to #140")
        if changed != REVERSE_CONTRACT_PATHS:
            fail(f"#145 -> #140 path mismatch: expected {sorted(REVERSE_CONTRACT_PATHS)!r}, found {sorted(changed)!r}")
        return

    fail(f"unauthorized active-slice transition: {base_issue!r} -> {head_issue!r}")


def is_gate_path(path: str) -> bool:
    if path.startswith(".github/workflows/"):
        return True
    if path.startswith("scripts/ci/check-"):
        return True
    if path in {".github/PULL_REQUEST_TEMPLATE.md", "perf/acceptance-matrix.toml"}:
        return True
    if path.startswith(".github/ISSUE_TEMPLATE/"):
        return True
    if path.startswith("perf/") and path.endswith(".schema.json"):
        return True
    return False


def main() -> int:
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
    merge_base, paths = merge_base_and_changed_paths()
    base_goal = load_toml_at_revision(merge_base, "ACTIVE_GOAL.toml")
    validate_transition_scope(base_goal, active_goal, merge_base, paths)
    autonomous = active_goal.get("autonomous_delivery", {})
    permissions = autonomous.get("permissions")
    if not isinstance(permissions, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.permissions]")
    require_bool(permissions.get("gate_bypass_allowed"), False, "autonomous_delivery.permissions.gate_bypass_allowed")
    require_bool(
        permissions.get("threshold_relaxation_allowed"),
        False,
        "autonomous_delivery.permissions.threshold_relaxation_allowed",
    )

    merge_policy = autonomous.get("merge_policy")
    if not isinstance(merge_policy, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.merge_policy]")
    require_bool(merge_policy.get("require_no_admin_bypass"), True, "autonomous_delivery.merge_policy.require_no_admin_bypass")
    require_bool(merge_policy.get("require_no_direct_main_push"), True, "autonomous_delivery.merge_policy.require_no_direct_main_push")

    active_slice = active_goal.get("scope", {}).get("active_slice", {})
    gate_changes = sorted(path for path in paths if is_gate_path(path))
    if gate_changes and not active_slice.get("scope_exception_reason"):
        fail("gate-changing diff requires scope.active_slice.scope_exception_reason")

    template = (ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text(encoding="utf-8").lower()
    for phrase in [
        "admin bypass is not used",
        "direct main push is not used",
        "requested changes are unresolved",
        "a required gate is bypassed",
        "performance thresholds are lowered",
        "benchmark lanes are mixed",
        "default: do not auto-merge scope exceptions",
    ]:
        if phrase not in template:
            fail(f".github/PULL_REQUEST_TEMPLATE.md: missing integrity phrase {phrase!r}")

    github_ledger = autonomous.get("github_ledger")
    if not isinstance(github_ledger, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.github_ledger]")
    require_bool(github_ledger.get("templates_materialized"), True, "autonomous_delivery.github_ledger.templates_materialized")
    for key in ["profile_issue_template", "pr_template"]:
        value = github_ledger.get(key)
        if not isinstance(value, str) or not value:
            fail(f"autonomous_delivery.github_ledger.{key}: expected path")
        if not (ROOT / value).is_file():
            fail(f"autonomous_delivery.github_ledger.{key}: missing {value}")

    print("check-gate-integrity.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-gate-integrity.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
