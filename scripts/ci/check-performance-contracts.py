#!/usr/bin/env python3
"""Validate public performance-goal contracts and schema fixtures.

This is intentionally standard-library only. It is a CI guard for public
contract files, not a replacement for a full JSON Schema implementation.
"""

from __future__ import annotations

import json
import hashlib
import pathlib
import runpy
import subprocess
import sys
import tomllib
from collections.abc import Mapping


ROOT = pathlib.Path(__file__).resolve().parents[2]
PERF = ROOT / "perf"
VALID_FIXTURES = PERF / "fixtures" / "valid"
INVALID_FIXTURES = PERF / "fixtures" / "invalid"
HEX64 = set("0123456789abcdef")
REQUIRED_BUCKETS = [
    "single_term",
    "and_2",
    "and_3_5",
    "and_6_16",
    "field_filter",
    "hybrid",
    "semantic",
]
PRIVACY_FALSE_FIELDS = [
    "contains_raw_resume_text",
    "contains_raw_query_text",
    "contains_candidate_results",
    "contains_local_paths",
    "contains_tokens",
    "contains_diagnostics_package",
]
OPTIMIZATION_LAYERS = ["L1", "L2", "L3", "L4"]
PLATFORM_LANES = [
    "macos_m4_discovery",
    "windows_weak_host_validation",
    "cross_os_ci_smoke",
]
GUI_DEFAULT_STACK = "tauri_react_vite_tailwind_typescript"
GUI_REFERENCE_ROLE = "visual_baseline_not_functional_clone"
SCALE_GATES = {
    "D10K_private_calibration",
    "D100K_weak_host",
    "D1M_scale",
}
ZERO_SHA256 = "0" * 64
CURRENT_LOOP_CONTRACT_FILES = {
    "active_goal_sha256": "ACTIVE_GOAL.toml",
    "acceptance_matrix_sha256": "perf/acceptance-matrix.toml",
    "loop_state_schema_sha256": "perf/loop-state.schema.json",
    "experiment_report_schema_sha256": "perf/experiment-report.schema.json",
    "synthetic_smoke_artifact_manifest_schema_sha256": "perf/synthetic-smoke-artifact-manifest.schema.json",
}
REMOVED_LOOP_POLICY_FIELDS = {
    "allowed_paths",
    "event_log",
    "goal_prompt",
    "runner_recovery",
    "runtime_capability_attestation",
    "transition_graph",
}
SYNTHETIC_SMOKE_COMPONENTS = {
    "synthetic_query",
    "ocr_throughput",
    "vector_quality",
    "private_query_runner",
}
SYNTHETIC_SMOKE_BATCH_PROTOCOL_STAGES = {
    "query_parse",
    "prefilter",
    "bm25",
    "ann",
    "fusion",
    "bulk_hydrate",
    "snippet",
    "elapsed",
}
MIXED_IMPORT_BENCHMARK_LAYERS = [
    "public_synthetic",
    "private_calibration",
    "blind_holdout",
]
MIXED_IMPORT_STATUSES = [
    "resume_candidate",
    "non_resume",
    "needs_review",
    "ocr_backlog",
    "failed",
]
MIXED_IMPORT_REQUIRED_METRICS = [
    "sample_counts",
    "label_counts",
    "extension_buckets",
    "size_buckets",
    "directory_depth_buckets",
    "classification_counts",
    "searchable_count",
    "indexed_resume_precision",
    "contamination_count",
    "resume_completeness",
    "clean_vs_mixed_wall_time",
    "stage_timings",
    "content_bytes_read",
    "docs_per_second",
    "mib_per_second",
    "h_tier",
    "resource_budget",
    "privacy_confirmation",
]
MIXED_IMPORT_FORBIDDEN_SIGNALS = [
    "path_whitelist",
    "directory_name_whitelist",
    "filename_whitelist",
    "raw_file_hash_classifier_key",
    "benchmark_mutation_to_fit_rules",
]
SYNTHETIC_SMOKE_TOP_LEVEL_KEYS = {
    "schema_version",
    "goal_id",
    "report_kind",
    "claim",
    "evidence_lane",
    "contract_pins",
    "synthetic_smoke",
    "thresholds",
    "privacy",
}
SYNTHETIC_SMOKE_KEYS = {
    "smoke_schema_version",
    "source",
    "benchmark_command",
    "document_count",
    "query_count",
    "top_k",
    "percentile_confidence",
    "batch_protocol_request_count",
    "component_reports",
    "harness_observations",
    "latency_ms",
    "resource_observations",
    "quality",
}

FORWARD_MIGRATION_FEATURE_TRAIN_IDENTITY = {
    "contract": "resume-ir.forward-migration-feature-train.v1",
    "schema_migration": "continuous_encrypted_cow_from_v29",
    "train_base_schema": 29,
    "train_final_schema": 33,
    "pre_v29_runtime_migration_allowed": False,
    "future_schema_read_allowed": False,
    "migration_dual_reader_allowed": False,
    "migration_dual_write_allowed": False,
    "migration_registry_contiguous_required": True,
    "migration_source_ciphertext_may_change": False,
    "migration_predecessor_retention_count": 1,
    "product_version_source": "apps/desktop/package.json",
    "feature_versions": ["0.1.3", "0.1.4", "0.1.5", "0.1.6", "0.1.7", "0.1.8"],
    "feature_schemas": [30, 31, 32, 33, 33, 33],
    "per_feature_installed_acceptance_required": True,
    "final_full_matrix_after_version": "0.1.8",
    "final_soak_minutes": 120,
    "existing_v29_key_repair_allowed": False,
    "unsupported_store_bytes_may_change": False,
    "fresh_current_store_requires_no_legacy_authority": True,
}

FEATURE_TRAIN_PUBLICATION_RETIREMENT = {
    "failed_publication_artifact_retirement": "exact_generation_or_terminal_block",
    "failed_publication_artifact_accumulation_allowed": False,
    "artifact_retirement_failure_action": "repair_required_without_next_attempt",
    "publication_authority": (
        "typed_current_head_or_exact_migration_attempt_or_exact_artifact_attempt"
    ),
    "publication_non_applied_cleanup": (
        "durable_retirement_intent_and_abandoned_then_exact_generation_retired"
    ),
    "publication_non_applied_outcomes": ["cancelled", "error", "superseded"],
    "publication_cleanup_order": [
        "typed_authority_retirement_intent_and_abandoned_committed",
        "exact_artifacts_retired_with_durable_completion_markers",
        "retirement_failure_atomically_settles_attempt_and_blocks_exact_head",
    ],
    "publication_retirement_artifacts": [
        "fulltext_snapshot",
        "fulltext_generation_pin",
        "fulltext_staging",
        "vector_snapshot",
        "vector_generation_pin",
        "vector_staging",
    ],
    "publication_non_applied_cas_preserves_prior_generation": True,
    "publication_committed_fence_outcome": "applied_only",
    "publication_cleanup_must_finish_before_next_attempt": True,
    "publication_cleanup_block_identity": (
        "persisted_typed_authority_not_base_generation_epoch_only"
    ),
    "publication_interrupted_restart_cleanup": (
        "bounded_pending_intent_replay_before_new_attempt_and_obsolete_gc"
    ),
    "publication_attempt_terminal_settlement_with_head_block_atomic": True,
    "publication_retirement_replay_limit": 64,
    "publication_retirement_overflow_action": (
        "reopen_and_new_publication_attempts_fail_closed"
    ),
    "publication_pending_replay_precedes_repair_blocked_return": True,
    "publication_pending_retirement_prunable": False,
    "publication_retirement_completion": (
        "transaction_authorized_exact_artifact_cas_only"
    ),
    "publication_retirement_row_direct_delete_or_replace_allowed": False,
    "publication_absent_artifact_replay": (
        "exact_pending_intent_and_not_retained_only"
    ),
    "publication_terminal_cleanup_attempt_preserved": True,
    "publication_supersession_may_accumulate_artifacts": False,
    "publication_prepared_owner_boundary": "internal_transaction_closure_only",
    "publication_prepared_plain_drop_allowed": False,
}

FEATURE_TRAIN_INSTALLED_ACCEPTANCE = {
    "macos_source_commit_provenance_required": True,
    "macos_v2_trust_lane_system_tools": "absolute_path_closed_env_shell_false",
    "macos_dmg_verified_consumption": "single_mount_lease",
    "macos_dmg_path_binding": "pre_post_identity_size_sha256",
    "macos_bundle_app_tree": (
        "all_regular_files_except_code_signature_and_self_evidence"
    ),
    "macos_partial_attach_cleanup": "mount_probe_then_single_detach",
    "macos_installed_acceptance_runwide_lifecycle_lock": True,
    "macos_installed_acceptance_product_version_source": "apps/desktop/package.json",
    "macos_installed_acceptance_source_head": "clean_head_equals_fresh_origin_main",
    "macos_installed_acceptance_source_observation": (
        "serial_bracketed_head_branch_status_origin_remote"
    ),
    "macos_installed_acceptance_pre_mutation_source_gate": (
        "readonly_before_recovery_then_revalidated_under_lease"
    ),
    "macos_installed_acceptance_build_source": "isolated_local_clone_of_exact_commit",
    "macos_installed_acceptance_mutation_authority": (
        "live_lease_and_exact_source_revalidated_before_each_mutation"
    ),
    "macos_installed_acceptance_branch_cleanup_main_sync_required": True,
    "macos_installed_acceptance_commit_binding": (
        "bundle_dmg_receipt_installed_acceptance_and_soak_equal"
    ),
    "macos_installed_acceptance_authorized_source_schema": 29,
    "macos_installed_acceptance_cow_mode": (
        "apfs_clonefile_no_fallback_source_unchanged"
    ),
    "macos_installed_acceptance_cold_gate": (
        "direct_v29_to_current_preserved_control_plane_ready_metadata_fulltext_vector_epoch_and_search"
    ),
    "macos_installed_acceptance_search_witness": (
        "owner_only_public_canary_daemon_import_nonzero_exact_epoch"
    ),
    "macos_installed_acceptance_artifact_digest": (
        "streaming_ciphertext_sha256_owner_mode_inode_stable"
    ),
    "macos_installed_acceptance_strong_kill_evidence": (
        "current_receipt_boundary_next_ready_generation"
    ),
    "macos_installed_acceptance_busy_attempts_per_index": 2,
    "macos_installed_acceptance_attempt_5_required": True,
    "macos_installed_acceptance_exact_pid_quit": True,
    "macos_installed_acceptance_native_executable_residue_count": 4,
    "macos_installed_acceptance_final_sequence": (
        "locks_released_then_quit_relaunch_ready_search_and_diagnostics"
    ),
    "macos_installed_acceptance_native_save_dialog_diagnostics_required": True,
    "macos_installed_acceptance_launch_authority": (
        "durable_intent_pending_running_guardian_process_group"
    ),
    "macos_installed_acceptance_process_identity": (
        "pid_pgid_start_executable_session_authority"
    ),
    "macos_installed_acceptance_interruption_cleanup": (
        "trusted_marker_exact_process_group_reaper_and_inode_quarantine"
    ),
    "macos_installed_acceptance_cow_cleanup": (
        "verified_parent_inode_random_quarantine_revalidate_before_delete"
    ),
}

FEATURE_TRAIN_FINAL_DELIVERY = {
    "synthetic_soak_minutes": 120,
    "merged_main_installed_acceptance_precedes_soak": True,
    "soak_commit_equals_installed_acceptance_commit": True,
    "all_deployed_failures_regressionized_before_non_soak": True,
    "deployed_regression_restarts_soak_from_zero": True,
}

FORWARD_MIGRATION_FEATURE_TRAIN_REQUIRED_FIELDS = {
    **FORWARD_MIGRATION_FEATURE_TRAIN_IDENTITY,
    **FEATURE_TRAIN_PUBLICATION_RETIREMENT,
    **FEATURE_TRAIN_INSTALLED_ACCEPTANCE,
    **FEATURE_TRAIN_FINAL_DELIVERY,
}

DAEMON_BOOTSTRAP_V1_REQUIRED_FIELDS = {
    "contract": "resume-ir.daemon-bootstrap.v1",
    "aggregate_ipc_contract": "resume-ir.ipc.v4",
    "discovery_contract": "resume-ir.daemon-ipc.v3",
    "auth_contract": "resume-ir.daemon-auth.v3",
    "status_contract": "daemon.status.v3",
    "diagnostics_contract": "resume-ir.diagnostics.v4",
    "error_contract": "resume-ir.error.v2",
    "launch_id_bytes": 32,
    "pre_spawn_discovery_probe_allowed": False,
    "foreign_launch_adoption_allowed": False,
    "status_authentication_required": True,
    "control_plane_deadline_ms": 10_000,
    "control_plane_publish_precedes_store_open": True,
    "control_plane_publish_precedes_runtime_validation": True,
    "status_reads_metadata_on_heartbeat": False,
    "initializing_business_route_status": 503,
    "initializing_business_route_code": "SERVICE_INITIALIZING",
    "automatic_business_request_replay_allowed": False,
}

DESKTOP_SUPERVISOR_V2_REQUIRED_FIELDS = {
    "contract": "resume-ir.desktop-daemon-lifecycle.v2",
    "lifecycle_receipt_contract": "resume-ir.desktop-daemon-lifecycle-receipt.v2",
    "desktop_diagnostics_contract": "resume-ir.desktop-diagnostics.v2",
    "restart_policy_scope": "tauri_process_generation",
    "persistent_restart_ledger_allowed": False,
    "legacy_restart_ledger_is_input": False,
    "child_poll_ms": 100,
    "heartbeat_interval_ms": 5_000,
    "heartbeat_timeout_ms": 2_000,
    "heartbeat_failures_before_recycle": 3,
    "unexpected_exit_budget_count": 5,
    "unexpected_exit_budget_window_seconds": 600,
    "circuit_open_seconds": 300,
    "ready_reset_seconds": 300,
    "restart_backoff_ms": [250, 1_000, 4_000, 15_000, 30_000],
    "normal_app_shutdown_consumes_failure_budget": False,
    "manual_retry_states": ["blocked", "circuit_open"],
    "retry_outside_allowed_state": "typed_retry_not_allowed",
}

RUNTIME_CAPABILITY_DEGRADATION_V1_REQUIRED_FIELDS = {
    "contract": "resume-ir.runtime-capabilities.v1",
    "optional_runtimes": ["embedding", "ocr", "classifier"],
    "operation_capabilities": [
        "keyword_search",
        "detail",
        "semantic_search",
        "hybrid_search",
        "text_import",
        "ocr_import",
        "index_publication",
    ],
    "runtime_validation_precedes_control_plane": False,
    "runtime_fault_may_exit_core_daemon": False,
    "keyword_and_detail_survive_optional_runtime_faults": True,
    "embedding_fault_hybrid_behavior": "lexical_partial",
    "ocr_fault_behavior": "retain_unclaimed_backlog",
    "classifier_fault_behavior": "freeze_mutation_preserve_epoch",
    "worker_capability_gate_precedes_claim": True,
    "runtime_degradation_may_weaken_package_gate": False,
}


def load_json(path: pathlib.Path) -> object:
    with path.open("rb") as fh:
        return json.load(fh)


def load_toml(path: pathlib.Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def fail(message: str) -> None:
    raise ValueError(message)


def require_mapping(value: object, path: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        fail(f"{path}: expected object")
    return value


def require_list(value: object, path: str) -> list:
    if not isinstance(value, list):
        fail(f"{path}: expected array")
    return value


def require_bool(value: object, expected: bool, path: str) -> None:
    if value is not expected:
        fail(f"{path}: expected {expected}")


def require_number_at_most(value: object, limit: float, path: str) -> None:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        fail(f"{path}: expected number")
    if value > limit:
        fail(f"{path}: {value} exceeds {limit}")


def require_number_at_least(value: object, minimum: float, path: str) -> None:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        fail(f"{path}: expected number")
    if value < minimum:
        fail(f"{path}: {value} below {minimum}")


def require_hex64(value: object, path: str) -> None:
    if not isinstance(value, str) or len(value) != 64 or any(ch not in HEX64 for ch in value):
        fail(f"{path}: expected lowercase sha256 hex")


def require_nonzero_hex64(value: object, path: str) -> None:
    require_hex64(value, path)
    if value == ZERO_SHA256:
        fail(f"{path}: zero sha256 placeholder is not allowed")


def require_non_empty_string(value: object, path: str) -> None:
    if not isinstance(value, str) or not value:
        fail(f"{path}: expected non-empty string")


def require_enum(value: object, allowed: list[str] | set[str], path: str) -> None:
    if value not in allowed:
        fail(f"{path}: invalid value {value!r}")


def require_bool_fields(value: Mapping[str, object], fields: list[str], expected: bool, path: str) -> None:
    for field in fields:
        require_bool(value.get(field), expected, f"{path}.{field}")


def require_exact_keys(value: Mapping[str, object], expected: set[str], path: str) -> None:
    observed = set(value.keys())
    if observed != expected:
        missing = sorted(expected - observed)
        extra = sorted(observed - expected)
        fail(f"{path}: key mismatch missing={missing} extra={extra}")


def require_main_reachable_commit(value: object, path: str) -> None:
    if not isinstance(value, str) or not value:
        fail(f"{path}: expected main-reachable git commit")
    if value == "working-tree":
        fail(f"{path}: expected main-reachable git commit, got working-tree")


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_git_commit_exists(value: object, path: str) -> None:
    if not isinstance(value, str) or len(value) < 7 or len(value) > 40 or any(ch not in HEX64 for ch in value):
        fail(f"{path}: expected git commit hex")
    completed = subprocess.run(
        ["git", "cat-file", "-e", f"{value}^{{commit}}"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if completed.returncode != 0:
        fail(f"{path}: commit does not exist in local git object database")


def require_git_commit_reachable_from_origin_main(value: object, path: str) -> None:
    require_git_commit_exists(value, path)
    origin_main = "refs/remotes/origin/main"
    has_origin_main = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", origin_main],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if has_origin_main.returncode != 0:
        fail(f"{path}: {origin_main} is required for current snapshot validation")
    reachable = subprocess.run(
        ["git", "merge-base", "--is-ancestor", str(value), origin_main],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if reachable.returncode != 0:
        fail(f"{path}: commit is not reachable from origin/main")


def validate_current_loop_contract_pins(state: Mapping[str, object]) -> None:
    pins = require_mapping(state.get("contract_pins"), "perf/current-loop-state.json.contract_pins")
    for key, rel_path in CURRENT_LOOP_CONTRACT_FILES.items():
        observed = pins.get(key)
        require_nonzero_hex64(observed, f"perf/current-loop-state.json.contract_pins.{key}")
        expected = sha256_file(ROOT / rel_path)
        if observed != expected:
            fail(
                "perf/current-loop-state.json.contract_pins."
                f"{key}: expected {expected} from {rel_path}, got {observed}"
            )
    head = pins.get("git_head_sha")
    if head == "working-tree":
        fail("perf/current-loop-state.json.contract_pins.git_head_sha: working-tree placeholder is not allowed")
    require_git_commit_reachable_from_origin_main(head, "perf/current-loop-state.json.contract_pins.git_head_sha")


def validate_current_file_contract_pins(value: object, path: str) -> Mapping[str, object]:
    pins = require_mapping(value, path)
    for key, rel_path in CURRENT_LOOP_CONTRACT_FILES.items():
        observed = pins.get(key)
        require_nonzero_hex64(observed, f"{path}.{key}")
        expected = sha256_file(ROOT / rel_path)
        if observed != expected:
            fail(f"{path}.{key}: expected {expected} from {rel_path}, got {observed}")
    head = pins.get("git_head_sha")
    if head == "working-tree":
        fail(f"{path}.git_head_sha: working-tree placeholder is not allowed")
    require_git_commit_exists(head, f"{path}.git_head_sha")
    return pins


def validate_privacy(report: Mapping[str, object], *, trace_required: bool, path: str) -> None:
    privacy = require_mapping(report.get("privacy"), f"{path}.privacy")
    for field in PRIVACY_FALSE_FIELDS:
        require_bool(privacy.get(field), False, f"{path}.privacy.{field}")
    if trace_required:
        require_bool(privacy.get("trace_summary_redacted"), True, f"{path}.privacy.trace_summary_redacted")


def validate_contract_pins(value: object, path: str) -> None:
    pins = require_mapping(value, path)
    for key in [
        "active_goal_sha256",
        "acceptance_matrix_sha256",
        "loop_state_schema_sha256",
        "experiment_report_schema_sha256",
    ]:
        require_hex64(pins.get(key), f"{path}.{key}")
    if "synthetic_smoke_artifact_manifest_schema_sha256" in pins:
        require_hex64(pins.get("synthetic_smoke_artifact_manifest_schema_sha256"), f"{path}.synthetic_smoke_artifact_manifest_schema_sha256")
    head = pins.get("git_head_sha")
    if not isinstance(head, str) or not head:
        fail(f"{path}.git_head_sha: expected git sha or working-tree")


def validate_schema_file(schema: Mapping[str, object], path: str, expected_version: str) -> None:
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        fail(f"{path}: missing draft 2020-12 schema declaration")
    props = require_mapping(schema.get("properties"), f"{path}.properties")
    schema_version = require_mapping(props.get("schema_version"), f"{path}.properties.schema_version")
    if schema_version.get("const") != expected_version:
        fail(f"{path}: wrong schema_version const")
    if expected_version != "resume-ir.synthetic-smoke-artifact-manifest.v1":
        all_of = require_list(schema.get("allOf"), f"{path}.allOf")
        if not all_of:
            fail(f"{path}: must use conditional schema rules")
    defs = require_mapping(schema.get("$defs"), f"{path}.$defs")
    required_defs = ["contract_pins", "privacy"]
    for required_def in required_defs:
        if required_def not in defs:
            fail(f"{path}: missing $defs.{required_def}")


def validate_component_array_schema(value: object, path: str) -> None:
    component_array = require_mapping(value, path)
    if component_array.get("minItems") != len(SYNTHETIC_SMOKE_COMPONENTS):
        fail(f"{path}.minItems: expected {len(SYNTHETIC_SMOKE_COMPONENTS)}")
    if component_array.get("maxItems") != len(SYNTHETIC_SMOKE_COMPONENTS):
        fail(f"{path}.maxItems: expected {len(SYNTHETIC_SMOKE_COMPONENTS)}")
    all_of = require_list(component_array.get("allOf"), f"{path}.allOf")
    observed = set()
    for index, rule in enumerate(all_of):
        contains = require_mapping(
            require_mapping(rule, f"{path}.allOf[{index}]").get("contains"),
            f"{path}.allOf[{index}].contains",
        )
        props = require_mapping(contains.get("properties"), f"{path}.allOf[{index}].contains.properties")
        component = require_mapping(props.get("component"), f"{path}.allOf[{index}].contains.properties.component")
        observed.add(component.get("const"))
    if observed != SYNTHETIC_SMOKE_COMPONENTS:
        fail(f"{path}.allOf: expected component coverage {sorted(SYNTHETIC_SMOKE_COMPONENTS)}, got {sorted(observed)}")


def validate_experiment_report_schema(schema: Mapping[str, object], matrix: Mapping[str, object]) -> None:
    validate_schema_file(schema, "perf/experiment-report.schema.json", "resume-ir.experiment-report.v2")
    smoke_baseline = require_mapping(
        matrix.get("synthetic_smoke_baseline"),
        "matrix.synthetic_smoke_baseline",
    )
    root_props = require_mapping(
        schema.get("properties"),
        "perf/experiment-report.schema.json.properties",
    )
    all_of = require_list(schema.get("allOf"), "perf/experiment-report.schema.json.allOf")
    smoke_rule = None
    for index, rule in enumerate(all_of):
        rule_mapping = require_mapping(rule, f"perf/experiment-report.schema.json.allOf[{index}]")
        condition = require_mapping(rule_mapping.get("if"), f"perf/experiment-report.schema.json.allOf[{index}].if")
        condition_props = require_mapping(
            condition.get("properties"),
            f"perf/experiment-report.schema.json.allOf[{index}].if.properties",
        )
        evidence_lane = condition_props.get("evidence_lane")
        if isinstance(evidence_lane, Mapping) and evidence_lane.get("const") == "smoke":
            smoke_rule = rule_mapping
            break
    if smoke_rule is None:
        fail("perf/experiment-report.schema.json.allOf: missing smoke evidence_lane rule")
    smoke_then = require_mapping(smoke_rule.get("then"), "perf/experiment-report.schema.json smoke then")
    smoke_not = require_mapping(smoke_then.get("not"), "perf/experiment-report.schema.json smoke then.not")
    forbidden_rules = require_list(
        smoke_not.get("anyOf"),
        "perf/experiment-report.schema.json smoke then.not.anyOf",
    )
    forbidden_fields = set()
    for index, rule in enumerate(forbidden_rules):
        required = require_list(
            require_mapping(rule, f"perf/experiment-report.schema.json smoke then.not.anyOf[{index}]").get("required"),
            f"perf/experiment-report.schema.json smoke then.not.anyOf[{index}].required",
        )
        if len(required) != 1:
            fail(f"perf/experiment-report.schema.json smoke then.not.anyOf[{index}].required: expected one field")
        forbidden_fields.add(required[0])
    expected_forbidden_fields = set(root_props) - SYNTHETIC_SMOKE_TOP_LEVEL_KEYS
    if forbidden_fields != expected_forbidden_fields:
        fail(
            "perf/experiment-report.schema.json smoke then.not.anyOf: expected forbidden fields "
            f"{sorted(expected_forbidden_fields)}, got {sorted(forbidden_fields)}"
        )
    smoke_props = require_mapping(
        smoke_then.get("properties"),
        "perf/experiment-report.schema.json smoke then.properties",
    )
    smoke_contract_pins = require_mapping(
        smoke_props.get("contract_pins"),
        "perf/experiment-report.schema.json smoke then.properties.contract_pins",
    )
    smoke_contract_required = require_list(
        smoke_contract_pins.get("required"),
        "perf/experiment-report.schema.json smoke then.properties.contract_pins.required",
    )
    if "synthetic_smoke_artifact_manifest_schema_sha256" not in smoke_contract_required:
        fail(
            "perf/experiment-report.schema.json smoke contract_pins must require "
            "synthetic_smoke_artifact_manifest_schema_sha256"
        )
    smoke_contract_props = require_mapping(
        smoke_contract_pins.get("properties"),
        "perf/experiment-report.schema.json smoke then.properties.contract_pins.properties",
    )
    smoke_git_head = require_mapping(
        smoke_contract_props.get("git_head_sha"),
        "perf/experiment-report.schema.json smoke then.properties.contract_pins.properties.git_head_sha",
    )
    smoke_git_head_not = require_mapping(
        smoke_git_head.get("not"),
        "perf/experiment-report.schema.json smoke then.properties.contract_pins.properties.git_head_sha.not",
    )
    if smoke_git_head_not.get("const") != "working-tree":
        fail("perf/experiment-report.schema.json smoke contract_pins.git_head_sha must reject working-tree")
    defs = require_mapping(schema.get("$defs"), "perf/experiment-report.schema.json.$defs")
    synthetic_smoke = require_mapping(defs.get("synthetic_smoke"), "perf/experiment-report.schema.json.$defs.synthetic_smoke")
    props = require_mapping(synthetic_smoke.get("properties"), "perf/experiment-report.schema.json.$defs.synthetic_smoke.properties")
    for key in ["document_count", "query_count", "top_k"]:
        expected = smoke_baseline.get(key)
        field_schema = require_mapping(
            props.get(key),
            f"perf/experiment-report.schema.json.$defs.synthetic_smoke.properties.{key}",
        )
        if field_schema.get("const") != expected:
            fail(f"perf/experiment-report.schema.json synthetic_smoke.{key}: expected const {expected}")
    command_schema = require_mapping(
        props.get("benchmark_command"),
        "perf/experiment-report.schema.json.$defs.synthetic_smoke.properties.benchmark_command",
    )
    if command_schema.get("const") != smoke_baseline.get("allowed_command"):
        fail("perf/experiment-report.schema.json synthetic_smoke.benchmark_command: expected matrix allowed_command const")
    validate_component_array_schema(
        props.get("component_reports"),
        "perf/experiment-report.schema.json.$defs.synthetic_smoke.properties.component_reports",
    )


def validate_synthetic_smoke_manifest_schema(schema: Mapping[str, object]) -> None:
    validate_schema_file(
        schema,
        "perf/synthetic-smoke-artifact-manifest.schema.json",
        "resume-ir.synthetic-smoke-artifact-manifest.v1",
    )
    props = require_mapping(schema.get("properties"), "perf/synthetic-smoke-artifact-manifest.schema.json.properties")
    for key in [
        "manifest_kind",
        "report_sha256",
        "report_size_bytes",
        "artifacts",
    ]:
        if key not in props:
            fail(f"perf/synthetic-smoke-artifact-manifest.schema.json.properties: missing {key}")
    validate_component_array_schema(
        props.get("artifacts"),
        "perf/synthetic-smoke-artifact-manifest.schema.json.properties.artifacts",
    )
    defs = require_mapping(schema.get("$defs"), "perf/synthetic-smoke-artifact-manifest.schema.json.$defs")
    contract_pins = require_mapping(
        defs.get("contract_pins"),
        "perf/synthetic-smoke-artifact-manifest.schema.json.$defs.contract_pins",
    )
    contract_pin_props = require_mapping(
        contract_pins.get("properties"),
        "perf/synthetic-smoke-artifact-manifest.schema.json.$defs.contract_pins.properties",
    )
    git_head = require_mapping(
        contract_pin_props.get("git_head_sha"),
        "perf/synthetic-smoke-artifact-manifest.schema.json.$defs.contract_pins.properties.git_head_sha",
    )
    git_head_not = require_mapping(
        git_head.get("not"),
        "perf/synthetic-smoke-artifact-manifest.schema.json.$defs.contract_pins.properties.git_head_sha.not",
    )
    if git_head_not.get("const") != "working-tree":
        fail("perf/synthetic-smoke-artifact-manifest.schema.json contract_pins.git_head_sha must reject working-tree")


def validate_forward_migration_feature_train(matrix: Mapping[str, object]) -> None:
    correctness = require_mapping(
        matrix.get("forward_migration_feature_train_v1"),
        "matrix.forward_migration_feature_train_v1",
    )
    for key, expected in FORWARD_MIGRATION_FEATURE_TRAIN_REQUIRED_FIELDS.items():
        observed = correctness.get(key)
        if observed != expected:
            fail(
                f"matrix.forward_migration_feature_train_v1.{key}: "
                f"expected {expected!r}, got {observed!r}"
            )


def validate_exact_contract_section(
    matrix: Mapping[str, object],
    section_name: str,
    required_fields: Mapping[str, object],
) -> None:
    section = require_mapping(matrix.get(section_name), f"matrix.{section_name}")
    for key, expected in required_fields.items():
        observed = section.get(key)
        if observed != expected:
            fail(
                f"matrix.{section_name}.{key}: "
                f"expected {expected!r}, got {observed!r}"
            )


def validate_matrix(matrix: Mapping[str, object]) -> None:
    if matrix.get("schema_version") != "resume-ir.perf.acceptance-matrix.v2":
        fail("perf/acceptance-matrix.toml: expected v2 schema")
    validate_forward_migration_feature_train(matrix)
    validate_exact_contract_section(
        matrix, "daemon_bootstrap_v1", DAEMON_BOOTSTRAP_V1_REQUIRED_FIELDS
    )
    validate_exact_contract_section(
        matrix, "desktop_supervisor_v2", DESKTOP_SUPERVISOR_V2_REQUIRED_FIELDS
    )
    validate_exact_contract_section(
        matrix,
        "runtime_capability_degradation_v1",
        RUNTIME_CAPABILITY_DEGRADATION_V1_REQUIRED_FIELDS,
    )
    smoke_lane = require_mapping(matrix.get("evidence_lanes", {}).get("smoke"), "matrix.evidence_lanes.smoke")
    if smoke_lane.get("report_schema") != "resume-ir.synthetic-smoke-baseline.v1":
        fail("matrix.evidence_lanes.smoke.report_schema mismatch")
    if smoke_lane.get("artifact_manifest_schema") != "resume-ir.synthetic-smoke-artifact-manifest.v1":
        fail("matrix.evidence_lanes.smoke.artifact_manifest_schema mismatch")
    require_bool(smoke_lane.get("requires_artifact_manifest"), True, "matrix.evidence_lanes.smoke.requires_artifact_manifest")
    cannot_claim = require_list(smoke_lane.get("cannot_claim"), "matrix.evidence_lanes.smoke.cannot_claim")
    expected_cannot_claim = [
        "w1_private_baseline",
        "d10k_private_calibration",
        "d100k_weak_host",
        "d1m_scale",
        "profile_optimization_issue",
        "million_scale_readiness",
        "scale_gate",
        "gui_manual_complete",
        "goal_complete",
    ]
    if cannot_claim != expected_cannot_claim:
        fail("matrix.evidence_lanes.smoke.cannot_claim mismatch")
    smoke_baseline = require_mapping(matrix.get("synthetic_smoke_baseline"), "matrix.synthetic_smoke_baseline")
    if smoke_baseline.get("source") != "synthetic_public_fixture":
        fail("matrix.synthetic_smoke_baseline.source mismatch")
    for key, expected in {
        "document_count": 24,
        "query_count": 6,
        "top_k": 5,
    }.items():
        if smoke_baseline.get(key) != expected:
            fail(f"matrix.synthetic_smoke_baseline.{key} mismatch")
    for key in ["private_resume_root_required", "query_artifact_root_required", "resident_daemon_required"]:
        require_bool(smoke_baseline.get(key), False, f"matrix.synthetic_smoke_baseline.{key}")
    require_bool(
        smoke_baseline.get("batch_protocol_observation_required"),
        True,
        "matrix.synthetic_smoke_baseline.batch_protocol_observation_required",
    )
    if smoke_baseline.get("percentile_confidence") != "smoke":
        fail("matrix.synthetic_smoke_baseline.percentile_confidence mismatch")
    if smoke_baseline.get("report_kind") != "redacted_evidence":
        fail("matrix.synthetic_smoke_baseline.report_kind mismatch")
    if smoke_baseline.get("claim") != "no_claim":
        fail("matrix.synthetic_smoke_baseline.claim mismatch")
    expected_command = (
        "resume-benchmark synthetic-query --index-dir <redacted-temp-index> "
        "--documents 24 --queries 6 --top-k 5 --json"
    )
    if smoke_baseline.get("allowed_command") != expected_command:
        fail("matrix.synthetic_smoke_baseline.allowed_command mismatch")
    required_smoke_commands = require_list(
        smoke_baseline.get("required_commands"),
        "matrix.synthetic_smoke_baseline.required_commands",
    )
    expected_smoke_commands = [
        "python3 scripts/ci/check-performance-contracts.py",
        "RESUME_IR_BENCHMARK_SMOKE_REPORT_OUT=<tmp-report> RESUME_IR_BENCHMARK_SMOKE_MANIFEST_OUT=<tmp-manifest> ./scripts/ci/check-benchmark-smoke.sh",
        "./scripts/ci/guard-public-repo.sh",
    ]
    if required_smoke_commands != expected_smoke_commands:
        fail("matrix.synthetic_smoke_baseline.required_commands mismatch")

    mixed_import = require_mapping(
        matrix.get("mixed_import_correctness"),
        "matrix.mixed_import_correctness",
    )
    expected_mixed_strings = {
        "phase": "product_contract_precondition",
        "must_complete_after_issue": "#138",
        "contract_owner_issue": "#140",
        "report_schema_target": "resume-ir.mixed-import-report.v1",
        "report_schema_path": "perf/mixed-import-report.schema.json",
        "contract_checker_target": "scripts/ci/check-mixed-import-contracts.py",
        "public_synthetic_fixture_path": "perf/fixtures/mixed-import/public-synthetic-benchmark.json",
        "valid_report_fixture_path": "perf/fixtures/mixed-import/valid-public-report.json",
        "negative_cases_fixture_path": "perf/fixtures/mixed-import/invalid-cases.json",
        "freeze_identity_scheme": "hmac_sha256_opaque_manifest_v1",
        "public_synthetic_source_visibility": "committed_synthetic_exact",
        "public_synthetic_output_visibility": "bounded_aggregate_only",
        "private_calibration_visibility": "local_tuning_redacted_aggregate_public",
        "blind_holdout_visibility": "acceptance_only_redacted_aggregate_public",
        "indexed_resume_precision_formula": "indexed_true_resumes / indexed_total",
        "resume_completeness_formula": "indexed_true_resumes / expected_true_resumes",
        "classifier_position": "after_parse_text_extraction_before_searchable_indexing",
        "fs_crawler_role": "cheap_source_discovery_only",
        "public_evidence": "redacted_aggregate_only",
    }
    for key, expected in expected_mixed_strings.items():
        if mixed_import.get(key) != expected:
            fail(f"matrix.mixed_import_correctness.{key} mismatch")
    require_bool_fields(
        mixed_import,
        [
            "required_before_gui_implementation",
            "required_before_query_hot_path_optimization",
            "benchmark_must_be_frozen_before_classifier",
            "report_contract_implemented",
            "precision_first",
            "local_per_file_audit_required",
            "completeness_improvement_requires_precision_non_regression",
            "completeness_improvement_requires_contamination_within_limit",
        ],
        True,
        "matrix.mixed_import_correctness",
    )
    require_bool_fields(
        mixed_import,
        [
            "mutation_after_freeze_allowed",
            "blind_holdout_visible_during_calibration",
            "synthetic_fixture_ids_classifier_features_allowed",
            "labels_classifier_features_allowed",
            "classifier_uses_ai_or_llm",
            "needs_review_indexed_by_default",
            "non_resume_indexed",
            "ocr_backlog_classified_as_non_resume_before_ocr",
        ],
        False,
        "matrix.mixed_import_correctness",
    )
    if mixed_import.get("indexed_resume_precision_min") != 1.0:
        fail("matrix.mixed_import_correctness.indexed_resume_precision_min mismatch")
    if mixed_import.get("contamination_count_max") != 0:
        fail("matrix.mixed_import_correctness.contamination_count_max mismatch")
    if mixed_import.get("precision_non_regression_tolerance") != 0.0:
        fail("matrix.mixed_import_correctness.precision_non_regression_tolerance mismatch")
    if mixed_import.get("public_synthetic_sample_count") != 9:
        fail("matrix.mixed_import_correctness.public_synthetic_sample_count mismatch")
    if mixed_import.get("benchmark_layers") != MIXED_IMPORT_BENCHMARK_LAYERS:
        fail("matrix.mixed_import_correctness.benchmark_layers mismatch")
    if mixed_import.get("status_values") != MIXED_IMPORT_STATUSES:
        fail("matrix.mixed_import_correctness.status_values mismatch")
    if mixed_import.get("required_metrics") != MIXED_IMPORT_REQUIRED_METRICS:
        fail("matrix.mixed_import_correctness.required_metrics mismatch")
    if mixed_import.get("forbidden_signals") != MIXED_IMPORT_FORBIDDEN_SIGNALS:
        fail("matrix.mixed_import_correctness.forbidden_signals mismatch")

    scale_gates = require_mapping(matrix.get("scale_gates"), "matrix.scale_gates")
    for gate, minimums in {
        "D10K_private_calibration": (10_000, 8_000, False),
        "D100K_weak_host": (100_000, 90_000, False),
        "D1M_scale": (1_000_000, 900_000, True),
    }.items():
        entry = require_mapping(scale_gates.get(gate), f"matrix.scale_gates.{gate}")
        if entry.get("min_document_count") != minimums[0]:
            fail(f"matrix.scale_gates.{gate}.min_document_count mismatch")
        if entry.get("min_searchable_document_count") != minimums[1]:
            fail(f"matrix.scale_gates.{gate}.min_searchable_document_count mismatch")
        if entry.get("may_claim_goal_complete") is not minimums[2]:
            fail(f"matrix.scale_gates.{gate}.may_claim_goal_complete mismatch")
    query_semantics = require_mapping(matrix.get("query_semantics"), "matrix.query_semantics")
    required = query_semantics.get("required_query_buckets")
    if required != REQUIRED_BUCKETS:
        fail("matrix.query_semantics.required_query_buckets mismatch")
    gui_redlines = require_mapping(matrix.get("gui_redlines"), "matrix.gui_redlines")
    if gui_redlines.get("desktop_reference_visible_cards") != 4:
        fail("matrix.gui_redlines.desktop_reference_visible_cards must be 4")
    if gui_redlines.get("buffered_cards_max") != 8:
        fail("matrix.gui_redlines.buffered_cards_max must be 8")
    if gui_redlines.get("navigation_modes") != ["scroll", "pagination"]:
        fail("matrix.gui_redlines.navigation_modes mismatch")
    require_bool(
        gui_redlines.get("stable_card_height_required"),
        True,
        "matrix.gui_redlines.stable_card_height_required",
    )

    optimization_layers = require_mapping(matrix.get("optimization_layers"), "matrix.optimization_layers")
    if optimization_layers.get("allowed") != OPTIMIZATION_LAYERS:
        fail("matrix.optimization_layers.allowed mismatch")
    require_bool_fields(
        optimization_layers,
        ["require_single_primary_layer", "allow_affected_layers", "l0_is_precondition_not_layer"],
        True,
        "matrix.optimization_layers",
    )
    if optimization_layers.get("algorithm_index_choice_layer") != "L1":
        fail("matrix.optimization_layers.algorithm_index_choice_layer mismatch")
    if optimization_layers.get("data_quality_workload_representativeness_layer") != "L0":
        fail("matrix.optimization_layers.data_quality_workload_representativeness_layer mismatch")

    optimization_redlines = require_mapping(matrix.get("optimization_layer_redlines"), "matrix.optimization_layer_redlines")
    require_bool_fields(
        optimization_redlines,
        [
            "missing_baseline_blocks_optimization",
            "missing_profile_blocks_optimization",
            "missing_hypothesis_blocks_optimization",
            "missing_expected_delta_blocks_optimization",
            "missing_rollback_condition_blocks_optimization",
            "missing_negative_controls_blocks_optimization",
            "lower_layer_cannot_close_higher_layer_blocker",
            "hand_written_simd_requires_scope_exception",
        ],
        True,
        "matrix.optimization_layer_redlines",
    )

    platform_lanes = require_mapping(matrix.get("platform_lanes"), "matrix.platform_lanes")
    if platform_lanes.get("allowed") != PLATFORM_LANES:
        fail("matrix.platform_lanes.allowed mismatch")
    require_bool(platform_lanes.get("macos_m4_can_close_windows_gate"), False, "matrix.platform_lanes.macos_m4_can_close_windows_gate")
    require_bool(
        platform_lanes.get("cross_os_ci_smoke_can_replace_weak_host_perf"),
        False,
        "matrix.platform_lanes.cross_os_ci_smoke_can_replace_weak_host_perf",
    )

    gui_stack = require_mapping(matrix.get("gui_stack"), "matrix.gui_stack")
    if gui_stack.get("default_stack") != GUI_DEFAULT_STACK:
        fail("matrix.gui_stack.default_stack mismatch")
    require_bool(gui_stack.get("production_next_server_allowed"), False, "matrix.gui_stack.production_next_server_allowed")
    if gui_stack.get("visual_reference_role") != GUI_REFERENCE_ROLE:
        fail("matrix.gui_stack.visual_reference_role mismatch")
    require_bool(
        gui_stack.get("pixel_level_visual_similarity_required"),
        True,
        "matrix.gui_stack.pixel_level_visual_similarity_required",
    )
    require_bool(
        gui_stack.get("toolkit_bakeoff_requires_blocker_issue"),
        True,
        "matrix.gui_stack.toolkit_bakeoff_requires_blocker_issue",
    )

    gui_visual_redlines = require_mapping(matrix.get("gui_visual_redlines"), "matrix.gui_visual_redlines")
    require_bool_fields(
        gui_visual_redlines,
        [
            "left_rail_required",
            "top_command_bar_required",
            "center_workspace_required",
            "detail_panel_or_side_sheet_required",
            "dense_result_list_required",
            "stable_row_or_card_dimensions_required",
            "lucide_style_icon_vocabulary_required",
            "tailwind_token_inventory_required",
            "reference_screenshot_inventory_required",
        ],
        True,
        "matrix.gui_visual_redlines",
    )
    require_bool(gui_visual_redlines.get("functional_clone_required"), False, "matrix.gui_visual_redlines.functional_clone_required")


def required_completion_cells(matrix: Mapping[str, object]) -> set[str]:
    completion = require_mapping(matrix.get("completion"), "matrix.completion")
    cells = require_list(completion.get("goal_complete_requires"), "matrix.completion.goal_complete_requires")
    if not cells:
        fail("matrix.completion.goal_complete_requires: must not be empty")
    for index, cell in enumerate(cells):
        if not isinstance(cell, str) or not cell:
            fail(f"matrix.completion.goal_complete_requires[{index}]: expected non-empty string")
    return set(cells)


def validate_thresholds(report: Mapping[str, object], path: str) -> None:
    thresholds = require_mapping(report.get("thresholds"), f"{path}.thresholds")
    if thresholds.get("matrix") != "perf/acceptance-matrix.toml":
        fail(f"{path}.thresholds.matrix mismatch")
    if thresholds.get("matrix_schema_version") != "resume-ir.perf.acceptance-matrix.v2":
        fail(f"{path}.thresholds.matrix_schema_version mismatch")
    require_list(thresholds.get("failed_redlines"), f"{path}.thresholds.failed_redlines")


def validate_query_buckets(value: object, matrix: Mapping[str, object], path: str, *, samples: bool) -> None:
    counts = require_mapping(value, path)
    if set(counts.keys()) != set(REQUIRED_BUCKETS):
        fail(f"{path}: bucket set mismatch")
    min_counts = require_mapping(matrix.get("query_bucket_min_counts"), "matrix.query_bucket_min_counts")
    for bucket in REQUIRED_BUCKETS:
        minimum = 0 if samples else int(min_counts[bucket])
        require_number_at_least(counts.get(bucket), minimum, f"{path}.{bucket}")


def validate_w0_report(report: Mapping[str, object], matrix: Mapping[str, object], path: str) -> None:
    docs_gate = require_mapping(report.get("docs_gate"), f"{path}.docs_gate")
    commands = require_list(docs_gate.get("commands"), f"{path}.docs_gate.commands")
    if not commands:
        fail(f"{path}.docs_gate.commands: must not be empty")
    for index, command in enumerate(commands):
        command = require_mapping(command, f"{path}.docs_gate.commands[{index}]")
        if command.get("exit_code") != 0:
            fail(f"{path}.docs_gate.commands[{index}].exit_code must be 0")
    require_bool(docs_gate.get("private_data_in_git"), False, f"{path}.docs_gate.private_data_in_git")
    validate_thresholds(report, path)


def validate_optimization(value: object, path: str) -> None:
    optimization = require_mapping(value, path)
    require_enum(optimization.get("optimization_layer"), OPTIMIZATION_LAYERS, f"{path}.optimization_layer")
    affected_layers = optimization.get("affected_layers")
    if affected_layers is not None:
        layers = require_list(affected_layers, f"{path}.affected_layers")
        if len(layers) != len(set(layers)):
            fail(f"{path}.affected_layers: duplicate layers")
        for index, layer in enumerate(layers):
            require_enum(layer, OPTIMIZATION_LAYERS, f"{path}.affected_layers[{index}]")
    for key in [
        "baseline_artifact",
        "profiler_summary",
        "stage_histogram",
        "bottleneck_statement",
        "hypothesis",
        "expected_delta",
        "rollback_condition",
    ]:
        require_non_empty_string(optimization.get(key), f"{path}.{key}")
    negative_controls = require_list(optimization.get("negative_controls"), f"{path}.negative_controls")
    if not negative_controls:
        fail(f"{path}.negative_controls: must not be empty")
    for index, control in enumerate(negative_controls):
        require_non_empty_string(control, f"{path}.negative_controls[{index}]")
    require_enum(optimization.get("acceptance_gate"), SCALE_GATES, f"{path}.acceptance_gate")
    require_bool(
        optimization.get("lower_layer_closes_higher_layer_blocker"),
        False,
        f"{path}.lower_layer_closes_higher_layer_blocker",
    )


def validate_workload_manifest(value: object, path: str) -> None:
    workload = require_mapping(value, path)
    require_non_empty_string(workload.get("query_set_source"), f"{path}.query_set_source")
    require_enum(workload.get("corpus_scale"), SCALE_GATES, f"{path}.corpus_scale")
    for key in ["hardware_class", "warm_or_cold_definition", "cache_state"]:
        require_non_empty_string(workload.get(key), f"{path}.{key}")


def validate_platform_evidence(value: object, path: str) -> None:
    platform = require_mapping(value, path)
    require_enum(platform.get("platform_lane"), PLATFORM_LANES, f"{path}.platform_lane")
    for key in ["hardware_class", "os_build_class", "power_mode", "runner_version"]:
        require_non_empty_string(platform.get(key), f"{path}.{key}")


def validate_gui_visual(value: object, path: str) -> None:
    visual = require_mapping(value, path)
    if visual.get("visual_reference_role") != GUI_REFERENCE_ROLE:
        fail(f"{path}.visual_reference_role mismatch")
    if visual.get("default_stack") != GUI_DEFAULT_STACK:
        fail(f"{path}.default_stack mismatch")
    require_bool(
        visual.get("production_next_server_allowed"),
        False,
        f"{path}.production_next_server_allowed",
    )
    require_non_empty_string(visual.get("token_inventory_ref"), f"{path}.token_inventory_ref")
    require_non_empty_string(visual.get("screenshot_inventory_ref"), f"{path}.screenshot_inventory_ref")
    require_bool(
        visual.get("pixel_level_similarity_reviewed"),
        True,
        f"{path}.pixel_level_similarity_reviewed",
    )


def validate_w1_report(report: Mapping[str, object], matrix: Mapping[str, object], path: str) -> None:
    validate_optimization(report.get("optimization"), f"{path}.optimization")
    validate_workload_manifest(report.get("workload_manifest"), f"{path}.workload_manifest")
    validate_platform_evidence(report.get("platform_evidence"), f"{path}.platform_evidence")

    dataset = require_mapping(report.get("dataset"), f"{path}.dataset")
    if "dataset_sha256" in dataset:
        fail(f"{path}.dataset.dataset_sha256: legacy field removed; use dataset_manifest_sha256")
    require_hex64(
        dataset.get("dataset_manifest_sha256"),
        f"{path}.dataset.dataset_manifest_sha256",
    )
    query_set = require_mapping(report.get("query_set"), f"{path}.query_set")
    scale_gate = dataset.get("scale_gate")
    scale = require_mapping(matrix.get("scale_gates", {}).get(scale_gate), f"matrix.scale_gates.{scale_gate}")

    require_number_at_least(dataset.get("document_count"), scale["min_document_count"], f"{path}.dataset.document_count")
    require_number_at_least(
        dataset.get("searchable_document_count"),
        scale["min_searchable_document_count"],
        f"{path}.dataset.searchable_document_count",
    )
    require_number_at_least(query_set.get("query_count"), scale["min_query_count"], f"{path}.query_set.query_count")
    require_number_at_least(
        query_set.get("request_sample_count"),
        scale["min_request_sample_count"],
        f"{path}.query_set.request_sample_count",
    )
    require_hex64(query_set.get("query_set_sha256"), f"{path}.query_set.query_set_sha256")
    require_hex64(query_set.get("tune_sha256"), f"{path}.query_set.tune_sha256")
    require_hex64(query_set.get("holdout_sha256"), f"{path}.query_set.holdout_sha256")
    validate_query_buckets(query_set.get("bucket_counts"), matrix, f"{path}.query_set.bucket_counts", samples=False)
    validate_query_buckets(query_set.get("samples_per_bucket"), matrix, f"{path}.query_set.samples_per_bucket", samples=True)
    for bucket, count in require_mapping(query_set.get("samples_per_bucket"), f"{path}.query_set.samples_per_bucket").items():
        require_number_at_least(count, scale["min_samples_per_bucket"], f"{path}.query_set.samples_per_bucket.{bucket}")

    semantic = require_mapping(report.get("semantic_contract"), f"{path}.semantic_contract")
    if semantic.get("query_semantics_version") != matrix.get("query_semantics", {}).get("version"):
        fail(f"{path}.semantic_contract.query_semantics_version mismatch")
    require_bool(semantic.get("metamorphic_checks_passed"), True, f"{path}.semantic_contract.metamorphic_checks_passed")

    runner = require_mapping(report.get("runner"), f"{path}.runner")
    require_bool(runner.get("resident_daemon"), True, f"{path}.runner.resident_daemon")
    require_bool(runner.get("spawn_per_query"), False, f"{path}.runner.spawn_per_query")
    require_bool(runner.get("persistent_connection"), True, f"{path}.runner.persistent_connection")
    require_bool(runner.get("raw_query_stream_local_only"), True, f"{path}.runner.raw_query_stream_local_only")

    hot_path = require_mapping(report.get("hot_path"), f"{path}.hot_path")
    for field in ["ocr", "parsing", "heavy_model_inference", "spawn_per_query"]:
        require_bool(hot_path.get(field), False, f"{path}.hot_path.{field}")

    latency = require_mapping(report.get("latency"), f"{path}.latency")
    p95 = require_mapping(latency.get("p95_ms"), f"{path}.latency.p95_ms")
    p99 = require_mapping(latency.get("p99_ms"), f"{path}.latency.p99_ms")
    p95_limits = require_mapping(matrix.get("latency_p95_ms", {}).get(scale_gate), f"matrix.latency_p95_ms.{scale_gate}")
    p99_limits = require_mapping(matrix.get("latency_p99_ms", {}).get(scale_gate), f"matrix.latency_p99_ms.{scale_gate}")
    for bucket in REQUIRED_BUCKETS:
        require_number_at_most(p95.get(bucket), p95_limits[bucket], f"{path}.latency.p95_ms.{bucket}")
        require_number_at_most(p99.get(bucket), p99_limits[bucket], f"{path}.latency.p99_ms.{bucket}")
    stage = require_mapping(latency.get("stage_p95_ms"), f"{path}.latency.stage_p95_ms")
    for name, limit in require_mapping(matrix.get("stage_p95_ms"), "matrix.stage_p95_ms").items():
        require_number_at_most(stage.get(name), limit, f"{path}.latency.stage_p95_ms.{name}")

    resources = require_mapping(report.get("resources"), f"{path}.resources")
    require_number_at_most(
        resources.get("private_or_anonymous_peak_mb"),
        matrix["import_redlines"]["daemon_private_or_anonymous_peak_mb"],
        f"{path}.resources.private_or_anonymous_peak_mb",
    )

    profiling = require_mapping(report.get("profiling"), f"{path}.profiling")
    require_bool(profiling.get("release_build"), True, f"{path}.profiling.release_build")
    require_number_at_least(profiling.get("repetitions"), matrix["profiling_redlines"]["repetitions_min"], f"{path}.profiling.repetitions")
    require_bool(
        profiling.get("coordinated_omission_corrected"),
        True,
        f"{path}.profiling.coordinated_omission_corrected",
    )
    require_number_at_most(
        profiling.get("observability_overhead_pct"),
        matrix["profiling_redlines"]["observability_overhead_pct_max"],
        f"{path}.profiling.observability_overhead_pct",
    )
    if not require_list(profiling.get("profiler_capture_refs"), f"{path}.profiling.profiler_capture_refs"):
        fail(f"{path}.profiling.profiler_capture_refs: must not be empty")

    incremental = require_mapping(report.get("import_incremental"), f"{path}.import_incremental")
    for key, limit_key in [
        ("first_file_searchable_p95_ms", "first_file_searchable_p95_ms"),
        ("directory_ttf100_ms", "directory_ttf100_ms"),
        ("all_volume_ttf100_ms", "all_volume_ttf100_ms"),
        ("ttf1000_ms", "ttf1000_ms"),
        ("single_file_modify_visible_p95_ms", "single_file_modify_visible_p95_ms"),
        ("single_file_modify_visible_p99_ms", "single_file_modify_visible_p99_ms"),
        ("delete_invisible_p95_ms", "delete_invisible_p95_ms"),
        ("burst_100_files_converged_ms", "burst_100_files_converged_ms"),
    ]:
        require_number_at_most(incremental.get(key), matrix["import_redlines"][limit_key], f"{path}.import_incremental.{key}")
    for key in [
        "rename_parse_count",
        "rename_body_rewrite_count",
        "zero_change_content_open_count",
        "zero_change_parse_count",
        "zero_change_index_mutation_count",
    ]:
        require_number_at_most(incremental.get(key), 0, f"{path}.import_incremental.{key}")

    validate_thresholds(report, path)


def validate_soak_fault(report: Mapping[str, object], matrix: Mapping[str, object], path: str) -> None:
    soak = require_mapping(report.get("soak_fault"), f"{path}.soak_fault")
    require_number_at_least(soak.get("duration_minutes"), matrix["soak_fault_redlines"]["duration_minutes_min"], f"{path}.soak_fault.duration_minutes")
    cases = set(require_list(soak.get("fault_cases"), f"{path}.soak_fault.fault_cases"))
    for required in ["daemon_restart", "cancel", "overload", "journal_gap"]:
        if required not in cases:
            fail(f"{path}.soak_fault.fault_cases missing {required}")


def validate_gui_manual(report: Mapping[str, object], matrix: Mapping[str, object], path: str) -> None:
    validate_gui_visual(report.get("gui_visual"), f"{path}.gui_visual")

    gui = require_mapping(report.get("gui_manual"), f"{path}.gui_manual")
    require_number_at_least(gui.get("logical_rows"), matrix["gui_redlines"]["representative_rows"], f"{path}.gui_manual.logical_rows")
    require_number_at_least(gui.get("visible_cards"), matrix["gui_redlines"]["desktop_reference_visible_cards"], f"{path}.gui_manual.visible_cards")
    require_number_at_most(gui.get("visible_cards"), matrix["gui_redlines"]["desktop_reference_visible_cards"], f"{path}.gui_manual.visible_cards")
    require_number_at_most(gui.get("buffered_cards"), matrix["gui_redlines"]["buffered_cards_max"], f"{path}.gui_manual.buffered_cards")
    require_enum(gui.get("navigation_mode"), set(matrix["gui_redlines"]["navigation_modes"]), f"{path}.gui_manual.navigation_mode")
    require_bool(gui.get("stable_card_height"), matrix["gui_redlines"]["stable_card_height_required"], f"{path}.gui_manual.stable_card_height")
    require_number_at_most(gui.get("input_to_paint_p95_ms"), matrix["gui_redlines"]["input_to_paint_p95_ms"], f"{path}.gui_manual.input_to_paint_p95_ms")
    require_number_at_most(gui.get("frame_time_p95_ms"), matrix["gui_redlines"]["frame_time_p95_ms"], f"{path}.gui_manual.frame_time_p95_ms")
    require_number_at_most(gui.get("scroll_dropped_frame_pct"), matrix["gui_redlines"]["scroll_dropped_frame_pct_max"], f"{path}.gui_manual.scroll_dropped_frame_pct")
    require_bool(gui.get("journey_checklist_passed"), True, f"{path}.gui_manual.journey_checklist_passed")


def validate_synthetic_smoke_matrix_contract(
    smoke: Mapping[str, object],
    matrix: Mapping[str, object],
    path: str,
) -> None:
    smoke_baseline = require_mapping(
        matrix.get("synthetic_smoke_baseline"),
        "matrix.synthetic_smoke_baseline",
    )
    for key in ["document_count", "query_count", "top_k"]:
        expected = smoke_baseline.get(key)
        if smoke.get(key) != expected:
            fail(f"{path}.{key}: expected matrix synthetic_smoke_baseline {expected}")
    if smoke.get("benchmark_command") != smoke_baseline.get("allowed_command"):
        fail(f"{path}.benchmark_command: expected matrix synthetic_smoke_baseline.allowed_command")
    if smoke.get("percentile_confidence") != smoke_baseline.get("percentile_confidence"):
        fail(f"{path}.percentile_confidence: expected matrix synthetic_smoke_baseline.percentile_confidence")


def validate_synthetic_smoke_components(value: object, path: str) -> None:
    reports = require_list(value, path)
    if len(reports) != len(SYNTHETIC_SMOKE_COMPONENTS):
        fail(f"{path}: expected exactly {len(SYNTHETIC_SMOKE_COMPONENTS)} components")
    seen = set()
    for index, entry in enumerate(reports):
        component = require_mapping(entry, f"{path}[{index}]")
        require_exact_keys(
            component,
            {"component", "schema_version", "report_sha256", "report_size_bytes", "target_claim"},
            f"{path}[{index}]",
        )
        name = component.get("component")
        require_enum(name, SYNTHETIC_SMOKE_COMPONENTS, f"{path}[{index}].component")
        if name in seen:
            fail(f"{path}[{index}].component: duplicate component {name!r}")
        seen.add(name)
        require_non_empty_string(component.get("schema_version"), f"{path}[{index}].schema_version")
        require_nonzero_hex64(component.get("report_sha256"), f"{path}[{index}].report_sha256")
        require_number_at_least(component.get("report_size_bytes"), 1, f"{path}[{index}].report_size_bytes")
        target_claim = component.get("target_claim")
        if target_claim != "not_evaluated":
            fail(f"{path}[{index}].target_claim: expected 'not_evaluated'")
    if seen != SYNTHETIC_SMOKE_COMPONENTS:
        fail(f"{path}: expected components {sorted(SYNTHETIC_SMOKE_COMPONENTS)}, got {sorted(seen)}")


def validate_synthetic_smoke_observations(value: object, path: str) -> None:
    observations = require_mapping(value, path)
    require_exact_keys(
        observations,
        {
            "uses_private_resume_root",
            "uses_query_artifact_root",
            "uses_synthetic_public_fixtures",
            "resident_daemon_required",
            "resident_daemon_observed",
            "batch_protocol_observed",
            "private_query_runner_query_protocol",
            "private_query_runner_request_sample_count",
            "private_query_runner_query_embedding_command_invocations",
            "spawn_per_query",
        },
        path,
    )
    for field in [
        "uses_private_resume_root",
        "uses_query_artifact_root",
        "resident_daemon_required",
        "resident_daemon_observed",
        "spawn_per_query",
    ]:
        require_bool(observations.get(field), False, f"{path}.{field}")
    require_bool(observations.get("uses_synthetic_public_fixtures"), True, f"{path}.uses_synthetic_public_fixtures")
    require_bool(observations.get("batch_protocol_observed"), True, f"{path}.batch_protocol_observed")
    if observations.get("private_query_runner_query_protocol") != "resume-ir-query-v2":
        fail(f"{path}.private_query_runner_query_protocol mismatch")
    if observations.get("private_query_runner_request_sample_count") != 2:
        fail(f"{path}.private_query_runner_request_sample_count: expected 2")
    if observations.get("private_query_runner_query_embedding_command_invocations") != 2:
        fail(f"{path}.private_query_runner_query_embedding_command_invocations: expected 2")


def validate_synthetic_smoke_report(report: Mapping[str, object], matrix: Mapping[str, object], path: str) -> None:
    require_exact_keys(report, SYNTHETIC_SMOKE_TOP_LEVEL_KEYS, path)
    if report.get("report_kind") != "redacted_evidence":
        fail(f"{path}.report_kind: synthetic smoke baseline must be redacted_evidence")
    if report.get("claim") != "no_claim":
        fail(f"{path}.claim: synthetic smoke baseline must use no_claim")
    if report.get("evidence_lane") != "smoke":
        fail(f"{path}.evidence_lane: synthetic smoke baseline must use smoke")
    validate_current_file_contract_pins(report.get("contract_pins"), f"{path}.contract_pins")
    validate_privacy(report, trace_required=True, path=path)
    validate_thresholds(report, path)
    thresholds = require_mapping(report.get("thresholds"), f"{path}.thresholds")
    require_bool(thresholds.get("passed"), True, f"{path}.thresholds.passed")

    smoke = require_mapping(report.get("synthetic_smoke"), f"{path}.synthetic_smoke")
    require_exact_keys(smoke, SYNTHETIC_SMOKE_KEYS, f"{path}.synthetic_smoke")
    if smoke.get("smoke_schema_version") != "resume-ir.synthetic-smoke-baseline.v1":
        fail(f"{path}.synthetic_smoke.smoke_schema_version mismatch")
    if smoke.get("source") != "synthetic_public_fixture":
        fail(f"{path}.synthetic_smoke.source: expected synthetic_public_fixture")
    if smoke.get("batch_protocol_request_count") != 2:
        fail(f"{path}.synthetic_smoke.batch_protocol_request_count: expected 2")
    validate_synthetic_smoke_matrix_contract(smoke, matrix, f"{path}.synthetic_smoke")
    validate_synthetic_smoke_components(smoke.get("component_reports"), f"{path}.synthetic_smoke.component_reports")
    validate_synthetic_smoke_observations(smoke.get("harness_observations"), f"{path}.synthetic_smoke.harness_observations")

    latency = require_mapping(smoke.get("latency_ms"), f"{path}.synthetic_smoke.latency_ms")
    require_exact_keys(
        latency,
        {"query_p95", "ocr_p95", "batch_protocol_stage"},
        f"{path}.synthetic_smoke.latency_ms",
    )
    require_number_at_least(latency.get("query_p95"), 0, f"{path}.synthetic_smoke.latency_ms.query_p95")
    require_number_at_least(latency.get("ocr_p95"), 0, f"{path}.synthetic_smoke.latency_ms.ocr_p95")
    protocol_stage = require_mapping(
        latency.get("batch_protocol_stage"),
        f"{path}.synthetic_smoke.latency_ms.batch_protocol_stage",
    )
    require_exact_keys(
        protocol_stage,
        SYNTHETIC_SMOKE_BATCH_PROTOCOL_STAGES,
        f"{path}.synthetic_smoke.latency_ms.batch_protocol_stage",
    )
    for stage in SYNTHETIC_SMOKE_BATCH_PROTOCOL_STAGES:
        require_number_at_least(
            protocol_stage.get(stage),
            0,
            f"{path}.synthetic_smoke.latency_ms.batch_protocol_stage.{stage}",
        )
    resource_observations = require_mapping(
        smoke.get("resource_observations"),
        f"{path}.synthetic_smoke.resource_observations",
    )
    require_exact_keys(
        resource_observations,
        {"batch_protocol_rss_delta_mb", "private_query_runner_rss_delta_mb"},
        f"{path}.synthetic_smoke.resource_observations",
    )
    require_number_at_least(
        resource_observations.get("batch_protocol_rss_delta_mb"),
        0,
        f"{path}.synthetic_smoke.resource_observations.batch_protocol_rss_delta_mb",
    )
    private_query_rss = require_mapping(
        resource_observations.get("private_query_runner_rss_delta_mb"),
        f"{path}.synthetic_smoke.resource_observations.private_query_runner_rss_delta_mb",
    )
    require_exact_keys(
        private_query_rss,
        {"samples", "min", "mean", "p50", "p95", "p99", "max"},
        f"{path}.synthetic_smoke.resource_observations.private_query_runner_rss_delta_mb",
    )
    require_number_at_least(
        private_query_rss.get("samples"),
        1,
        f"{path}.synthetic_smoke.resource_observations.private_query_runner_rss_delta_mb.samples",
    )
    for field in ["min", "mean", "p50", "p95", "p99", "max"]:
        require_number_at_least(
            private_query_rss.get(field),
            0,
            f"{path}.synthetic_smoke.resource_observations.private_query_runner_rss_delta_mb.{field}",
        )
    quality = require_mapping(smoke.get("quality"), f"{path}.synthetic_smoke.quality")
    require_exact_keys(
        quality,
        {"vector_recall_at_k", "vector_mrr", "vector_ndcg_at_k", "zero_result_queries", "zero_recall_queries"},
        f"{path}.synthetic_smoke.quality",
    )
    for field in ["vector_recall_at_k", "vector_mrr", "vector_ndcg_at_k"]:
        require_number_at_least(quality.get(field), 0, f"{path}.synthetic_smoke.quality.{field}")
        require_number_at_most(quality.get(field), 1, f"{path}.synthetic_smoke.quality.{field}")
    require_number_at_most(quality.get("zero_result_queries"), 0, f"{path}.synthetic_smoke.quality.zero_result_queries")
    require_number_at_most(quality.get("zero_recall_queries"), 0, f"{path}.synthetic_smoke.quality.zero_recall_queries")


def validate_synthetic_smoke_artifact_manifest(value: object, path: str) -> Mapping[str, object]:
    manifest = require_mapping(value, path)
    require_exact_keys(
        manifest,
        {
            "schema_version",
            "goal_id",
            "manifest_kind",
            "report_schema_version",
            "report_kind",
            "evidence_lane",
            "claim",
            "contract_pins",
            "privacy",
            "report_sha256",
            "report_size_bytes",
            "artifacts",
        },
        path,
    )
    if manifest.get("schema_version") != "resume-ir.synthetic-smoke-artifact-manifest.v1":
        fail(f"{path}.schema_version mismatch")
    if manifest.get("goal_id") != "resume-ir.performance-gui-loop.2026-06":
        fail(f"{path}.goal_id mismatch")
    if manifest.get("manifest_kind") != "synthetic_smoke_baseline":
        fail(f"{path}.manifest_kind mismatch")
    if manifest.get("report_schema_version") != "resume-ir.experiment-report.v2":
        fail(f"{path}.report_schema_version mismatch")
    if manifest.get("report_kind") != "redacted_evidence":
        fail(f"{path}.report_kind mismatch")
    if manifest.get("evidence_lane") != "smoke":
        fail(f"{path}.evidence_lane mismatch")
    if manifest.get("claim") != "no_claim":
        fail(f"{path}.claim mismatch")
    validate_current_file_contract_pins(manifest.get("contract_pins"), f"{path}.contract_pins")
    validate_privacy(manifest, trace_required=True, path=path)
    require_nonzero_hex64(manifest.get("report_sha256"), f"{path}.report_sha256")
    require_number_at_least(manifest.get("report_size_bytes"), 1, f"{path}.report_size_bytes")
    validate_synthetic_smoke_components(manifest.get("artifacts"), f"{path}.artifacts")
    return manifest


def validate_synthetic_smoke_report_manifest_pair(
    report: Mapping[str, object],
    manifest: Mapping[str, object],
    report_path: pathlib.Path,
    manifest_path: pathlib.Path,
    matrix: Mapping[str, object],
) -> None:
    report_rel = str(report_path.relative_to(ROOT)) if report_path.is_relative_to(ROOT) else str(report_path)
    manifest_rel = str(manifest_path.relative_to(ROOT)) if manifest_path.is_relative_to(ROOT) else str(manifest_path)
    validate_synthetic_smoke_report(report, matrix, report_rel)
    validate_synthetic_smoke_artifact_manifest(manifest, manifest_rel)
    digest = sha256_file(report_path)
    if manifest.get("report_sha256") != digest:
        fail(f"{manifest_rel}.report_sha256: expected {digest} from {report_rel}")
    size = report_path.stat().st_size
    if manifest.get("report_size_bytes") != size:
        fail(f"{manifest_rel}.report_size_bytes: expected {size} from {report_rel}")
    for key in ["report_kind", "evidence_lane", "claim", "contract_pins", "privacy"]:
        if manifest.get(key) != report.get(key):
            fail(f"{manifest_rel}.{key}: must match {report_rel}.{key}")
    report_smoke = require_mapping(report.get("synthetic_smoke"), f"{report_rel}.synthetic_smoke")
    if manifest.get("artifacts") != report_smoke.get("component_reports"):
        fail(f"{manifest_rel}.artifacts: must match {report_rel}.synthetic_smoke.component_reports")


def is_synthetic_smoke_report(value: object) -> bool:
    return (
        isinstance(value, Mapping)
        and value.get("schema_version") == "resume-ir.experiment-report.v2"
        and value.get("evidence_lane") == "smoke"
        and "synthetic_smoke" in value
    )


def is_synthetic_smoke_manifest(value: object) -> bool:
    return isinstance(value, Mapping) and value.get("schema_version") == "resume-ir.synthetic-smoke-artifact-manifest.v1"


def validate_synthetic_smoke_fixture_pairs(
    paths_and_values: list[tuple[pathlib.Path, object]],
    matrix: Mapping[str, object],
) -> None:
    reports: list[tuple[pathlib.Path, Mapping[str, object]]] = []
    manifests: list[tuple[pathlib.Path, Mapping[str, object]]] = []
    for path, value in paths_and_values:
        if is_synthetic_smoke_report(value):
            reports.append((path, require_mapping(value, str(path))))
        elif is_synthetic_smoke_manifest(value):
            manifests.append((path, require_mapping(value, str(path))))

    for manifest_path, manifest in manifests:
        matches = [
            (report_path, report)
            for report_path, report in reports
            if manifest.get("report_sha256") == sha256_file(report_path)
        ]
        if len(matches) != 1:
            manifest_rel = str(manifest_path.relative_to(ROOT)) if manifest_path.is_relative_to(ROOT) else str(manifest_path)
            fail(f"{manifest_rel}: expected exactly one matching synthetic smoke report")
        report_path, report = matches[0]
        validate_synthetic_smoke_report_manifest_pair(report, manifest, report_path, manifest_path, matrix)

    for report_path, report in reports:
        matches = [
            manifest_path
            for manifest_path, manifest in manifests
            if manifest.get("report_sha256") == sha256_file(report_path)
        ]
        if len(matches) != 1:
            report_rel = str(report_path.relative_to(ROOT)) if report_path.is_relative_to(ROOT) else str(report_path)
            fail(f"{report_rel}: synthetic smoke report requires exactly one matching manifest")


def validate_experiment_report(value: object, matrix: Mapping[str, object], path: str) -> None:
    report = require_mapping(value, path)
    if report.get("schema_version") != "resume-ir.experiment-report.v2":
        fail(f"{path}.schema_version mismatch")
    if report.get("goal_id") != "resume-ir.performance-gui-loop.2026-06":
        fail(f"{path}.goal_id mismatch")
    if report.get("report_kind") not in {"schema_fixture", "redacted_evidence"}:
        fail(f"{path}.report_kind invalid")
    if report.get("claim") not in {"no_claim", "blocked", "slice_complete"}:
        fail(f"{path}.claim invalid")
    validate_contract_pins(report.get("contract_pins"), f"{path}.contract_pins")
    validate_privacy(report, trace_required=True, path=path)
    lane = report.get("evidence_lane")
    if lane == "w0_docs":
        validate_w0_report(report, matrix, path)
    elif lane == "w1_private":
        validate_w1_report(report, matrix, path)
    elif lane == "soak_fault":
        validate_soak_fault(report, matrix, path)
        validate_thresholds(report, path)
    elif lane == "gui_manual":
        validate_gui_manual(report, matrix, path)
        validate_thresholds(report, path)
    elif lane == "smoke":
        validate_synthetic_smoke_report(report, matrix, path)
    else:
        fail(f"{path}.evidence_lane invalid")


def validate_loop_state(value: object, matrix: Mapping[str, object], path: str) -> None:
    state = require_mapping(value, path)
    if "accepted_cells" in state:
        fail(f"{path}.accepted_cells: legacy field removed; use evidence_cells")
    for field in sorted(REMOVED_LOOP_POLICY_FIELDS):
        if field in state:
            fail(f"{path}.{field}: policy mirror removed; keep policy in ACTIVE_GOAL.toml")
    if state.get("schema_version") != "resume-ir.loop-state-report.v2":
        fail(f"{path}.schema_version mismatch")
    if state.get("goal_id") != "resume-ir.performance-gui-loop.2026-06":
        fail(f"{path}.goal_id mismatch")
    validate_contract_pins(state.get("contract_pins"), f"{path}.contract_pins")
    validate_privacy(state, trace_required=False, path=path)
    verification = require_mapping(state.get("verification"), f"{path}.verification")
    claim = verification.get("claim")
    if claim not in {"pass", "fail", "blocked", "partial"}:
        fail(f"{path}.verification.claim invalid")
    commands = require_list(verification.get("commands"), f"{path}.verification.commands")
    workflow = state.get("workflow_state")
    experiment = state.get("experiment_state")
    if workflow == "goal_complete":
        if experiment != "complete":
            fail(f"{path}: goal_complete requires experiment_state=complete")
        if claim != "pass":
            fail(f"{path}: goal_complete requires verification.claim=pass")
        require_bool(verification.get("all_required_commands_ran"), True, f"{path}.verification.all_required_commands_ran")
        if not commands:
            fail(f"{path}: goal_complete requires at least one command")
        evidence_cells = require_list(state.get("evidence_cells"), f"{path}.evidence_cells")
        cell_names = set()
        for index, cell in enumerate(evidence_cells):
            evidence_cell = require_mapping(cell, f"{path}.evidence_cells[{index}]")
            cell_name = evidence_cell.get("cell")
            if not isinstance(cell_name, str) or not cell_name:
                fail(f"{path}.evidence_cells[{index}].cell: expected non-empty string")
            cell_names.add(cell_name)
            require_main_reachable_commit(
                evidence_cell.get("main_reachable_commit"),
                f"{path}.evidence_cells[{index}].main_reachable_commit",
            )
        missing = required_completion_cells(matrix) - cell_names
        if missing:
            fail(f"{path}.evidence_cells missing {sorted(missing)}")
    if experiment in {"hypothesis_registered", "accepted", "reverted", "complete"}:
        hypothesis = require_mapping(state.get("hypothesis"), f"{path}.hypothesis")
        for key in ["id", "acceptance_cell", "expected_effect", "before_measurement_ref"]:
            if not hypothesis.get(key):
                fail(f"{path}.hypothesis.{key}: required")
        if experiment in {"accepted", "reverted", "complete"}:
            for key in ["after_measurement_ref", "reprofile_ref", "decision"]:
                if not hypothesis.get(key):
                    fail(f"{path}.hypothesis.{key}: required")


def validate_fixture(path: pathlib.Path, matrix: Mapping[str, object]) -> None:
    value = load_json(path)
    if isinstance(value, Mapping) and value.get("schema_version") == "resume-ir.synthetic-smoke-artifact-manifest.v1":
        validate_synthetic_smoke_artifact_manifest(value, str(path.relative_to(ROOT)))
    elif path.name.startswith("loop") or "loop-state" in path.name:
        validate_loop_state(value, matrix, str(path.relative_to(ROOT)))
    else:
        validate_experiment_report(value, matrix, str(path.relative_to(ROOT)))


def main() -> int:
    matrix = load_toml(PERF / "acceptance-matrix.toml")
    validate_matrix(matrix)
    mixed_module = runpy.run_path(str(ROOT / "scripts" / "ci" / "check-mixed-import-contracts.py"))
    mixed_main = mixed_module.get("main")
    if not callable(mixed_main):
        fail("mixed-import contract check: missing callable main")
    try:
        mixed_main()
    except (OSError, ValueError) as exc:
        fail(f"mixed-import contract check failed: {exc}")
    validate_experiment_report_schema(
        require_mapping(load_json(PERF / "experiment-report.schema.json"), "experiment schema"),
        matrix,
    )
    validate_schema_file(
        require_mapping(load_json(PERF / "loop-state.schema.json"), "loop schema"),
        "perf/loop-state.schema.json",
        "resume-ir.loop-state-report.v2",
    )
    validate_synthetic_smoke_manifest_schema(
        require_mapping(load_json(PERF / "synthetic-smoke-artifact-manifest.schema.json"), "synthetic smoke manifest schema")
    )
    current_loop_state = require_mapping(load_json(PERF / "current-loop-state.json"), "perf/current-loop-state.json")
    validate_loop_state(current_loop_state, matrix, "perf/current-loop-state.json")
    validate_current_loop_contract_pins(current_loop_state)

    valid_fixture_values = []
    for path in sorted(VALID_FIXTURES.glob("*.json")):
        value = load_json(path)
        validate_fixture(path, matrix)
        valid_fixture_values.append((path, value))
    validate_synthetic_smoke_fixture_pairs(valid_fixture_values, matrix)

    invalid_count = 0
    for path in sorted(INVALID_FIXTURES.glob("*.json")):
        invalid_count += 1
        value = load_json(path)
        try:
            validate_fixture(path, matrix)
            validate_synthetic_smoke_fixture_pairs([(path, value)], matrix)
        except ValueError:
            continue
        fail(f"{path.relative_to(ROOT)}: invalid fixture unexpectedly passed")
    if invalid_count == 0:
        fail("no invalid fixtures found")

    print("performance contract check passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"performance contract check failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
