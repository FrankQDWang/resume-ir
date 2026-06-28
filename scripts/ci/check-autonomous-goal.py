#!/usr/bin/env python3
"""Validate autonomous delivery permissions in ACTIVE_GOAL.toml."""

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


def main() -> int:
    contracts = load_contracts_module()
    active_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
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

    current_pr = active_goal.get("scope", {}).get("current_pr", {})
    require_bool(current_pr.get("contract_change_allowed"), True, "scope.current_pr.contract_change_allowed")
    require_bool(current_pr.get("production_code_allowed"), False, "scope.current_pr.production_code_allowed")
    require_bool(current_pr.get("private_benchmark_allowed"), False, "scope.current_pr.private_benchmark_allowed")
    require_bool(current_pr.get("scope_exception"), True, "scope.current_pr.scope_exception")
    require_bool(
        current_pr.get("scope_exception_auto_merge_allowed"),
        False,
        "scope.current_pr.scope_exception_auto_merge_allowed",
    )
    require_non_empty_string(current_pr.get("scope_exception_reason"), "scope.current_pr.scope_exception_reason")

    gui = active_goal.get("gui")
    if not isinstance(gui, dict):
        fail("ACTIVE_GOAL.toml: missing [gui]")
    require_string(gui.get("default_stack"), contracts.GUI_DEFAULT_STACK, "gui.default_stack")
    require_bool(gui.get("production_next_server_allowed"), False, "gui.production_next_server_allowed")
    require_bool(gui.get("toolkit_bakeoff_default_required"), False, "gui.toolkit_bakeoff_default_required")
    require_bool(gui.get("toolkit_bakeoff_requires_blocker_issue"), True, "gui.toolkit_bakeoff_requires_blocker_issue")
    require_string(gui.get("visual_reference_role"), contracts.GUI_REFERENCE_ROLE, "gui.visual_reference_role")
    require_string(gui.get("visual_reference"), "UI-reference/", "gui.visual_reference")

    platform_lanes = active_goal.get("platform_lanes")
    if not isinstance(platform_lanes, dict):
        fail("ACTIVE_GOAL.toml: missing [platform_lanes]")
    require_string(platform_lanes.get("primary_discovery"), contracts.PLATFORM_LANES[0], "platform_lanes.primary_discovery")
    require_string(platform_lanes.get("weak_host_validation"), contracts.PLATFORM_LANES[1], "platform_lanes.weak_host_validation")
    require_string(platform_lanes.get("ci_smoke"), contracts.PLATFORM_LANES[2], "platform_lanes.ci_smoke")
    require_bool(platform_lanes.get("macos_m4_can_close_windows_gate"), False, "platform_lanes.macos_m4_can_close_windows_gate")
    require_bool(
        platform_lanes.get("cross_os_ci_smoke_can_replace_weak_host_perf"),
        False,
        "platform_lanes.cross_os_ci_smoke_can_replace_weak_host_perf",
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
    require_bool(activation.get("current_pr_contract_only"), False, "autonomous_delivery.activation.current_pr_contract_only")
    require_bool(
        activation.get("applies_after_current_pr_merge"),
        True,
        "autonomous_delivery.activation.applies_after_current_pr_merge",
    )
    require_bool(
        activation.get("scope_current_pr_permissions_win_until_merge"),
        False,
        "autonomous_delivery.activation.scope_current_pr_permissions_win_until_merge",
    )
    require_bool(
        activation.get("production_implementation_in_current_pr_allowed"),
        False,
        "autonomous_delivery.activation.production_implementation_in_current_pr_allowed",
    )
    require_bool(
        activation.get("private_benchmark_in_current_pr_allowed"),
        False,
        "autonomous_delivery.activation.private_benchmark_in_current_pr_allowed",
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
    require_bool(
        activation.get("normal_path_human_confirmation_required"),
        False,
        "autonomous_delivery.activation.normal_path_human_confirmation_required",
    )

    pull_request = active_goal.get("pull_request")
    if not isinstance(pull_request, dict):
        fail("ACTIVE_GOAL.toml: missing [pull_request]")
    if pull_request.get("number") != 10:
        fail("pull_request.number: expected 10")
    require_string(pull_request.get("state"), "merged", "pull_request.state")
    require_bool(pull_request.get("draft"), False, "pull_request.draft")
    require_string(pull_request.get("review_status"), "merged", "pull_request.review_status")
    require_string(
        pull_request.get("semantic_role"),
        "autonomous_delivery_contract_foundation",
        "pull_request.semantic_role",
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

    prompt = autonomous.get("goal_prompt")
    if not isinstance(prompt, dict):
        fail("ACTIVE_GOAL.toml: missing [autonomous_delivery.goal_prompt]")
    if prompt.get("format_version") != 1:
        fail("autonomous_delivery.goal_prompt.format_version: expected 1")
    if prompt.get("max_chars") != 4000:
        fail("autonomous_delivery.goal_prompt.max_chars: expected 4000")
    require_string(prompt.get("compiler"), "scripts/loop/compile-goal-prompt.py", "autonomous_delivery.goal_prompt.compiler")
    require_bool(
        prompt.get("compiler_implemented_in_current_pr"),
        False,
        "autonomous_delivery.goal_prompt.compiler_implemented_in_current_pr",
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
    transition_names = {transition.get("name") for transition in transitions if isinstance(transition, dict)}
    for expected in [
        "capture_baseline",
        "open_profile_issue",
        "record_hypothesis",
        "select_slice",
        "activate_branch",
        "implement_slice",
        "verify_slice",
        "open_pr",
        "sync_base",
        "mark_review_ready",
        "mark_ci_green",
        "mark_local_gate_green",
        "mark_privacy_gate_green",
        "select_merge_method",
        "merge_pr",
        "reconcile_issue_lifecycle",
        "advance_to_next_issue_or_goal_complete",
    ]:
        if expected not in transition_names:
            fail(f"autonomous_delivery.transitions: missing {expected}")
    for index, transition in enumerate(transitions):
        if not isinstance(transition, dict):
            fail(f"autonomous_delivery.transitions[{index}]: expected table")
        for key in ["name", "to"]:
            require_non_empty_string(transition.get(key), f"autonomous_delivery.transitions[{index}].{key}")
        for key in ["from", "required_permissions", "required_evidence", "allowed_actions"]:
            values = require_list(transition.get(key), f"autonomous_delivery.transitions[{index}].{key}")
            if key in {"from", "allowed_actions"} and not values:
                fail(f"autonomous_delivery.transitions[{index}].{key}: expected non-empty list")

    reconcile_issue = require_transition(transitions, "reconcile_issue_lifecycle")
    if reconcile_issue.get("from") != ["pr_merged"]:
        fail("autonomous_delivery.transitions.reconcile_issue_lifecycle.from: expected ['pr_merged']")
    require_string(
        reconcile_issue.get("to"),
        "issue_reconciled_with_evidence",
        "autonomous_delivery.transitions.reconcile_issue_lifecycle.to",
    )
    required_evidence = require_list(
        reconcile_issue.get("required_evidence"),
        "autonomous_delivery.transitions.reconcile_issue_lifecycle.required_evidence",
    )
    for expected in ["main_reachable_commit", "issue_lifecycle_outcome", "before_after_metrics", "privacy_boundary"]:
        if expected not in required_evidence:
            fail(
                "autonomous_delivery.transitions.reconcile_issue_lifecycle.required_evidence: "
                f"missing {expected}"
            )

    advance_transition = require_transition(transitions, "advance_to_next_issue_or_goal_complete")
    if advance_transition.get("from") != ["issue_reconciled_with_evidence"]:
        fail(
            "autonomous_delivery.transitions.advance_to_next_issue_or_goal_complete.from: "
            "expected ['issue_reconciled_with_evidence']"
        )

    profile_template = (ROOT / ".github" / "ISSUE_TEMPLATE" / "profile_issue.md").read_text(encoding="utf-8")
    for phrase in [
        "Issue lifecycle after merge",
        "closed_here | same_lane_continues | follow_up_issue_linked",
        "Follow-up issue linked, if any:",
    ]:
        if phrase not in profile_template:
            fail(f".github/ISSUE_TEMPLATE/profile_issue.md: missing {phrase!r}")

    pr_template = (ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text(encoding="utf-8")
    if "The linked issue records a reconciled post-merge lifecycle outcome." not in pr_template:
        fail(".github/PULL_REQUEST_TEMPLATE.md: missing reconciled post-merge lifecycle readiness anchor")

    print("check-autonomous-goal.py passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"check-autonomous-goal.py failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
