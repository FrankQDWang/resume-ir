#!/usr/bin/env python3
"""Validate autonomous delivery permissions in ACTIVE_GOAL.toml."""

from __future__ import annotations

import importlib.util
import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[2]


def fail(message: str) -> None:
    raise ValueError(message)


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


def require_bool(value: object, expected: bool, path: str) -> None:
    if value is not expected:
        fail(f"{path}: expected {expected}")


def require_string(value: object, expected: str, path: str) -> None:
    if value != expected:
        fail(f"{path}: expected {expected!r}")


def require_list(value: object, path: str) -> list:
    if not isinstance(value, list):
        fail(f"{path}: expected list")
    return value


def require_non_empty_string(value: object, path: str) -> None:
    if not isinstance(value, str) or not value:
        fail(f"{path}: expected non-empty string")


def require_transition(transitions: list, name: str) -> dict:
    for index, transition in enumerate(transitions):
        if isinstance(transition, dict) and transition.get("name") == name:
            return transition
    fail(f"autonomous_delivery.transitions: missing {name}")


def require_transition_shape(
    transitions: list,
    *,
    name: str,
    expected_from: list[str],
    expected_to: str,
    required_evidence: list[str],
) -> None:
    transition = require_transition(transitions, name)
    if transition.get("from") != expected_from:
        fail(f"autonomous_delivery.transitions.{name}.from: expected {expected_from!r}")
    require_string(
        transition.get("to"),
        expected_to,
        f"autonomous_delivery.transitions.{name}.to",
    )
    evidence = require_list(
        transition.get("required_evidence"),
        f"autonomous_delivery.transitions.{name}.required_evidence",
    )
    for expected in required_evidence:
        if expected not in evidence:
            fail(
                f"autonomous_delivery.transitions.{name}.required_evidence: "
                f"missing {expected}"
            )


def require_transition_contains(
    transitions: list,
    *,
    name: str,
    required_permissions: set[str] | None = None,
    required_evidence: set[str] | None = None,
) -> dict:
    transition = require_transition(transitions, name)
    if required_permissions is not None:
        permissions = set(
            require_list(
                transition.get("required_permissions"),
                f"autonomous_delivery.transitions.{name}.required_permissions",
            )
        )
        if not required_permissions.issubset(permissions):
            missing = sorted(required_permissions - permissions)
            fail(
                f"autonomous_delivery.transitions.{name}.required_permissions: "
                f"missing {missing}"
            )
    if required_evidence is not None:
        evidence = set(
            require_list(
                transition.get("required_evidence"),
                f"autonomous_delivery.transitions.{name}.required_evidence",
            )
        )
        if not required_evidence.issubset(evidence):
            missing = sorted(required_evidence - evidence)
            fail(
                f"autonomous_delivery.transitions.{name}.required_evidence: "
                f"missing {missing}"
            )
    return transition


def main() -> int:
    contracts = load_contracts_module()
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
    matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
    if "status" in active_goal:
        fail("ACTIVE_GOAL.toml.status: legacy top-level field removed; current state belongs in perf/current-loop-state.json")
    autonomous = active_goal.get("autonomous_delivery")
    if not isinstance(autonomous, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery]")

    permissions = autonomous.get("permissions")
    if not isinstance(permissions, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.permissions]")

    for key in [
        "production_code_allowed",
        "private_benchmark_allowed",
        "private_resume_root_read_allowed",
        "private_mixed_source_root_read_allowed",
        "seektalent_artifacts_query_read_allowed",
        "github_issue_write_allowed",
        "github_pr_write_allowed",
        "commit_push_allowed",
        "auto_merge_allowed",
        "branch_cleanup_allowed",
    ]:
        require_bool(permissions.get(key), True, f"autonomous_delivery.permissions.{key}")

    for key in [
        "direct_main_push_allowed",
        "admin_bypass_allowed",
        "raw_private_data_commit_allowed",
        "raw_query_commit_allowed",
        "gate_bypass_allowed",
        "threshold_relaxation_allowed",
    ]:
        require_bool(permissions.get(key), False, f"autonomous_delivery.permissions.{key}")

    active_slice = active_goal.get("scope", {}).get("active_slice", {})
    require_string(active_slice.get("issue"), "#143", "scope.active_slice.issue")
    require_string(
        active_slice.get("name"),
        "remove_parallel_redaction_counter_test_race",
        "scope.active_slice.name",
    )
    require_bool(active_slice.get("contract_change_allowed"), False, "scope.active_slice.contract_change_allowed")
    require_bool(active_slice.get("production_code_allowed"), True, "scope.active_slice.production_code_allowed")
    require_bool(
        active_slice.get("production_semantics_change_allowed"),
        False,
        "scope.active_slice.production_semantics_change_allowed",
    )
    require_bool(
        active_slice.get("test_only_change_required"),
        True,
        "scope.active_slice.test_only_change_required",
    )
    require_bool(
        active_slice.get("global_test_serialization_allowed"),
        False,
        "scope.active_slice.global_test_serialization_allowed",
    )
    require_bool(active_slice.get("private_benchmark_allowed"), False, "scope.active_slice.private_benchmark_allowed")
    require_bool(active_slice.get("scope_exception"), False, "scope.active_slice.scope_exception")
    require_non_empty_string(active_slice.get("scope_exception_reason"), "scope.active_slice.scope_exception_reason")
    allowed_paths = require_list(active_slice.get("allowed_paths"), "scope.active_slice.allowed_paths")
    expected_allowed_paths = ["crates/index-fulltext/src/lib.rs", "PROGRESS.md"]
    if allowed_paths != expected_allowed_paths:
        fail(
            "scope.active_slice.allowed_paths: expected "
            f"{expected_allowed_paths!r}"
        )
    allowed_transition_targets = require_list(
        active_slice.get("allowed_contract_transition_targets"),
        "scope.active_slice.allowed_contract_transition_targets",
    )
    if allowed_transition_targets != ["#140"]:
        fail(
            "scope.active_slice.allowed_contract_transition_targets: "
            "expected ['#140']"
        )

    gui = active_goal.get("gui")
    if not isinstance(gui, dict):
        fail("ACTIVE_GOAL.toml: missing [gui]")
    require_bool(gui.get("toolkit_bakeoff_default_required"), False, "gui.toolkit_bakeoff_default_required")
    require_string(gui.get("visual_reference"), "UI-reference/", "gui.visual_reference")

    gui_stack = matrix.get("gui_stack")
    if not isinstance(gui_stack, dict):
        fail("perf/acceptance-matrix.toml: missing [gui_stack]")
    require_string(gui_stack.get("default_stack"), contracts.GUI_DEFAULT_STACK, "matrix.gui_stack.default_stack")
    require_bool(gui_stack.get("production_next_server_allowed"), False, "matrix.gui_stack.production_next_server_allowed")
    require_bool(
        gui_stack.get("toolkit_bakeoff_requires_blocker_issue"),
        True,
        "matrix.gui_stack.toolkit_bakeoff_requires_blocker_issue",
    )
    require_string(gui_stack.get("visual_reference_role"), contracts.GUI_REFERENCE_ROLE, "matrix.gui_stack.visual_reference_role")

    platform_lanes = active_goal.get("platform_lanes")
    if not isinstance(platform_lanes, dict):
        fail("ACTIVE_GOAL.toml: missing [platform_lanes]")

    matrix_platform_lanes = matrix.get("platform_lanes")
    if not isinstance(matrix_platform_lanes, dict):
        fail("perf/acceptance-matrix.toml: missing [platform_lanes]")
    if matrix_platform_lanes.get("allowed") != contracts.PLATFORM_LANES:
        fail("matrix.platform_lanes.allowed mismatch")
    require_string(matrix_platform_lanes.get("primary_discovery"), contracts.PLATFORM_LANES[0], "matrix.platform_lanes.primary_discovery")
    require_string(matrix_platform_lanes.get("weak_host_validation"), contracts.PLATFORM_LANES[1], "matrix.platform_lanes.weak_host_validation")
    require_string(matrix_platform_lanes.get("ci_smoke"), contracts.PLATFORM_LANES[2], "matrix.platform_lanes.ci_smoke")
    require_bool(
        matrix_platform_lanes.get("macos_m4_can_close_windows_gate"),
        False,
        "matrix.platform_lanes.macos_m4_can_close_windows_gate",
    )
    require_bool(
        matrix_platform_lanes.get("cross_os_ci_smoke_can_replace_weak_host_perf"),
        False,
        "matrix.platform_lanes.cross_os_ci_smoke_can_replace_weak_host_perf",
    )

    private_corpus_transfer = platform_lanes.get("private_corpus_transfer")
    if not isinstance(private_corpus_transfer, dict):
        fail("ACTIVE_GOAL.toml: missing [platform_lanes.private_corpus_transfer]")
    require_bool(
        private_corpus_transfer.get("runner_may_choose_transfer_to_windows"),
        True,
        "platform_lanes.private_corpus_transfer.runner_may_choose_transfer_to_windows",
    )
    require_bool(
        private_corpus_transfer.get("transfer_public_evidence_allowed"),
        False,
        "platform_lanes.private_corpus_transfer.transfer_public_evidence_allowed",
    )
    require_bool(
        private_corpus_transfer.get("raw_private_paths_public_allowed"),
        False,
        "platform_lanes.private_corpus_transfer.raw_private_paths_public_allowed",
    )
    require_string(
        private_corpus_transfer.get("public_source_name"),
        "$RESUME_IR_PRIVATE_RESUME_ROOT",
        "platform_lanes.private_corpus_transfer.public_source_name",
    )
    require_bool(
        private_corpus_transfer.get("windows_unavailable_starts_reconciliation"),
        True,
        "platform_lanes.private_corpus_transfer.windows_unavailable_starts_reconciliation",
    )

    activation = autonomous.get("activation")
    if not isinstance(activation, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.activation]")
    require_bool(activation.get("contract_foundation_merged"), True, "autonomous_delivery.activation.contract_foundation_merged")
    require_bool(
        activation.get("active_slice_contract_applies"),
        True,
        "autonomous_delivery.activation.active_slice_contract_applies",
    )
    require_bool(
        activation.get("future_autonomous_run_pre_authorized"),
        True,
        "autonomous_delivery.activation.future_autonomous_run_pre_authorized",
    )
    require_bool(
        activation.get("runtime_capability_attestation_required"),
        True,
        "autonomous_delivery.activation.runtime_capability_attestation_required",
    )

    human_policy = autonomous.get("human_intervention_policy")
    if not isinstance(human_policy, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.human_intervention_policy]")
    for key, expected in {
        "normal_path_human_confirmation_required": False,
        "routine_commit_push_pr_issue_confirmation_required": False,
        "routine_auto_merge_confirmation_required": False,
        "ask_human_instead_of_terminal_state_allowed": False,
        "terminal_state_instead_of_human_prompt": True,
    }.items():
        require_bool(human_policy.get(key), expected, f"autonomous_delivery.human_intervention_policy.{key}")
    if require_list(human_policy.get("allowed_mid_run_human_prompts"), "autonomous_delivery.human_intervention_policy.allowed_mid_run_human_prompts"):
        fail("autonomous_delivery.human_intervention_policy.allowed_mid_run_human_prompts: expected empty list")
    required_terminal_states = {
        "goal_complete",
        "blocked_external_retryable",
        "blocked_permission",
        "contract_conflict",
        "goal_unsatisfiable",
        "budget_exhausted",
        "aborted_by_policy",
        "contract_invalid",
    }
    if set(require_list(human_policy.get("terminal_states"), "autonomous_delivery.human_intervention_policy.terminal_states")) != required_terminal_states:
        fail("autonomous_delivery.human_intervention_policy.terminal_states mismatch")

    pr_budget = autonomous.get("pr_budget")
    if not isinstance(pr_budget, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.pr_budget]")
    require_bool(
        pr_budget.get("allow_scope_exception_auto_merge"),
        False,
        "autonomous_delivery.pr_budget.allow_scope_exception_auto_merge",
    )

    prompt = autonomous.get("goal_prompt")
    if not isinstance(prompt, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.goal_prompt]")
    if prompt.get("format_version") != 1:
        fail("autonomous_delivery.goal_prompt.format_version: expected 1")
    if prompt.get("max_chars") != 4000:
        fail("autonomous_delivery.goal_prompt.max_chars: expected 4000")
    require_string(prompt.get("compiler"), "scripts/loop/compile-goal-prompt.py", "autonomous_delivery.goal_prompt.compiler")
    require_bool(
        prompt.get("compiler_implemented_in_active_slice"),
        False,
        "autonomous_delivery.goal_prompt.compiler_implemented_in_active_slice",
    )
    for key in [
        "deterministic_serialization_required",
        "field_priority_required",
        "character_budget_required",
        "state_version_required",
        "state_hash_required",
        "policy_hash_required",
        "prompt_hash_required",
        "fail_on_over_budget",
        "untrusted_external_text_is_data",
        "one_transition_per_wake",
        "minimal_next_transition_only",
    ]:
        require_bool(prompt.get(key), True, f"autonomous_delivery.goal_prompt.{key}")
    for key in ["silent_truncation_allowed", "historical_details_in_prompt_allowed"]:
        require_bool(prompt.get(key), False, f"autonomous_delivery.goal_prompt.{key}")

    capability = autonomous.get("runtime_capability_attestation")
    if not isinstance(capability, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.runtime_capability_attestation]")
    for key in ["required", "permissions_are_policy_not_capability", "observe_before_act", "record_unavailable_capability"]:
        require_bool(capability.get(key), True, f"autonomous_delivery.runtime_capability_attestation.{key}")
    required_capabilities = {
        "workspace_write",
        "network",
        "github_read",
        "github_write",
        "git_push",
        "git_merge_or_auto_merge",
        "branch_protection_compatible",
        "private_resume_root_read",
        "seektalent_artifacts_query_read",
        "automation_scheduler",
    }
    if set(require_list(capability.get("required_capabilities"), "autonomous_delivery.runtime_capability_attestation.required_capabilities")) != required_capabilities:
        fail("autonomous_delivery.runtime_capability_attestation.required_capabilities mismatch")
    missing_states = set(require_list(capability.get("missing_capability_terminal_states"), "autonomous_delivery.runtime_capability_attestation.missing_capability_terminal_states"))
    if missing_states != {"blocked_external_retryable", "blocked_permission", "aborted_by_policy"}:
        fail("autonomous_delivery.runtime_capability_attestation.missing_capability_terminal_states mismatch")

    event_log = autonomous.get("event_log")
    if not isinstance(event_log, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.event_log]")
    require_string(
        event_log.get("events_path_template"),
        "perf/runs/<run_id>/events/<state_version>.json",
        "autonomous_delivery.event_log.events_path_template",
    )
    for key in [
        "required_for_runner_implementation",
        "append_only",
        "derived_current_state_only",
        "previous_event_hash_required",
        "idempotency_key_required",
        "intent_before_side_effect_required",
        "verify_after_side_effect_required",
        "compare_and_swap_state_update_required",
        "lease_required",
        "heartbeat_required",
    ]:
        require_bool(event_log.get(key), True, f"autonomous_delivery.event_log.{key}")
    require_bool(event_log.get("direct_current_state_edit_allowed"), False, "autonomous_delivery.event_log.direct_current_state_edit_allowed")

    transitions = require_list(autonomous.get("transitions"), "autonomous_delivery.transitions")
    transition_names: set[str] = set()
    for index, transition in enumerate(transitions):
        if not isinstance(transition, dict):
            fail(f"autonomous_delivery.transitions[{index}]: expected table")
        for key in ["name", "to"]:
            require_non_empty_string(transition.get(key), f"autonomous_delivery.transitions[{index}].{key}")
        name = transition["name"]
        if name in transition_names:
            fail(f"autonomous_delivery.transitions: duplicate transition {name!r}")
        transition_names.add(name)
        for key in ["from", "required_permissions", "required_evidence", "allowed_actions"]:
            values = require_list(transition.get(key), f"autonomous_delivery.transitions[{index}].{key}")
            if key in {"from", "allowed_actions"} and not values:
                fail(f"autonomous_delivery.transitions[{index}].{key}: expected non-empty list")

    require_transition_shape(
        transitions,
        name="capture_synthetic_smoke_baseline",
        expected_from=["goal_authorized", "blocked_permission"],
        expected_to="baseline_captured",
        required_evidence=[
            "synthetic_smoke_report",
            "synthetic_smoke_artifact_manifest",
            "privacy_boundary",
        ],
    )
    synthetic_transition = require_transition(transitions, "capture_synthetic_smoke_baseline")
    if synthetic_transition.get("required_permissions") != []:
        fail(
            "autonomous_delivery.transitions.capture_synthetic_smoke_baseline."
            "required_permissions: expected []"
        )
    atomic_transition = require_transition(transitions, "atomic_redaction_counter_test_fix")
    expected_atomic = {
        "from": ["slice_selected"],
        "to": "pr_opened",
        "required_permissions": ["production_code_allowed", "commit_push_allowed", "github_pr_write_allowed", "github_issue_write_allowed"],
        "required_evidence": ["explicit_user_authorization", "linked_issue", "issue_intent_comment", "exact_test_only_patch", "local_tests", "privacy_boundary"],
        "allowed_actions": ["edit_contract", "edit_exact_test", "run_tests", "commit", "push_branch", "create_pr", "update_issue"],
    }
    for key, expected in expected_atomic.items():
        if atomic_transition.get(key) != expected:
            fail(
                "autonomous_delivery.transitions.atomic_redaction_counter_test_fix."
                f"{key}: expected {expected!r}"
            )
    require_transition_contains(
        transitions,
        name="capture_baseline",
        required_permissions={"private_benchmark_allowed"},
        required_evidence={"baseline_command", "redacted_baseline_artifact"},
    )
    open_profile_issue = require_transition_contains(
        transitions,
        name="open_profile_issue",
        required_permissions={"github_issue_write_allowed"},
        required_evidence={"baseline_artifact", "privacy_boundary"},
    )
    if "baseline_captured" not in open_profile_issue.get("from", []):
        fail(
            "autonomous_delivery.transitions.open_profile_issue.from: "
            "baseline_captured is required"
        )

    loop_doc = (ROOT / "03_next_goal_高性能本地检索GUI闭环" / "13_Loop_Engineering状态机.md").read_text(encoding="utf-8")
    allowed_paths_source = "allowed_paths_source: ACTIVE_GOAL.toml [scope.active_slice].allowed_paths"
    if allowed_paths_source not in loop_doc:
        fail(
            "03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md: "
            f"missing {allowed_paths_source!r}"
        )
    if "allowed_paths_for_active_slice:" in loop_doc:
        fail(
            "03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md: "
            "inline allowed path list is a stale mirror; use ACTIVE_GOAL.toml [scope.active_slice].allowed_paths"
        )
    print("check-autonomous-goal.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-autonomous-goal.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
