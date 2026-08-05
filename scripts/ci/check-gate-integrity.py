#!/usr/bin/env python3
"""Validate guard and merge-policy integrity for autonomous delivery."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import tomllib
from copy import deepcopy


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
BLIND_HOLDOUT_ACCEPTANCE_PATHS = {
    "ACTIVE_GOAL.toml",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
}
SUCCESSOR_TRANSITION_RECORD = "perf/active-slice-transition.json"
SUCCESSOR_TRANSITION_PATHS = {
    "ACTIVE_GOAL.toml",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    SUCCESSOR_TRANSITION_RECORD,
}
SUCCESSOR_TRANSITION_BOOTSTRAP_PATHS = SUCCESSOR_TRANSITION_PATHS | {
    "scripts/ci/check-gate-integrity.py",
}
TRANSITION_COMMON_KEYS = {
    "schema_version", "transition_id", "from_issue", "to_issue",
    "successor_issue_ref", "base_active_goal_sha256", "state_paths",
    "bootstrap_gate_change", "production_code_changed", "blind_holdout_reread",
    "gate_weakening", "privacy",
}
CURRENT_MAIN_ATTRIBUTION_PR_EVENT = (
    "perf/runs/contract-reconciliation-2026-07-30/events/555.json"
)
CURRENT_MAIN_ATTRIBUTION_PRE_PR_PATHS = {
    "ACTIVE_GOAL.toml",
    "PROGRESS.md",
    "docs/superpowers/plans/2026-07-30-current-main-import-attribution-contract.md",
    "03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md",
    "perf/acceptance-matrix.toml",
    "perf/active-slice-transition.json",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "perf/runs/contract-reconciliation-2026-07-30/events/554.json",
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-gate-integrity.py",
    "scripts/ci/check-performance-contracts.py",
    "scripts/loop/reduce-current-loop-state.py",
}
CURRENT_MAIN_ATTRIBUTION_FINAL_PATHS = (
    CURRENT_MAIN_ATTRIBUTION_PRE_PR_PATHS | {CURRENT_MAIN_ATTRIBUTION_PR_EVENT}
)
CURRENT_MAIN_ATTRIBUTION_STATE_PATHS = {
    "ACTIVE_GOAL.toml",
    "perf/active-slice-transition.json",
    "perf/current-loop-state.json",
    "perf/fixtures/valid/synthetic-smoke-artifact-manifest.json",
    "perf/fixtures/valid/synthetic-smoke-baseline-report.json",
    "perf/runs/contract-reconciliation-2026-07-30/events/554.json",
    "scripts/loop/reduce-current-loop-state.py",
}
CURRENT_MAIN_ATTRIBUTION_REACTIVATION = ("#272", "#270")
CURRENT_MAIN_ATTRIBUTION_REACTIVATION_EVENT_PATHS = {
    f"perf/runs/contract-reconciliation-2026-07-30/events/{version}.json"
    for version in range(556, 566)
}
CURRENT_MAIN_ATTRIBUTION_REACTIVATION_SUPPORT_PATHS = {
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-private-evidence-redaction.py",
    "scripts/loop/reduce-current-loop-state.py",
}
CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS = (
    SUCCESSOR_TRANSITION_PATHS
    | {"scripts/ci/check-gate-integrity.py"}
    | CURRENT_MAIN_ATTRIBUTION_REACTIVATION_EVENT_PATHS
    | CURRENT_MAIN_ATTRIBUTION_REACTIVATION_SUPPORT_PATHS
)
CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS_ROLE = "exact_changed_paths_for_reactivation_pr"
CURRENT_MAIN_ATTRIBUTION_ACTIVE_ALLOWED_PATHS_ROLE = "reactivation_pr_paths"
CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATH_TEMPLATES = [
    "PROGRESS.md",
    "perf/current-loop-state.json",
    "perf/runs/<run_id>/events/<state_version>.json",
    "perf/runs/<run_id>/redacted/<aggregate>.json",
]
CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATHS_ROLE = "post_merge_execution_evidence_templates"
CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATH_PATTERNS = (
    ("progress", re.compile(r"PROGRESS\.md")),
    ("derived_state", re.compile(r"perf/current-loop-state\.json")),
    (
        "append_only_event",
        re.compile(r"perf/runs/[A-Za-z0-9][A-Za-z0-9._-]{0,127}/events/[0-9]{1,9}\.json"),
    ),
    (
        "redacted_aggregate",
        re.compile(r"perf/runs/[A-Za-z0-9][A-Za-z0-9._-]{0,127}/redacted/aggregate(?:-[A-Za-z0-9][A-Za-z0-9._-]{0,63})?\.json"),
    ),
)
CURRENT_MAIN_ATTRIBUTION_MILESTONES = [
    "first_searchable",
    "keyword_ready",
    "embedding_complete",
    "ocr_backlog_full_import",
]
CURRENT_MAIN_ATTRIBUTION_REQUIRED_RUNTIME_PROVENANCE = [
    "source_commit",
    "cli_build_provenance",
    "daemon_build_provenance",
    "sidecar_build_provenance",
    "command_shape",
]
CURRENT_MAIN_ATTRIBUTION_POST_MERGE_AUTHORIZATION_EVIDENCE = [
    "owner_issue_270_open",
    "fresh_remote_main_sha",
    "clean_main_equals_remote_main",
    "configured_private_roots",
    "runtime_capability_attestation",
    "primary_lane_full_import_ocr_backlog",
    "phase_attribution_execution",
    "privacy_boundary",
    "reactivation_pr_merged",
    "merged_reactivation_pr",
    "merged_reactivation_head_sha",
    "post_merge_base_sha",
    "base_snapshot_workflow_state",
    "active_prs_empty",
]
CURRENT_MAIN_ATTRIBUTION_AUTHORIZATION_EVENT_NAME = "authorize_current_main_import_attribution"
CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR = "#278"
CURRENT_MAIN_ATTRIBUTION_PUBLIC_PRIVACY_FLAGS = {
    "contains_raw_resume_text",
    "contains_raw_query_text",
    "contains_candidate_results",
    "contains_local_paths",
    "contains_tokens",
    "contains_diagnostics_package",
}
CURRENT_MAIN_ATTRIBUTION_MAX_PERF_RUN_JSON_BYTES = 64 * 1024
CURRENT_MAIN_REACTIVATION_CONTRACT_KEYS = {"reactivation_contract"}


def fail(message: str) -> None:
    raise ValueError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def load_json(path: pathlib.Path) -> object:
    with path.open("rb") as fh:
        return json.load(fh)


def load_toml(path: pathlib.Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def require_bool(value: object, expected: bool, path: str) -> None:
    if value is not expected:
        fail(f"{path}: expected {expected}")


def require_non_empty_string(value: object, path: str) -> str:
    if not isinstance(value, str) or not value or "\n" in value or "\r" in value:
        fail(f"{path}: expected bounded single-line string")
    if len(value) > 512:
        fail(f"{path}: exceeds 512 characters")
    return value


def require_issue_ref(value: object, path: str) -> str:
    value = require_non_empty_string(value, path)
    if not value.startswith("#") or not value[1:].isdigit():
        fail(f"{path}: expected issue ref")
    return value


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


def git_raw(args: list[str]) -> bytes:
    completed = subprocess.run(
        ["git", "-c", "core.quotePath=false", *args],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        fail(f"git {' '.join(args)} failed: {completed.stderr.decode(errors='replace').strip()}")
    return completed.stdout


def parse_name_status(raw: str) -> dict[str, str]:
    statuses: dict[str, str] = {}
    for line in raw.splitlines():
        fields = line.split("\t")
        if len(fields) == 2:
            status, path = fields
            if not path or path in statuses:
                fail(f"gate integrity: malformed or duplicate name-status path {line!r}")
            statuses[path] = status
            continue
        if len(fields) >= 3 and fields[0][:1] in {"R", "C"}:
            fail(f"gate integrity: rename/copy status is not allowed in this contract: {line!r}")
        fail(f"gate integrity: malformed name-status record {line!r}")
    return statuses


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


def merge_base_and_changed_paths() -> tuple[str, dict[str, str]]:
    base_ref = select_base_ref()
    merge_base = git(["merge-base", base_ref, "HEAD"])
    if git(["diff", "--name-only"]) or git(["ls-files", "--others", "--exclude-standard"]):
        fail("gate integrity requires the index and working tree to match with no untracked files")
    output = git(["diff", "--cached", "--name-status", "--find-renames", merge_base])
    return merge_base, parse_name_status(output)


def load_toml_at_revision(revision: str, path: str) -> dict:
    return tomllib.loads(git(["show", f"{revision}:{path}"]))


def load_json_at_revision(revision: str, path: str) -> tuple[dict, bytes]:
    raw = git_raw(["show", f"{revision}:{path}"])
    value = json.loads(raw)
    if not isinstance(value, dict):
        fail(f"{path}@{revision}: expected JSON object")
    return value, raw


def validate_transition_record_shape(raw: dict, base_issue: str, head_issue: str) -> bool:
    current_main_attribution = (base_issue, head_issue) == ("#217", "#270")
    current_main_reactivation = (base_issue, head_issue) == CURRENT_MAIN_ATTRIBUTION_REACTIVATION
    if current_main_attribution:
        routing_keys = {
            "routing_kind", "source_issue_role", "source_issue_remains_open",
            "source_issue_terminal_claim", "routing_evidence_ref",
        }
        routing = {
            "routing_kind": "umbrella_to_bounded_execution_owner",
            "source_issue_role": "umbrella", "source_issue_remains_open": True,
            "source_issue_terminal_claim": False,
        }
        for key, expected in routing.items():
            if raw.get(key) != expected:
                fail(f"{SUCCESSOR_TRANSITION_RECORD}.{key}: mismatch")
        schema, evidence_key = "resume-ir.active-slice-transition.v2", "routing_evidence_ref"
    else:
        routing_keys = {"predecessor_terminal_evidence_ref"}
        schema = "resume-ir.active-slice-transition.v1"
        evidence_key = "predecessor_terminal_evidence_ref"
    if raw.get("schema_version") != schema:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.schema_version: mismatch")
    extra_keys = CURRENT_MAIN_REACTIVATION_CONTRACT_KEYS if current_main_reactivation else set()
    if set(raw) != TRANSITION_COMMON_KEYS | routing_keys | extra_keys:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}: keys mismatch")
    evidence_ref = require_non_empty_string(
        raw.get(evidence_key), f"{SUCCESSOR_TRANSITION_RECORD}.{evidence_key}"
    )
    if not evidence_ref.startswith(
        f"https://github.com/FrankQDWang/resume-ir/issues/{base_issue[1:]}#issuecomment-"
    ):
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.{evidence_key}: mismatch")
    return current_main_attribution


def validate_successor_scope_exception(
    active_slice: dict,
    *,
    historical_exception_required: bool,
) -> None:
    value = active_slice.get("scope_exception")
    if not isinstance(value, bool):
        fail("head.scope.active_slice.scope_exception: expected boolean")
    if historical_exception_required:
        require_bool(value, True, "head.scope.active_slice.scope_exception")


def self_test_transition_record_shapes() -> None:
    common = {key: None for key in TRANSITION_COMMON_KEYS}
    v1 = common | {
        "schema_version": "resume-ir.active-slice-transition.v1",
        "predecessor_terminal_evidence_ref": "https://github.com/FrankQDWang/resume-ir/issues/1#issuecomment-1",
    }
    v2 = common | {
        "schema_version": "resume-ir.active-slice-transition.v2",
        "routing_kind": "umbrella_to_bounded_execution_owner",
        "source_issue_role": "umbrella",
        "source_issue_remains_open": True,
        "source_issue_terminal_claim": False,
        "routing_evidence_ref": "https://github.com/FrankQDWang/resume-ir/issues/217#issuecomment-1",
    }
    validate_transition_record_shape(v1, "#1", "#2")
    validate_transition_record_shape(v2, "#217", "#270")
    for label, record, pair in [
        ("generic v2", v2, ("#217", "#271")),
        ("attribution v1", v1, ("#217", "#270")),
        ("false terminal claim", v2 | {"source_issue_terminal_claim": True}, ("#217", "#270")),
    ]:
        try:
            validate_transition_record_shape(record, *pair)
        except ValueError:
            continue
        fail(f"transition shape self-test accepted {label}")


def self_test_successor_scope_exception_contract() -> None:
    for value in (False, True):
        validate_successor_scope_exception(
            {"scope_exception": value},
            historical_exception_required=False,
        )

    for label, active_slice in (
        ("missing value", {}),
        ("string value", {"scope_exception": "true"}),
        ("integer value", {"scope_exception": 1}),
    ):
        try:
            validate_successor_scope_exception(
                active_slice,
                historical_exception_required=False,
            )
        except ValueError:
            continue
        fail(f"successor scope-exception self-test accepted {label}")

    for label in ("current-main attribution", "current-main reactivation"):
        validate_successor_scope_exception(
            {"scope_exception": True},
            historical_exception_required=True,
        )
        try:
            validate_successor_scope_exception(
                {"scope_exception": False},
                historical_exception_required=True,
            )
        except ValueError:
            continue
        fail(f"successor scope-exception self-test accepted false for {label}")


def validate_current_main_reactivation_contract(raw: dict, head_slice: dict) -> None:
    contract = raw.get("reactivation_contract")
    if not isinstance(contract, dict):
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract: expected object")
    expected_keys = {
        "pre_merge_phase",
        "post_merge_phase",
        "pre_merge_benchmark_or_profile_allowed",
        "post_merge_benchmark_or_profile_allowed",
        "reactivation_pr_paths",
        "reactivation_pr_paths_role",
        "reactivation_event_paths",
        "public_output_paths",
        "public_output_paths_role",
    }
    if set(contract) != expected_keys:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract: keys mismatch")
    if contract["pre_merge_phase"] != "contract_reconciliation":
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.pre_merge_phase: mismatch")
    if contract["post_merge_phase"] != "attribution_execution":
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.post_merge_phase: mismatch")
    if contract["pre_merge_benchmark_or_profile_allowed"] is not False:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.pre_merge_benchmark_or_profile_allowed: expected false")
    if contract["post_merge_benchmark_or_profile_allowed"] is not True:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.post_merge_benchmark_or_profile_allowed: expected true")

    if contract["reactivation_pr_paths_role"] != CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS_ROLE:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.reactivation_pr_paths_role: mismatch")
    allowed_paths = contract["reactivation_pr_paths"]
    if allowed_paths != sorted(CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS):
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.reactivation_pr_paths: mismatch")
    if head_slice.get("allowed_paths_role") != CURRENT_MAIN_ATTRIBUTION_ACTIVE_ALLOWED_PATHS_ROLE:
        fail("successor active slice allowed_paths must declare the reactivation PR path role")
    if head_slice.get("allowed_paths") != allowed_paths:
        fail("successor active slice allowed_paths must equal the reactivation PR path contract")

    event_paths = contract["reactivation_event_paths"]
    if event_paths != sorted(CURRENT_MAIN_ATTRIBUTION_REACTIVATION_EVENT_PATHS):
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.reactivation_event_paths: mismatch")

    public_output_paths = contract["public_output_paths"]
    if contract["public_output_paths_role"] != CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATHS_ROLE:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.public_output_paths_role: mismatch")
    if public_output_paths != CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATH_TEMPLATES:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.reactivation_contract.public_output_paths: mismatch")
    attribution = head_slice.get("attribution")
    if not isinstance(attribution, dict):
        fail("successor active slice attribution: expected table")
    execution = attribution.get("attribution_execution")
    if (
        not isinstance(execution, dict)
        or execution.get("public_output_paths") != public_output_paths
        or execution.get("public_output_paths_role") != CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATHS_ROLE
    ):
        fail("attribution execution public_output_paths must equal the reactivation contract")


def self_test_current_main_reactivation_contract() -> None:
    contract = {
        "pre_merge_phase": "contract_reconciliation",
        "post_merge_phase": "attribution_execution",
        "pre_merge_benchmark_or_profile_allowed": False,
        "post_merge_benchmark_or_profile_allowed": True,
        "reactivation_pr_paths": sorted(CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS),
        "reactivation_pr_paths_role": CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS_ROLE,
        "reactivation_event_paths": sorted(CURRENT_MAIN_ATTRIBUTION_REACTIVATION_EVENT_PATHS),
        "public_output_paths": CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATH_TEMPLATES.copy(),
        "public_output_paths_role": CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATHS_ROLE,
    }
    raw = {"reactivation_contract": contract}
    head_slice = {
        "allowed_paths_role": CURRENT_MAIN_ATTRIBUTION_ACTIVE_ALLOWED_PATHS_ROLE,
        "allowed_paths": contract["reactivation_pr_paths"].copy(),
        "attribution": {"attribution_execution": {
            "public_output_paths": contract["public_output_paths"].copy(),
            "public_output_paths_role": contract["public_output_paths_role"],
        }},
    }
    validate_current_main_reactivation_contract(raw, head_slice)
    for label, mutate in (
        ("extra allowed path", lambda item: item["reactivation_contract"]["reactivation_pr_paths"].append("extra")),
        ("missing allowed path", lambda item: item["reactivation_contract"]["reactivation_pr_paths"].pop()),
        ("extra reactivation event", lambda item: item["reactivation_contract"]["reactivation_event_paths"].append("extra")),
        ("missing reactivation event", lambda item: item["reactivation_contract"]["reactivation_event_paths"].pop()),
    ):
        mutated = deepcopy(raw)
        mutate(mutated)
        try:
            validate_current_main_reactivation_contract(mutated, head_slice)
        except ValueError:
            continue
        fail(f"reactivation contract self-test accepted {label}")


def classify_current_main_post_merge_evidence_path(path: str) -> str | None:
    if not isinstance(path, str):
        return None
    for kind, pattern in CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATH_PATTERNS:
        if pattern.fullmatch(path):
            return kind
    return None


def require_commit_sha(value: object, path: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{40}", value) is None:
        fail(f"{path}: expected lowercase 40-character commit SHA")
    return value


def git_path_exists(revision: str, path: str) -> bool:
    completed = subprocess.run(
        ["git", "cat-file", "-e", f"{revision}:{path}"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return completed.returncode == 0


def validate_public_perf_run_content(rel_path: str, raw: bytes) -> None:
    checker_path = ROOT / "scripts/ci/check-private-evidence-redaction.py"
    spec = importlib.util.spec_from_file_location("private_evidence_redaction", checker_path)
    if spec is None or spec.loader is None:
        fail("unable to load public perf evidence checker")
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    checker.validate_perf_run_json_content(rel_path, raw)


def validate_current_main_post_merge_authorization(
    event: dict,
    merge_base: str,
    base_state: dict,
) -> None:
    edge = event.get("transition")
    if edge != {
        "name": CURRENT_MAIN_ATTRIBUTION_AUTHORIZATION_EVENT_NAME,
        "from": "contract_conflict",
        "to": "goal_authorized",
    }:
        fail("first post-merge authorization must be contract_conflict -> goal_authorized")
    if base_state.get("workflow_state") != "contract_conflict":
        fail("first post-merge authorization requires a contract_conflict base snapshot")
    if base_state.get("evidence_lane") != "w0_docs":
        fail("first post-merge authorization requires the w0_docs base lane")
    ledger = base_state.get("github_ledger")
    if not isinstance(ledger, dict) or ledger.get("primary_issue") != "#270":
        fail("first post-merge authorization requires the #270 base ledger")
    observation = event.get("observation")
    evidence = event.get("evidence")
    if not isinstance(observation, dict) or not isinstance(evidence, dict):
        fail("post-merge authorization requires observation and evidence objects")
    if set(evidence) != set(CURRENT_MAIN_ATTRIBUTION_POST_MERGE_AUTHORIZATION_EVIDENCE):
        fail("post-merge authorization evidence set is incomplete or expanded")
    require(observation.get("active_prs") == [], "post-merge authorization observation active_prs must be empty")
    require(observation.get("active_prs_empty") is True, "post-merge authorization requires active_prs_empty=true")
    require(observation.get("base_snapshot_workflow_state") == "contract_conflict",
            "post-merge authorization observation base state mismatch")
    require(observation.get("reactivation_pr_merged") is True,
            "post-merge authorization requires merged reactivation evidence")
    require(observation.get("merged_reactivation_pr") == CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR,
            "post-merge authorization requires the #278 identity")
    live_sha = require_commit_sha(observation.get("live_main_sha"), "observation.live_main_sha")
    if live_sha != merge_base:
        fail("post-merge authorization observation SHA must equal the PR merge base")
    if observation.get("post_merge_base_sha") != merge_base:
        fail("post-merge authorization post_merge_base_sha must equal the PR merge base")
    merged_head_sha = require_commit_sha(
        observation.get("merged_reactivation_head_sha"),
        "observation.merged_reactivation_head_sha",
    )
    git(["merge-base", "--is-ancestor", merged_head_sha, merge_base])
    require(evidence.get("owner_issue_270_open") == "true", "post-merge authorization owner evidence mismatch")
    require(evidence.get("fresh_remote_main_sha") == merge_base,
            "post-merge authorization fresh_remote_main_sha must equal the PR merge base")
    require(evidence.get("post_merge_base_sha") == merge_base,
            "post-merge authorization evidence base SHA mismatch")
    require(evidence.get("clean_main_equals_remote_main") == "true_at_merged_base_observation",
            "post-merge authorization clean-main evidence mismatch")
    require(evidence.get("reactivation_pr_merged") is True,
            "post-merge authorization merged PR evidence mismatch")
    require(evidence.get("merged_reactivation_pr") == CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR,
            "post-merge authorization merged PR identity mismatch")
    require(evidence.get("merged_reactivation_head_sha") == merged_head_sha,
            "post-merge authorization merged head SHA mismatch")
    require(evidence.get("base_snapshot_workflow_state") == "contract_conflict",
            "post-merge authorization base-state evidence mismatch")
    require(evidence.get("active_prs_empty") is True,
            "post-merge authorization active-pr evidence mismatch")
    require(evidence.get("configured_private_roots") == "configured_private_roots_readable",
            "post-merge authorization private-root evidence mismatch")
    require(isinstance(evidence.get("runtime_capability_attestation"), str)
            and evidence["runtime_capability_attestation"].startswith("macOS"),
            "post-merge authorization runtime evidence must identify macOS")
    require(evidence.get("primary_lane_full_import_ocr_backlog") == "true",
            "post-merge authorization primary lane evidence mismatch")
    require(evidence.get("phase_attribution_execution") == "authorized_by_current_goal_contract",
            "post-merge authorization phase evidence mismatch")
    require(evidence.get("privacy_boundary") == "bounded_redacted_aggregates_only; no raw/private artifacts",
            "post-merge authorization privacy evidence mismatch")


def validate_current_main_same_issue_evidence_scope(
    head_slice: dict,
    changed: set[str],
    status_by_path: dict[str, str] | None = None,
    merge_base: str | None = None,
    base_state: dict | None = None,
    head_goal: dict | None = None,
    validate_event_chain: bool = True,
    synthetic_head_files: dict[str, bytes] | None = None,
) -> None:
    if not changed:
        fail("same-issue #270 evidence changes require at least one bounded public-output path")
    status_by_path = status_by_path or {path: "M" for path in changed}
    if set(status_by_path) != changed:
        fail("same-issue #270 evidence name-status paths do not match changed paths")
    if merge_base is None:
        merge_base = "HEAD"
    if base_state is None:
        base_state = {"workflow_state": "goal_authorized", "state_version": 565}
    if head_slice.get("allowed_paths_role") != CURRENT_MAIN_ATTRIBUTION_ACTIVE_ALLOWED_PATHS_ROLE:
        fail("same-issue #270 evidence requires the reactivation PR path role")
    if set(head_slice.get("allowed_paths", [])) != CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS:
        fail("same-issue #270 evidence cannot expand or replace reactivation PR allowed_paths")
    require_bool(
        head_slice.get("production_code_allowed"),
        False,
        "head.scope.active_slice.production_code_allowed",
    )
    require_bool(
        head_slice.get("private_benchmark_allowed"),
        False,
        "head.scope.active_slice.private_benchmark_allowed",
    )
    attribution = head_slice.get("attribution")
    if not isinstance(attribution, dict) or attribution.get("milestones") != CURRENT_MAIN_ATTRIBUTION_MILESTONES:
        fail("same-issue #270 evidence must preserve the ordered milestone contract")
    execution = attribution.get("attribution_execution")
    if not isinstance(execution, dict):
        fail("same-issue #270 evidence requires attribution execution contract")
    for key, expected in {
        "evidence_lane": "w1_private",
        "benchmark_or_profile_execution_allowed": True,
        "production_code_allowed": False,
        "execution_boundary": "post_merge_only",
        "requires_fresh_merged_main": True,
        "requires_executable_provenance": True,
        "required_runtime_provenance": CURRENT_MAIN_ATTRIBUTION_REQUIRED_RUNTIME_PROVENANCE,
        "authorization_transition": "authorize_current_main_import_attribution",
        "missing_roots_transition": "block_current_main_import_attribution_missing_roots",
        "public_output_paths": CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATH_TEMPLATES,
        "public_output_paths_role": CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATHS_ROLE,
    }.items():
        if execution.get(key) != expected:
            fail(f"same-issue #270 evidence contract {key} mismatch")
    current_state_path = "perf/current-loop-state.json"
    if status_by_path.get(current_state_path) != "M":
        fail("same-issue #270 evidence must modify the derived current state")
    event_paths = [
        path for path in changed
        if classify_current_main_post_merge_evidence_path(path) == "append_only_event"
    ]
    if len(event_paths) != 1:
        fail("same-issue #270 evidence requires exactly one new append-only event")
    redacted_paths = [
        path for path in changed
        if classify_current_main_post_merge_evidence_path(path) == "redacted_aggregate"
    ]
    for path in sorted(changed):
        kind = classify_current_main_post_merge_evidence_path(path)
        if kind is None:
            fail(
                "same-issue #270 evidence path is outside bounded public-output templates: "
                f"{path!r}"
            )
        if kind == "append_only_event" and status_by_path[path] != "A":
            fail(f"same-issue #270 event path must be newly added (A), found {status_by_path[path]!r}: {path!r}")
        if kind == "redacted_aggregate":
            if status_by_path[path] != "A":
                fail(f"same-issue #270 redacted aggregate must be newly added (A): {path!r}")
            raw = synthetic_head_files[path] if synthetic_head_files and path in synthetic_head_files else (ROOT / path).read_bytes()
            validate_public_perf_run_content(path, raw)
    if status_by_path.get(current_state_path) != "M":
        fail("same-issue #270 current state must be modified (M)")
    event_path = event_paths[0]
    event_raw = (
        synthetic_head_files[event_path]
        if synthetic_head_files and event_path in synthetic_head_files
        else (ROOT / event_path).read_bytes()
    )
    validate_public_perf_run_content(event_path, event_raw)
    event = json.loads(event_raw)
    if not isinstance(event, dict):
        fail(f"{event_path}: event must be an object")
    if base_state.get("workflow_state") == "contract_conflict":
        if set(changed) != {current_state_path, event_path}:
            fail("first post-merge authorization allows only current state plus one new event")
        validate_current_main_post_merge_authorization(event, merge_base, base_state)
    if not isinstance(base_state.get("state_version"), int):
        fail("same-issue #270 base snapshot state_version is required")
    try:
        event_version = int(pathlib.PurePosixPath(event_path).stem)
    except ValueError:
        fail(f"{event_path}: event filename must be numeric state version")
    if event_version != base_state["state_version"] + 1:
        fail("same-issue #270 event is not the next state version")
    if event.get("state_version") != event_version or event.get("expected_state_version") != base_state["state_version"]:
        fail("same-issue #270 event CAS does not match the base snapshot")
    if validate_event_chain:
        if merge_base is None:
            fail("same-issue #270 event chain requires a real merge base")
        previous_path = str(pathlib.PurePosixPath(event_path).with_name(f"{event_version - 1}.json"))
        if not git_path_exists(merge_base, previous_path):
            fail(f"{previous_path}@{merge_base}: previous event is missing")
        if git_path_exists(merge_base, event_path):
            fail(f"{event_path}: event path already exists in the merge base")
        previous_hash = hashlib.sha256(git_raw(["show", f"{merge_base}:{previous_path}"])).hexdigest()
        reducer_path = ROOT / "scripts/loop/reduce-current-loop-state.py"
        spec = importlib.util.spec_from_file_location("current_loop_reducer", reducer_path)
        if spec is None or spec.loader is None:
            fail("unable to load current loop reducer")
        reducer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(reducer)
        if head_goal is None:
            fail("same-issue #270 event chain requires the head goal")
        reducer.validate(
            (ROOT / event_path, event, event_raw),
            previous_hash,
            base_state["state_version"],
            head_goal,
            reducer.contracts(head_goal),
            "#270",
            head_goal["scope"]["active_slice"]["attribution"]["primary_benchmark_lane"],
            head_goal["scope"]["active_slice"]["unconfigured_private_run_terminal"],
        )


def validate_declared_successor_transition(
    base_goal: dict,
    head_goal: dict,
    merge_base: str,
    changed: set[str],
) -> None:
    raw = load_json(ROOT / SUCCESSOR_TRANSITION_RECORD)
    if not isinstance(raw, dict):
        fail(f"{SUCCESSOR_TRANSITION_RECORD}: expected object")
    base_slice = base_goal.get("scope", {}).get("active_slice", {})
    head_slice = head_goal.get("scope", {}).get("active_slice", {})
    base_issue = require_issue_ref(base_slice.get("issue"), "base.scope.active_slice.issue")
    head_issue = require_issue_ref(head_slice.get("issue"), "head.scope.active_slice.issue")
    current_main_attribution = validate_transition_record_shape(raw, base_issue, head_issue)
    current_main_reactivation = (base_issue, head_issue) == CURRENT_MAIN_ATTRIBUTION_REACTIVATION
    if current_main_reactivation:
        validate_current_main_reactivation_contract(raw, head_slice)
    require_non_empty_string(raw.get("transition_id"), f"{SUCCESSOR_TRANSITION_RECORD}.transition_id")
    if raw.get("from_issue") != base_issue or raw.get("to_issue") != head_issue:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}: issue transition mismatch")
    expected_successor_ref = f"https://github.com/FrankQDWang/resume-ir/issues/{head_issue[1:]}"
    if raw.get("successor_issue_ref") != expected_successor_ref:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.successor_issue_ref: mismatch")
    base_goal_source = subprocess.check_output(
        ["git", "show", f"{merge_base}:ACTIVE_GOAL.toml"], cwd=ROOT
    )
    if raw.get("base_active_goal_sha256") != source_sha256(base_goal_source):
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.base_active_goal_sha256: mismatch")
    expected_state_paths = (
        CURRENT_MAIN_ATTRIBUTION_STATE_PATHS
        if current_main_attribution
        else SUCCESSOR_TRANSITION_PATHS
    )
    state_paths = raw.get("state_paths")
    if not isinstance(state_paths, list) or state_paths != sorted(expected_state_paths):
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.state_paths: mismatch")
    require_bool(raw.get("production_code_changed"), False, f"{SUCCESSOR_TRANSITION_RECORD}.production_code_changed")
    require_bool(raw.get("blind_holdout_reread"), False, f"{SUCCESSOR_TRANSITION_RECORD}.blind_holdout_reread")
    require_bool(raw.get("gate_weakening"), False, f"{SUCCESSOR_TRANSITION_RECORD}.gate_weakening")
    privacy = raw.get("privacy")
    if not isinstance(privacy, dict) or set(privacy) != {
        "contains_raw_resume_text",
        "contains_raw_query_text",
        "contains_candidate_results",
        "contains_local_paths",
        "contains_tokens",
        "contains_diagnostics_package",
    }:
        fail(f"{SUCCESSOR_TRANSITION_RECORD}.privacy: shape mismatch")
    for key, value in privacy.items():
        require_bool(value, False, f"{SUCCESSOR_TRANSITION_RECORD}.privacy.{key}")
    require_bool(base_slice.get("contract_change_allowed"), True, "base.scope.active_slice.contract_change_allowed")
    validate_successor_scope_exception(
        head_slice,
        historical_exception_required=(
            current_main_attribution or current_main_reactivation
        ),
    )
    allowed_paths = head_slice.get("allowed_paths")
    if not isinstance(allowed_paths, list) or not all(isinstance(path, str) and path for path in allowed_paths):
        fail("successor active slice requires non-empty allowed_paths")
    if len(set(allowed_paths)) != len(allowed_paths):
        fail("successor active slice allowed_paths must be unique")
    if current_main_attribution and set(allowed_paths) != CURRENT_MAIN_ATTRIBUTION_FINAL_PATHS:
        fail("successor active slice allowed_paths mismatch")
    if current_main_reactivation and set(allowed_paths) != CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS:
        fail("current-main reactivation allowed_paths mismatch")
    bootstrap = raw.get("bootstrap_gate_change")
    require_bool(bootstrap, bootstrap is True, f"{SUCCESSOR_TRANSITION_RECORD}.bootstrap_gate_change")
    if bootstrap:
        if current_main_attribution:
            if frozenset(changed) not in {
                frozenset(CURRENT_MAIN_ATTRIBUTION_PRE_PR_PATHS),
                frozenset(CURRENT_MAIN_ATTRIBUTION_FINAL_PATHS),
            }:
                fail("current-main attribution transition path mismatch")
        elif (base_issue, head_issue) != ("#170", "#173"):
            fail("successor transition bootstrap is restricted to #170 -> #173 or #217 -> #270")
        elif changed != SUCCESSOR_TRANSITION_BOOTSTRAP_PATHS:
            fail(
                "successor transition bootstrap path mismatch: expected "
                f"{sorted(SUCCESSOR_TRANSITION_BOOTSTRAP_PATHS)!r}, found {sorted(changed)!r}"
            )
    elif current_main_reactivation:
        expected = CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS
        if changed != expected:
            fail(
                "current-main attribution reactivation path mismatch: expected "
                f"{sorted(expected)!r}, found {sorted(changed)!r}"
            )
    elif changed != SUCCESSOR_TRANSITION_PATHS:
        fail(
            "successor transition path mismatch: expected "
            f"{sorted(SUCCESSOR_TRANSITION_PATHS)!r}, found {sorted(changed)!r}"
        )


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


def validate_transition_scope(
    base_goal: dict,
    head_goal: dict,
    merge_base: str,
    changed: set[str],
    status_by_path: dict[str, str] | None = None,
    base_state_override: dict | None = None,
    synthetic_head_files: dict[str, bytes] | None = None,
    validate_event_chain: bool = True,
) -> None:
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
            expected_names = (
                "make_ocr_terminal_failure_recoverable",
                "bind_ocr_attempt_to_persisted_generation",
            )
            if (base_name, head_name) != expected_names:
                fail(
                    "same-issue #159 changes require the exact terminal-OCR-lifecycle -> "
                    "generation-bound-attempt slice transition"
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
        if head_issue == "#270":
            base_state = base_state_override
            if base_state is None:
                base_state, _ = load_json_at_revision(merge_base, "perf/current-loop-state.json")
            validate_current_main_same_issue_evidence_scope(
                head_slice,
                changed,
                status_by_path,
                merge_base,
                base_state,
                head_goal,
                validate_event_chain,
                synthetic_head_files,
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

    if (base_issue, head_issue) == ("#159", "#165"):
        require_bool(head_slice.get("production_code_allowed"), True, "head.scope.active_slice.production_code_allowed")
        require_bool(head_slice.get("private_benchmark_allowed"), True, "head.scope.active_slice.private_benchmark_allowed")
        require_bool(head_slice.get("scope_exception"), False, "head.scope.active_slice.scope_exception")
        allowed_paths = head_slice.get("allowed_paths")
        if not isinstance(allowed_paths, list) or not all(isinstance(path, str) and path for path in allowed_paths):
            fail("#159 -> #165 requires non-empty allowed_paths")
        expected_paths = set(allowed_paths)
        if len(expected_paths) != len(allowed_paths) or changed != expected_paths:
            fail(
                "#159 -> #165 path mismatch: expected exact ACTIVE_GOAL allowed_paths "
                f"{sorted(expected_paths)!r}, found {sorted(changed)!r}"
            )
        return

    if (base_issue, head_issue) == ("#165", "#170"):
        targets = base_slice.get("allowed_contract_transition_targets")
        if not isinstance(targets, list) or "#170" not in targets:
            fail("#165 contract does not authorize transition to #170")
        require_bool(head_slice.get("production_code_allowed"), False, "head.scope.active_slice.production_code_allowed")
        require_bool(head_slice.get("private_benchmark_allowed"), True, "head.scope.active_slice.private_benchmark_allowed")
        require_bool(head_slice.get("scope_exception"), False, "head.scope.active_slice.scope_exception")
        allowed_paths = head_slice.get("allowed_paths")
        if not isinstance(allowed_paths, list) or not all(isinstance(path, str) and path for path in allowed_paths):
            fail("#165 -> #170 requires non-empty allowed_paths")
        expected_paths = set(allowed_paths)
        if len(expected_paths) != len(allowed_paths) or expected_paths != BLIND_HOLDOUT_ACCEPTANCE_PATHS:
            fail(
                "#165 -> #170 ACTIVE_GOAL allowed_paths mismatch: expected "
                f"{sorted(BLIND_HOLDOUT_ACCEPTANCE_PATHS)!r}, found {sorted(expected_paths)!r}"
            )
        if changed != BLIND_HOLDOUT_ACCEPTANCE_PATHS:
            fail(
                "#165 -> #170 path mismatch: expected "
                f"{sorted(BLIND_HOLDOUT_ACCEPTANCE_PATHS)!r}, found {sorted(changed)!r}"
            )
        return

    if SUCCESSOR_TRANSITION_RECORD in changed:
        validate_declared_successor_transition(base_goal, head_goal, merge_base, changed)
        return

    fail(f"unauthorized active-slice transition: {base_issue!r} -> {head_issue!r}")


def self_test_current_main_same_issue_evidence_scope() -> None:
    base_goal = {"scope": {"active_slice": {"issue": "#270"}}}
    head_goal = {
        "scope": {"active_slice": {
            "issue": "#270",
            "allowed_paths_role": CURRENT_MAIN_ATTRIBUTION_ACTIVE_ALLOWED_PATHS_ROLE,
            "allowed_paths": sorted(CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR_PATHS),
            "production_code_allowed": False,
            "private_benchmark_allowed": False,
            "attribution": {
                "milestones": CURRENT_MAIN_ATTRIBUTION_MILESTONES.copy(),
                "attribution_execution": {
                    "evidence_lane": "w1_private",
                    "benchmark_or_profile_execution_allowed": True,
                    "production_code_allowed": False,
                    "execution_boundary": "post_merge_only",
                    "requires_fresh_merged_main": True,
                    "requires_executable_provenance": True,
                    "required_runtime_provenance": CURRENT_MAIN_ATTRIBUTION_REQUIRED_RUNTIME_PROVENANCE.copy(),
                    "authorization_transition": "authorize_current_main_import_attribution",
                    "missing_roots_transition": "block_current_main_import_attribution_missing_roots",
                    "public_output_paths": CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATH_TEMPLATES.copy(),
                    "public_output_paths_role": CURRENT_MAIN_ATTRIBUTION_POST_MERGE_EVIDENCE_PATHS_ROLE,
                },
            },
        }},
    }
    event_path = "perf/runs/run-2026-07/events/566.json"
    aggregate_path = "perf/runs/run-2026-07/redacted/aggregate.json"
    current_state_path = "perf/current-loop-state.json"
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
        "transition": {
            "name": "capture_synthetic_smoke_baseline",
            "from": "goal_authorized",
            "to": "baseline_captured",
        },
        "result": "passed",
        "evidence_refs": ["https://github.com/FrankQDWang/resume-ir/issues/270"],
        "observation": {"owner_issue": "#270", "active_prs": []},
        "verification": [],
        "privacy": {key: False for key in CURRENT_MAIN_ATTRIBUTION_PUBLIC_PRIVACY_FLAGS},
    }
    valid_aggregate = {
        "schema_version": "resume-ir.redacted-aggregate.v1",
        "privacy": {key: False for key in CURRENT_MAIN_ATTRIBUTION_PUBLIC_PRIVACY_FLAGS},
        "aggregate": {
            "label": "import_attribution",
            "counts": {"documents": 1},
            "durations_ms": {"import": 1},
            "bytes": {"archive": 1},
        },
    }
    event_raw = json.dumps(valid_event, sort_keys=True).encode()
    aggregate_raw = json.dumps(valid_aggregate, sort_keys=True).encode()
    merged_sha = git(["rev-parse", "HEAD"])
    valid_paths = {current_state_path, event_path, aggregate_path}
    valid_status = {current_state_path: "M", event_path: "A", aggregate_path: "A"}
    synthetic_files = {event_path: event_raw, aggregate_path: aggregate_raw}
    baseline_state = {
        "workflow_state": "goal_authorized",
        "state_version": 565,
        "github_ledger": {"primary_issue": "#270", "active_prs": []},
    }
    validate_transition_scope(
        base_goal,
        head_goal,
        merged_sha,
        valid_paths,
        valid_status,
        baseline_state,
        synthetic_files,
        False,
    )
    for path, expected_kind in (
        ("PROGRESS.md", "progress"),
        (current_state_path, "derived_state"),
        (event_path, "append_only_event"),
        (aggregate_path, "redacted_aggregate"),
    ):
        if classify_current_main_post_merge_evidence_path(path) != expected_kind:
            fail(f"same-issue #270 evidence classifier misclassified {path!r}")

    for label, bad_path in (
        ("production path", "crates/meta-store/src/lib.rs"),
        ("arbitrary docs", "docs/unrelated.md"),
        ("checker", "scripts/ci/check-gate-integrity.py"),
        ("active goal", "ACTIVE_GOAL.toml"),
        ("transition contract", SUCCESSOR_TRANSITION_RECORD),
        ("malformed event", "perf/runs/run-2026-07/events/not-state.json"),
        ("nested redacted path", "perf/runs/run-2026-07/redacted/private/profile.json"),
    ):
        changed = valid_paths | {bad_path}
        statuses = valid_status | {bad_path: "A"}
        try:
            validate_transition_scope(
                base_goal,
                head_goal,
                merged_sha,
                changed,
                statuses,
                baseline_state,
                synthetic_files,
                False,
            )
        except ValueError:
            continue
        fail(f"same-issue #270 evidence self-test accepted {label}")

    for label, event_status in (("existing event modified", "M"), ("existing event deleted", "D")):
        statuses = valid_status | {event_path: event_status}
        try:
            validate_transition_scope(
                base_goal,
                head_goal,
                merged_sha,
                valid_paths,
                statuses,
                baseline_state,
                synthetic_files,
                False,
            )
        except ValueError:
            continue
        fail(f"same-issue #270 evidence self-test accepted {label}")

    auth_event_path = "perf/runs/run-2026-07/events/566.json"
    auth_evidence = {
        "owner_issue_270_open": "true",
        "fresh_remote_main_sha": merged_sha,
        "clean_main_equals_remote_main": "true_at_merged_base_observation",
        "configured_private_roots": "configured_private_roots_readable",
        "runtime_capability_attestation": "macOS_arm64; runtime capabilities attested",
        "primary_lane_full_import_ocr_backlog": "true",
        "phase_attribution_execution": "authorized_by_current_goal_contract",
        "privacy_boundary": "bounded_redacted_aggregates_only; no raw/private artifacts",
        "reactivation_pr_merged": True,
        "merged_reactivation_pr": CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR,
        "merged_reactivation_head_sha": merged_sha,
        "post_merge_base_sha": merged_sha,
        "base_snapshot_workflow_state": "contract_conflict",
        "active_prs_empty": True,
    }
    auth_event = deepcopy(valid_event)
    auth_event.update({
        "transition": {
            "name": CURRENT_MAIN_ATTRIBUTION_AUTHORIZATION_EVENT_NAME,
            "from": "contract_conflict",
            "to": "goal_authorized",
        },
        "evidence": auth_evidence,
        "observation": {
            "owner_issue": "#270",
            "primary_benchmark_lane": "full_import_ocr_backlog",
            "private_input_capability": "configured_private_roots",
            "current_slice": "#270 current-main installed-equivalent import attribution",
            "active_prs": [],
            "active_prs_empty": True,
            "transition_evidence_ref": "https://github.com/FrankQDWang/resume-ir/pull/278",
            "base_snapshot_workflow_state": "contract_conflict",
            "reactivation_pr_merged": True,
            "merged_reactivation_pr": CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR,
            "merged_reactivation_head_sha": merged_sha,
            "post_merge_base_sha": merged_sha,
            "live_main_sha": merged_sha,
            "runtime_capability_attestation": "macOS_arm64; runtime capabilities attested",
        },
    })
    auth_raw = json.dumps(auth_event, sort_keys=True).encode()
    auth_paths = {current_state_path, auth_event_path}
    auth_status = {current_state_path: "M", auth_event_path: "A"}
    auth_state = {
        "workflow_state": "contract_conflict",
        "evidence_lane": "w0_docs",
        "state_version": 565,
        "github_ledger": {"primary_issue": "#270", "active_prs": [CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR]},
    }
    auth_files = {auth_event_path: auth_raw}
    validate_transition_scope(
        base_goal,
        head_goal,
        merged_sha,
        auth_paths,
        auth_status,
        auth_state,
        auth_files,
        False,
    )
    for label, mutate in (
        ("stale main SHA", lambda item: item["observation"].update(live_main_sha="c4bdd1a98aeaac41b68765f0a4d96fefe86620ab")),
        ("active reactivation PR", lambda item: item["observation"].update(active_prs=[CURRENT_MAIN_ATTRIBUTION_REACTIVATION_PR], active_prs_empty=False)),
        ("missing merged identity", lambda item: item["observation"].pop("merged_reactivation_pr", None)),
    ):
        altered = deepcopy(auth_event)
        mutate(altered)
        altered_files = {auth_event_path: json.dumps(altered, sort_keys=True).encode()}
        try:
            validate_transition_scope(
                base_goal,
                head_goal,
                merged_sha,
                auth_paths,
                auth_status,
                auth_state,
                altered_files,
                False,
            )
        except ValueError:
            continue
        fail(f"same-issue #270 authorization self-test accepted {label}")

    for label, extra_path in (
        ("authorization PROGRESS", "PROGRESS.md"),
        ("authorization redacted aggregate", aggregate_path),
    ):
        changed = auth_paths | {extra_path}
        statuses = auth_status | {extra_path: "A"}
        files = auth_files | ({extra_path: aggregate_raw} if extra_path == aggregate_path else {})
        try:
            validate_transition_scope(
                base_goal,
                head_goal,
                merged_sha,
                changed,
                statuses,
                auth_state,
                files,
                False,
            )
        except ValueError:
            continue
        fail(f"same-issue #270 authorization self-test accepted {label}")


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
    if path == SUCCESSOR_TRANSITION_RECORD:
        return True
    return False


def main() -> int:
    self_test_transition_record_shapes()
    self_test_successor_scope_exception_contract()
    self_test_current_main_reactivation_contract()
    self_test_current_main_same_issue_evidence_scope()
    if sys.argv[1:]:
        if sys.argv[1:] == ["--self-test"]:
            print("check-gate-integrity.py self-test passed")
            return 0
        fail("usage: check-gate-integrity.py [--self-test]")
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
    merge_base, status_by_path = merge_base_and_changed_paths()
    paths = set(status_by_path)
    base_goal = load_toml_at_revision(merge_base, "ACTIVE_GOAL.toml")
    validate_transition_scope(base_goal, active_goal, merge_base, paths, status_by_path)
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
