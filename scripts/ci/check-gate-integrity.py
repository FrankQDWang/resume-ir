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
ATOMIC_BASE_SHA = "7eb3b155358ff91d5a3e4b900182980b28ec8b6d"
ATOMIC_BASE_GOAL_SHA256 = "07cba3670294625aaee873ef1889008308051f14545904d09132edcf025d8214"
INDEX_FULLTEXT_SOURCE = "crates/index-fulltext/src/lib.rs"
INDEX_FULLTEXT_BASE_BLOB = "37f3b8c10dc51c2d4fc5f24282d3e1d74c4aad89"
INDEX_FULLTEXT_BASE_SHA256 = "2cb94fa78593ea1d9af343031d7f7e3f19698ab2c295e7b1e037dde40114afe9"
INDEX_FULLTEXT_FIX_BLOB = "16b465c68dcb7504b5b6d4e196d7c00ca59e22f3"
INDEX_FULLTEXT_FIX_SHA256 = "cfcbee72af9fe60ad0ca781602567c62b834110dbbc4a942bcac7eaf0e37cb02"
FORWARD_CONTRACT_PATHS = {
    "ACTIVE_GOAL.toml",
    "PROGRESS.md",
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-gate-integrity.py",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md",
    "03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md",
    "03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md",
    "03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md",
    INDEX_FULLTEXT_SOURCE,
}
REVERSE_CONTRACT_PATHS = FORWARD_CONTRACT_PATHS - {
    "scripts/ci/check-gate-integrity.py", INDEX_FULLTEXT_SOURCE,
}
NEXT_ISSUE_CONTRACT_PATHS = {
    "ACTIVE_GOAL.toml",
    "PROGRESS.md",
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-gate-integrity.py",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md",
    "03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md",
    "03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md",
    "03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md",
}
CLASSIFIER_CORE_PATHS = {
    "ACTIVE_GOAL.toml",
    "03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md",
    "03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md",
    "03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md",
    "03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md",
    "Cargo.lock",
    "Cargo.toml",
    "PROGRESS.md",
    "crates/resume-classifier/Cargo.toml",
    "crates/resume-classifier/src/lib.rs",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-gate-integrity.py",
}
CLASSIFICATION_AUDIT_PATHS = {
    "ACTIVE_GOAL.toml",
    "PROGRESS.md",
    "Cargo.lock",
    "crates/cli/tests/s146_metadata_key_cli.rs",
    "crates/cli/tests/s147_metadata_key_rotation_cli.rs",
    "crates/meta-store/Cargo.toml",
    "crates/meta-store/src/classification.rs",
    "crates/meta-store/src/lib.rs",
    "crates/meta-store/tests/s3_sqlite.rs",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-gate-integrity.py",
}
CLASSIFIER_ADMISSION_FIXTURE_PATHS = {
    "ACTIVE_GOAL.toml",
    "PROGRESS.md",
    "crates/cli/tests/s10_search_filters.rs",
    "crates/cli/tests/s15_ocr_handoff.rs",
    "crates/cli/tests/s16_persisted_fields.rs",
    "crates/cli/tests/s21_import_candidate_assignment.rs",
    "crates/cli/tests/s9_import_search.rs",
    "crates/daemon/tests/s4_daemon.rs",
    "crates/daemon/tests/s50_ocr_worker.rs",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "scripts/ci/check-gate-integrity.py",
    "tests/fixtures/resumes/synthetic-java-engineer.docx",
    "tests/fixtures/resumes/synthetic-java-platform.pdf",
}
ISSUE_159_SAME_ISSUE_TRANSITIONS = {
    (
        "prepare_classifier_admission_synthetic_fixtures",
        "enforce_classifier_gated_admission",
    ),
    (
        "enforce_classifier_gated_admission",
        "make_ocr_terminal_failure_recoverable",
    ),
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


def approved_index_fulltext_source(base_source: bytes) -> bytes:
    source = base_source.replace(b"\r\n", b"\n").decode("utf-8")
    replacements = [
        (
            "use std::borrow::{Borrow, Cow};\nuse std::collections::BTreeSet;",
            "use std::borrow::{Borrow, Cow};\n#[cfg(test)]\n"
            "use std::cell::Cell;\nuse std::collections::BTreeSet;",
        ),
        (
            "#[cfg(test)]\nstatic REDACTION_REGEX_PASSES: AtomicUsize = AtomicUsize::new(0);\n\n"
            "#[cfg(test)]\nfn record_redaction_regex_pass() {\n"
            "    REDACTION_REGEX_PASSES.fetch_add(1, Ordering::Relaxed);\n}",
            "#[cfg(test)]\nstd::thread_local! {\n"
            "    static REDACTION_REGEX_PASSES: Cell<usize> = const { Cell::new(0) };\n}\n\n"
            "#[cfg(test)]\nfn record_redaction_regex_pass() {\n"
            "    REDACTION_REGEX_PASSES.with(|passes| passes.set(passes.get() + 1));\n}\n\n"
            "#[cfg(test)]\nfn reset_redaction_regex_passes() {\n"
            "    REDACTION_REGEX_PASSES.with(|passes| passes.set(0));\n}\n\n"
            "#[cfg(test)]\nfn redaction_regex_passes() -> usize {\n"
            "    REDACTION_REGEX_PASSES.with(Cell::get)\n}",
        ),
    ]
    for old, new in replacements:
        if source.count(old) != 1:
            fail(f"{INDEX_FULLTEXT_SOURCE}: approved test-only source anchor mismatch")
        source = source.replace(old, new)
    for old, new in [
        ("REDACTION_REGEX_PASSES.store(0, Ordering::Relaxed);", "reset_redaction_regex_passes();"),
        ("REDACTION_REGEX_PASSES.load(Ordering::Relaxed)", "redaction_regex_passes()"),
    ]:
        if source.count(old) != 3:
            fail(f"{INDEX_FULLTEXT_SOURCE}: approved counter call-site count mismatch")
        source = source.replace(old, new)
    anchor = "    #[test]\n    fn contact_redaction_borrows_text_when_no_redaction_is_needed() {"
    regression = (
        "    #[test]\n    fn redaction_regex_pass_observation_is_thread_local() {\n"
        "        reset_redaction_regex_passes();\n\n"
        "        let worker = thread::spawn(|| {\n"
        "            reset_redaction_regex_passes();\n"
        "            record_redaction_regex_pass();\n"
        "            assert_eq!(redaction_regex_passes(), 1);\n"
        "        });\n"
        "        worker.join().unwrap();\n\n"
        "        assert_eq!(redaction_regex_passes(), 0);\n"
        "    }\n\n"
    )
    if source.count(anchor) != 1:
        fail(f"{INDEX_FULLTEXT_SOURCE}: approved regression-test anchor mismatch")
    return source.replace(anchor, regression + anchor).encode()


def require_exact_index_fulltext_fix(merge_base: str, changed: set[str]) -> None:
    if INDEX_FULLTEXT_SOURCE not in changed:
        fail(f"{INDEX_FULLTEXT_SOURCE}: exact forward transition is missing the Rust repair")
    base_source = subprocess.check_output(
        ["git", "show", f"{merge_base}:{INDEX_FULLTEXT_SOURCE}"], cwd=ROOT
    )
    head_source = (ROOT / INDEX_FULLTEXT_SOURCE).read_bytes()
    approved_source = approved_index_fulltext_source(base_source)
    actual = (git(["rev-parse", f"{merge_base}:{INDEX_FULLTEXT_SOURCE}"]), source_sha256(base_source), git_blob_id(approved_source), source_sha256(approved_source), git_blob_id(head_source), source_sha256(head_source))
    expected = (INDEX_FULLTEXT_BASE_BLOB, INDEX_FULLTEXT_BASE_SHA256, INDEX_FULLTEXT_FIX_BLOB, INDEX_FULLTEXT_FIX_SHA256, INDEX_FULLTEXT_FIX_BLOB, INDEX_FULLTEXT_FIX_SHA256)
    if actual != expected or head_source.replace(b"\r\n", b"\n") != approved_source:
        fail(f"{INDEX_FULLTEXT_SOURCE}: #143 Rust change must match the exact approved test-only repair")


def require_atomic_forward_candidate(merge_base: str, changed: set[str]) -> None:
    if not ref_exists(ATOMIC_BASE_REF) or merge_base != ATOMIC_BASE_SHA or git(["rev-parse", ATOMIC_BASE_REF]) != ATOMIC_BASE_SHA:
        fail(f"atomic #143 base/ref must both equal {ATOMIC_BASE_SHA}")
    base_goal = subprocess.check_output(["git", "show", f"{merge_base}:ACTIVE_GOAL.toml"], cwd=ROOT)
    if source_sha256(base_goal) != ATOMIC_BASE_GOAL_SHA256:
        fail("atomic #143 base ACTIVE_GOAL.toml SHA-256 mismatch")

    expected_entries = {path: "M" for path in changed}
    actual_entries: dict[str, str] = {}
    for line in git(["diff", "--cached", "--raw", "--no-abbrev", merge_base]).splitlines():
        header, path = line.split("\t", 1)
        old_mode, new_mode, _old_oid, _new_oid, status = header[1:].split()
        expected_old_mode = "000000" if status == "A" else "100644"
        if status not in {"A", "M"} or old_mode != expected_old_mode or new_mode != "100644":
            fail(f"atomic #143 path {path!r} has invalid status/mode")
        actual_entries[path] = status
    if actual_entries != expected_entries:
        fail(f"atomic #143 status/path set mismatch: {actual_entries!r}")

    commit_count = int(git(["rev-list", "--count", f"{merge_base}..HEAD"]) or "0")
    if commit_count > 5:
        fail(f"atomic #143 commit budget exceeded: {commit_count} > 5")
    stats = [line.split("\t", 2) for line in git(["diff", "--cached", "--numstat", merge_base]).splitlines()]
    if any(added == "-" or deleted == "-" for added, deleted, _path in stats):
        fail("atomic #143 candidate must contain only text files")
    changed_lines = sum(int(added) + int(deleted) for added, deleted, _path in stats)
    if changed_lines > 800:
        fail(f"atomic #143 changed-line budget exceeded: {changed_lines} > 800")
    require_exact_index_fulltext_fix(merge_base, changed)


def validate_transition_scope(base_goal: dict, head_goal: dict, merge_base: str, changed: set[str]) -> None:
    base_slice = base_goal.get("scope", {}).get("active_slice", {})
    head_slice = head_goal.get("scope", {}).get("active_slice", {})
    base_issue = base_slice.get("issue")
    head_issue = head_slice.get("issue")
    if base_issue == head_issue:
        if head_issue == "#143" and changed:
            fail("same-issue #143 changes are forbidden; use the exact #143 -> #140 restoration")
        if head_issue == "#159" and changed:
            base_name = base_slice.get("name")
            head_name = head_slice.get("name")
            if (base_name, head_name) not in ISSUE_159_SAME_ISSUE_TRANSITIONS:
                fail(
                    "same-issue #159 changes require an explicitly authorized named "
                    "slice transition"
                )
            require_bool(
                head_slice.get("production_code_allowed"),
                True,
                "head.scope.active_slice.production_code_allowed",
            )
            require_bool(
                head_slice.get("private_benchmark_allowed"),
                False,
                "head.scope.active_slice.private_benchmark_allowed",
            )
            require_bool(head_slice.get("scope_exception"), False, "head.scope.active_slice.scope_exception")
            allowed_paths = head_slice.get("allowed_paths")
            if not isinstance(allowed_paths, list) or not all(
                isinstance(path, str) and path for path in allowed_paths
            ):
                fail("same-issue #159 production slice requires non-empty allowed_paths")
            expected_paths = set(allowed_paths)
            if len(expected_paths) != len(allowed_paths) or changed != expected_paths:
                fail(
                    "same-issue #159 path mismatch: expected exact ACTIVE_GOAL allowed_paths "
                    f"{sorted(expected_paths)!r}, found {sorted(changed)!r}"
                )
        return

    if (base_issue, head_issue) == ("#140", "#143"):
        require_bool(
            base_slice.get("contract_change_allowed"),
            True,
            "base.scope.active_slice.contract_change_allowed",
        )
        if changed != FORWARD_CONTRACT_PATHS:
            fail(f"#140 -> #143 path mismatch: expected {sorted(FORWARD_CONTRACT_PATHS)!r}, found {sorted(changed)!r}")
        require_atomic_forward_candidate(merge_base, changed)
        return

    if (base_issue, head_issue) == ("#143", "#140"):
        targets = base_slice.get("allowed_contract_transition_targets")
        if not isinstance(targets, list) or "#140" not in targets:
            fail("#143 contract does not authorize return to #140")
        if changed != REVERSE_CONTRACT_PATHS:
            fail(f"#143 -> #140 path mismatch: expected {sorted(REVERSE_CONTRACT_PATHS)!r}, found {sorted(changed)!r}")
        return

    if (base_issue, head_issue) == ("#140", "#152"):
        require_bool(
            base_slice.get("contract_change_allowed"),
            True,
            "base.scope.active_slice.contract_change_allowed",
        )
        require_bool(
            head_slice.get("production_code_allowed"),
            False,
            "head.scope.active_slice.production_code_allowed",
        )
        require_bool(
            head_slice.get("private_benchmark_allowed"),
            True,
            "head.scope.active_slice.private_benchmark_allowed",
        )
        if changed != NEXT_ISSUE_CONTRACT_PATHS:
            fail(
                "#140 -> #152 path mismatch: expected "
                f"{sorted(NEXT_ISSUE_CONTRACT_PATHS)!r}, found {sorted(changed)!r}"
            )
        return

    if (base_issue, head_issue) == ("#152", "#155"):
        require_bool(head_slice.get("production_code_allowed"), True, "head.scope.active_slice.production_code_allowed")
        require_bool(head_slice.get("private_benchmark_allowed"), False, "head.scope.active_slice.private_benchmark_allowed")
        if changed != CLASSIFIER_CORE_PATHS:
            fail(f"#152 -> #155 path mismatch: expected {sorted(CLASSIFIER_CORE_PATHS)!r}, found {sorted(changed)!r}")
        return

    if (base_issue, head_issue) == ("#155", "#157"):
        require_bool(head_slice.get("production_code_allowed"), True, "head.scope.active_slice.production_code_allowed")
        require_bool(head_slice.get("private_benchmark_allowed"), False, "head.scope.active_slice.private_benchmark_allowed")
        require_bool(head_slice.get("scope_exception"), False, "head.scope.active_slice.scope_exception")
        if changed != CLASSIFICATION_AUDIT_PATHS:
            fail(
                "#155 -> #157 path mismatch: expected "
                f"{sorted(CLASSIFICATION_AUDIT_PATHS)!r}, found {sorted(changed)!r}"
            )
        return

    if (base_issue, head_issue) == ("#157", "#159"):
        require_bool(head_slice.get("production_code_allowed"), True, "head.scope.active_slice.production_code_allowed")
        require_bool(head_slice.get("private_benchmark_allowed"), False, "head.scope.active_slice.private_benchmark_allowed")
        require_bool(head_slice.get("scope_exception"), False, "head.scope.active_slice.scope_exception")
        if changed != CLASSIFIER_ADMISSION_FIXTURE_PATHS:
            fail(
                "#157 -> #159 path mismatch: expected "
                f"{sorted(CLASSIFIER_ADMISSION_FIXTURE_PATHS)!r}, found {sorted(changed)!r}"
            )
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
