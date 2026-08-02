#!/usr/bin/env python3
"""Validate the bounded public #319 resident role-isolation report."""

from __future__ import annotations

import copy
import json
import math
import pathlib
import re
import sys
from collections.abc import Mapping

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "perf" / "resident-embedding-role-isolation.schema.json"
FIXTURES = ROOT / "perf" / "fixtures" / "resident-embedding-role-isolation"
VALID = FIXTURES / "valid-public-report.json"
INVALID = FIXTURES / "invalid-cases.json"
ARMS = ("shared_i3_b4", "split_i3_bulk3_b4", "split_i3_bulk4_b4")
CANDIDATES = ARMS[1:]
CORRECTNESS = {
    "vectors_elementwise_exact", "counts_exact", "order_exact", "cancellation_exact",
    "timeout_exact", "restart_exact", "ready_exact", "cleanup_exact", "query_results_exact",
}
PRIVACY = {
    "contains_private_paths", "contains_filenames", "contains_resume_text",
    "contains_query_text", "contains_candidate_results", "contains_token_ids",
    "contains_vectors", "contains_pids", "contains_logs_or_traces",
    "contains_direct_raw_hashes", "contains_databases_or_indexes",
    "contains_model_or_runtime_bytes",
}
FIXED_WORKLOAD = {
    "model_id": "intfloat-multilingual-e5-small-qint8-r1",
    "upstream_revision": "614241f622f53c4eeff9890bdc4f31cfecc418b3",
    "tokenizer_sha256": "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    "runtime": "onnxruntime-1.27", "quantization": "dynamic_u8s8",
    "prepacking": False, "prefix": "passage",
    "pooling": "attention_mask_mean_then_l2", "input_policy": "whole_head_512",
    "max_tokens": 512, "dimension": 384, "bulk_batch": 4,
    "bulk_tokens_per_input": 512, "bulk_grouping": "fixed_groups_of_four",
    "bulk_order": "seeded_frozen", "bulk_saturated": True,
    "interactive_batch": 1, "interactive_tokens": 32, "interactive_qps": 2,
    "resident_lifetime": "daemon_lifetime",
    "split_reclaim": "signal_both_before_joining_either",
}


def fail(message: str) -> None:
    raise ValueError(message)


def load(path: pathlib.Path) -> object:
    with path.open("rb") as handle:
        return json.load(handle)


def mapping(value: object, label: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        fail(f"{label}: expected object")
    return value


def closed(value: object, keys: set[str], label: str) -> Mapping[str, object]:
    result = mapping(value, label)
    if set(result) != keys:
        fail(f"{label}: fields are not closed")
    return result


def count(value: object, label: str, *, positive: bool = False) -> int:
    minimum = 1 if positive else 0
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        fail(f"{label}: expected integer >= {minimum}")
    return value


def number(value: object, label: str, *, positive: bool = False) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{label}: expected finite number")
    result = float(value)
    if not math.isfinite(result) or (positive and result <= 0.0):
        fail(f"{label}: expected finite{' positive' if positive else ''} number")
    return result


def boolean(value: object, label: str) -> bool:
    if not isinstance(value, bool):
        fail(f"{label}: expected boolean")
    return value


def close(actual: float, expected: float, label: str) -> None:
    if not math.isclose(actual, expected, rel_tol=0.0, abs_tol=1e-6):
        fail(f"{label}: inconsistent aggregate")


def validate(report: object) -> None:
    root_keys = {"schema_version", "artifact_id", "issue", "source", "revision", "platform", "run", "fixed_workload", "arms", "comparisons", "decision", "privacy", "claims"}
    report = closed(report, root_keys, "report")
    try:
        encoded = json.dumps(report, allow_nan=False, ensure_ascii=True, separators=(",", ":"))
    except (TypeError, ValueError) as error:
        fail(f"report: not finite JSON: {error}")
    if len(encoded.encode("utf-8")) > 64 * 1024:
        fail("report: exceeds 64 KiB")
    if any(signal in encoded for signal in ("/Users/", "file://", "\\Users\\")):
        fail("report: contains path signal")
    identity = {
        "schema_version": "resume-ir.resident-embedding-role-isolation.v1",
        "artifact_id": "resident-embedding-role-isolation-issue-319",
        "issue": "#319", "source": "public_synthetic_daemon_sessions",
    }
    if any(report[key] != value for key, value in identity.items()):
        fail("report: identity mismatch")
    if not isinstance(report["revision"], str) or re.fullmatch(r"[0-9a-f]{40}", report["revision"]) is None:
        fail("revision: expected exact lowercase commit")
    platform = closed(report["platform"], {"os", "architecture", "machine", "governor", "memory_measurement"}, "platform")
    expected_platform = {"os": "macos", "architecture": "arm64", "machine": "M4", "governor": "H2_Aggressive", "memory_measurement": "process_tree_private_or_anonymous_peak_mib"}
    if platform != expected_platform:
        fail("platform: identity mismatch")
    run_keys = {"kind", "seed", "blocks", "sessions", "sessions_per_arm", "independent_release_daemon_sessions", "williams_balanced", "warmup_seconds", "measurement_seconds", "all_sessions_completed", "thermal_guard_passed", "host_load_guard_passed"}
    run = closed(report["run"], run_keys, "run")
    kind = run["kind"]
    if kind not in {"smoke", "formal_public_matrix"} or run["seed"] != 20260802:
        fail("run: identity mismatch")
    if run["independent_release_daemon_sessions"] is not True or run["williams_balanced"] is not True:
        fail("run: daemon-session design mismatch")
    blocks = count(run["blocks"], "run.blocks", positive=True)
    configured_sessions = count(run["sessions"], "run.sessions", positive=True)
    sessions_per_arm = count(run["sessions_per_arm"], "run.sessions_per_arm", positive=True)
    warmup = number(run["warmup_seconds"], "run.warmup_seconds", positive=True)
    measurement = number(run["measurement_seconds"], "run.measurement_seconds", positive=True)
    expected_shape = (1, 3, 1, 1.0, 1.0) if kind == "smoke" else (10, 30, 10, 30.0, 60.0)
    if (blocks, configured_sessions, sessions_per_arm, warmup, measurement) != expected_shape:
        fail("run: workload shape mismatch")
    run_guards = [boolean(run[key], f"run.{key}") for key in ("all_sessions_completed", "thermal_guard_passed", "host_load_guard_passed")]
    if closed(report["fixed_workload"], set(FIXED_WORKLOAD), "fixed_workload") != FIXED_WORKLOAD:
        fail("fixed_workload: mismatch")
    arms = closed(report["arms"], set(ARMS), "arms")
    expected_arm_identity = {
        "shared_i3_b4": ("shared", 3, 1),
        "split_i3_bulk3_b4": ("split", 3, 2),
        "split_i3_bulk4_b4": ("split", 4, 2),
    }
    arm_correct: dict[str, bool] = {}
    completed_sessions = 0
    for name in ARMS:
        arm = closed(arms[name], {"topology", "interactive_threads", "bulk_threads", "resident_count", "sessions", "bulk", "interactive", "resources", "correctness"}, f"arms.{name}")
        if (arm["topology"], arm["bulk_threads"], arm["resident_count"]) != expected_arm_identity[name] or arm["interactive_threads"] != 3:
            fail(f"arms.{name}: topology mismatch")
        sessions = count(arm["sessions"], f"arms.{name}.sessions", positive=True)
        if sessions > sessions_per_arm or (run["all_sessions_completed"] and sessions != sessions_per_arm):
            fail(f"arms.{name}: session count mismatch")
        completed_sessions += sessions
        bulk = closed(arm["bulk"], {"completed_batches", "completed_inputs", "mean_throughput_inputs_per_second"}, f"arms.{name}.bulk")
        batches = count(bulk["completed_batches"], f"arms.{name}.bulk.completed_batches", positive=True)
        if count(bulk["completed_inputs"], f"arms.{name}.bulk.completed_inputs", positive=True) != batches * 4:
            fail(f"arms.{name}.bulk: Batch 4 counts do not reconcile")
        number(bulk["mean_throughput_inputs_per_second"], f"arms.{name}.bulk.throughput", positive=True)
        interactive = closed(arm["interactive"], {"samples", "successes", "failures", "p50_ms", "p95_ms", "p99_ms", "max_queue_wait_upper_bound_ms"}, f"arms.{name}.interactive")
        samples = count(interactive["samples"], f"arms.{name}.interactive.samples", positive=True)
        expected_samples = round(measurement * 2) * sessions
        if samples != expected_samples:
            fail(f"arms.{name}.interactive: fixed 2 QPS sample count mismatch")
        successes = count(interactive["successes"], f"arms.{name}.interactive.successes")
        failures = count(interactive["failures"], f"arms.{name}.interactive.failures")
        if successes + failures != samples:
            fail(f"arms.{name}.interactive: outcomes do not reconcile")
        percentiles = [number(interactive[key], f"arms.{name}.interactive.{key}", positive=True) for key in ("p50_ms", "p95_ms", "p99_ms", "max_queue_wait_upper_bound_ms")]
        if percentiles != sorted(percentiles):
            fail(f"arms.{name}.interactive: latency aggregates are not monotonic")
        resources = closed(arm["resources"], {"process_tree_private_or_anonymous_peak_mib"}, f"arms.{name}.resources")
        number(resources["process_tree_private_or_anonymous_peak_mib"], f"arms.{name}.resources.peak", positive=True)
        correctness = closed(arm["correctness"], CORRECTNESS, f"arms.{name}.correctness")
        arm_correct[name] = failures == 0 and all(boolean(value, f"arms.{name}.correctness") for value in correctness.values())
    sessions_complete = completed_sessions == configured_sessions
    run_valid = all(run_guards) and sessions_complete
    comparisons = report["comparisons"]
    if not isinstance(comparisons, list) or len(comparisons) != 2:
        fail("comparisons: expected exactly two candidates")
    accepted: list[str] = []
    comparison_keys = {"control", "candidate", "paired_blocks", "bulk_improvement_percent", "bulk_paired_ci95_low_percent", "bulk_paired_ci95_high_percent", "query_p95_regression_percent", "query_p99_regression_percent", "max_queue_wait_upper_bound_ms", "process_tree_private_or_anonymous_peak_mib", "correctness_pass", "gates"}
    gate_keys = {"bulk_at_least_8_percent", "bulk_ci_positive", "query_p95_within_5_percent", "query_p99_within_10_percent", "queue_wait_within_200_ms", "resource_within_1536_mib", "correctness_exact", "accepted"}
    for index, candidate in enumerate(CANDIDATES):
        comparison = closed(comparisons[index], comparison_keys, f"comparisons.{index}")
        if comparison["control"] != ARMS[0] or comparison["candidate"] != candidate:
            fail("comparisons: order or identity mismatch")
        paired_blocks = count(comparison["paired_blocks"], f"comparisons.{candidate}.paired_blocks", positive=True)
        if paired_blocks > blocks or (run["all_sessions_completed"] and paired_blocks != blocks):
            fail(f"comparisons.{candidate}: paired block count mismatch")
        improvement = number(comparison["bulk_improvement_percent"], f"comparisons.{candidate}.bulk_improvement")
        ci_low = number(comparison["bulk_paired_ci95_low_percent"], f"comparisons.{candidate}.ci_low")
        ci_high = number(comparison["bulk_paired_ci95_high_percent"], f"comparisons.{candidate}.ci_high")
        if not ci_low <= improvement <= ci_high:
            fail(f"comparisons.{candidate}: confidence interval is inconsistent")
        p95_regression = number(comparison["query_p95_regression_percent"], f"comparisons.{candidate}.p95_regression")
        p99_regression = number(comparison["query_p99_regression_percent"], f"comparisons.{candidate}.p99_regression")
        control_interactive = arms[ARMS[0]]["interactive"]
        candidate_interactive = arms[candidate]["interactive"]
        close(p95_regression, (float(candidate_interactive["p95_ms"]) / float(control_interactive["p95_ms"]) - 1.0) * 100.0, f"comparisons.{candidate}.p95_regression")
        close(p99_regression, (float(candidate_interactive["p99_ms"]) / float(control_interactive["p99_ms"]) - 1.0) * 100.0, f"comparisons.{candidate}.p99_regression")
        queue_upper = number(comparison["max_queue_wait_upper_bound_ms"], f"comparisons.{candidate}.queue_upper", positive=True)
        memory_peak = number(comparison["process_tree_private_or_anonymous_peak_mib"], f"comparisons.{candidate}.memory_peak", positive=True)
        close(queue_upper, float(candidate_interactive["max_queue_wait_upper_bound_ms"]), f"comparisons.{candidate}.queue_upper")
        close(memory_peak, float(arms[candidate]["resources"]["process_tree_private_or_anonymous_peak_mib"]), f"comparisons.{candidate}.memory_peak")
        correctness_pass = arm_correct[ARMS[0]] and arm_correct[candidate]
        if comparison["correctness_pass"] is not correctness_pass:
            fail(f"comparisons.{candidate}: correctness mismatch")
        expected_gates = {
            "bulk_at_least_8_percent": improvement >= 8.0,
            "bulk_ci_positive": ci_low > 0.0,
            "query_p95_within_5_percent": p95_regression <= 5.0,
            "query_p99_within_10_percent": p99_regression <= 10.0,
            "queue_wait_within_200_ms": queue_upper <= 200.0,
            "resource_within_1536_mib": memory_peak <= 1536.0,
            "correctness_exact": correctness_pass,
        }
        expected_gates["accepted"] = kind == "formal_public_matrix" and run_valid and paired_blocks == blocks and all(expected_gates.values())
        if closed(comparison["gates"], gate_keys, f"comparisons.{candidate}.gates") != expected_gates:
            fail(f"comparisons.{candidate}: gate decision mismatch")
        if expected_gates["accepted"]:
            accepted.append(candidate)
    decision = closed(report["decision"], {"status", "winner", "private_matrix_eligible"}, "decision")
    if kind == "smoke":
        expected_decision = {"status": "smoke_pass" if run_valid and all(arm_correct.values()) else "smoke_failed", "winner": None, "private_matrix_eligible": False}
        expected_claims = ["capability_only", "no_product_speedup", "no_private_claim", "no_release_claim"]
    elif not run_valid or len(accepted) > 1:
        expected_decision = {"status": "inconclusive", "winner": None, "private_matrix_eligible": False}
        expected_claims = ["candidate_selection_only", "no_product_migration", "no_private_product_claim", "no_release_claim"]
    elif not accepted:
        expected_decision = {"status": "lost", "winner": None, "private_matrix_eligible": False}
        expected_claims = ["candidate_selection_only", "no_product_migration", "no_private_product_claim", "no_release_claim"]
    else:
        expected_decision = {"status": "won", "winner": accepted[0], "private_matrix_eligible": True}
        expected_claims = ["candidate_selection_only", "no_product_migration", "no_private_product_claim", "no_release_claim"]
    if decision != expected_decision:
        fail("decision: inconsistent with evidence")
    if any(value is not False for value in closed(report["privacy"], PRIVACY, "privacy").values()):
        fail("privacy: all leak flags must be false")
    if report["claims"] != expected_claims:
        fail("claims: unsupported claim set")


def mutate(report: object, mutation: str) -> object:
    candidate = copy.deepcopy(report)
    if mutation == "unknown_field": candidate["private"] = "payload"
    elif mutation == "identity": candidate["issue"] = "#320"
    elif mutation == "path_signal": candidate["artifact_id"] = "/Users/private/report"
    elif mutation == "oversized": candidate["artifact_id"] = "x" * (64 * 1024)
    elif mutation == "nan": candidate["comparisons"][0]["bulk_improvement_percent"] = float("nan")
    elif mutation == "run_shape": candidate["run"]["sessions"] = 29
    elif mutation == "arm_identity": candidate["arms"]["shared_i3_b4"]["resident_count"] = 2
    elif mutation == "batch_count": candidate["arms"]["shared_i3_b4"]["bulk"]["completed_inputs"] += 1
    elif mutation == "query_count": candidate["arms"]["shared_i3_b4"]["interactive"]["samples"] -= 1
    elif mutation == "percentile_order": candidate["arms"]["shared_i3_b4"]["interactive"]["p95_ms"] = 1
    elif mutation == "comparison_order": candidate["comparisons"].reverse()
    elif mutation == "ci_order": candidate["comparisons"][0]["bulk_paired_ci95_low_percent"] = 50
    elif mutation == "regression": candidate["comparisons"][0]["query_p95_regression_percent"] = 0
    elif mutation == "gate": candidate["comparisons"][0]["gates"]["accepted"] = False
    elif mutation == "smoke_acceptance":
        candidate["run"].update({"kind": "smoke", "blocks": 1, "sessions": 3, "sessions_per_arm": 1, "warmup_seconds": 1, "measurement_seconds": 1})
        for arm in candidate["arms"].values():
            arm["sessions"] = 1
            arm["interactive"].update({"samples": 2, "successes": 2})
        for comparison in candidate["comparisons"]:
            comparison["paired_blocks"] = 1
        candidate["decision"] = {"status": "smoke_pass", "winner": None, "private_matrix_eligible": False}
        candidate["claims"] = ["capability_only", "no_product_speedup", "no_private_claim", "no_release_claim"]
    elif mutation == "decision": candidate["decision"]["winner"] = None
    elif mutation == "privacy": candidate["privacy"]["contains_query_text"] = True
    elif mutation == "claims": candidate["claims"][0] = "product_speedup"
    else: fail(f"invalid fixture: unknown mutation {mutation}")
    return candidate


def main(paths: list[str] | None = None) -> int:
    schema = mapping(load(SCHEMA), "schema")
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema" or schema.get("additionalProperties") is not False:
        fail("schema: root must be closed draft 2020-12")
    valid = load(VALID)
    validate(valid)
    cases = load(INVALID)
    if not isinstance(cases, list) or not cases:
        fail("invalid fixtures: expected nonempty list")
    for case in cases:
        case = closed(case, {"name", "mutation"}, "invalid fixture")
        try:
            validate(mutate(valid, str(case["mutation"])))
        except ValueError:
            continue
        fail(f"invalid fixture accepted: {case['name']}")
    for path in paths or []:
        validate(load(pathlib.Path(path)))
    suffix = f" and {len(paths)} report(s)" if paths else ""
    print(f"resident embedding role-isolation contract check passed ({len(cases)} negative cases{suffix})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"check-resident-embedding-role-isolation.py failed: {error}", file=sys.stderr)
        raise SystemExit(1)
