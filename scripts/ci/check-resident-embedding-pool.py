#!/usr/bin/env python3
"""Validate the bounded public #341 fixed-B4 resident-pool report."""

from __future__ import annotations

import copy
import json
import math
import pathlib
import re
import sys
from collections.abc import Mapping

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "perf" / "resident-embedding-pool.schema.json"
FIXTURES = ROOT / "perf" / "fixtures" / "resident-embedding-pool"
VALID = FIXTURES / "valid-public-report.json"
INVALID = FIXTURES / "invalid-cases.json"
ARMS = ("i3_bulk1x4_b4", "i3_bulk2x2_b4", "i3_bulk2x3_b4")
CANDIDATES = ARMS[1:]
OUTCOMES = (
    "exact_expected", "valid_epoch_result_change", "deadline_partial",
    "semantic_partial", "overload", "http_error", "protocol_error",
    "transport_error", "cancelled",
)
DEGRADED_OUTCOMES = set(OUTCOMES[2:])
CORRECTNESS = {
    "vectors_elementwise_exact", "complete_batch_grouping_exact", "counts_exact",
    "order_exact", "cancellation_exact", "timeout_exact", "crash_restart_exact",
    "ready_exact", "publication_atomicity_exact", "cleanup_exact",
    "query_outcomes_exact",
}
PRIVACY = {
    "contains_private_paths", "contains_filenames", "contains_resume_text",
    "contains_query_text", "contains_candidate_results",
    "contains_document_or_version_ids", "contains_token_ids", "contains_vectors",
    "contains_pids", "contains_logs_or_traces", "contains_direct_raw_hashes",
    "contains_databases_or_indexes", "contains_model_or_runtime_bytes",
}
FIXED_WORKLOAD = {
    "model_id": "intfloat-multilingual-e5-small-qint8-r1",
    "upstream_revision": "614241f622f53c4eeff9890bdc4f31cfecc418b3",
    "tokenizer_sha256": "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    "runtime": "onnxruntime-1.27", "quantization": "dynamic_u8s8",
    "prepacking": False, "prefix": "passage",
    "pooling": "attention_mask_mean_then_l2", "input_policy": "whole_head_512",
    "max_tokens": 512, "dimension": 384, "bulk_batch": 4,
    "bulk_tokens_per_input": 512, "bulk_grouping": "fixed_complete_groups_of_four",
    "bulk_order": "seeded_frozen", "bulk_dispatch": "resident_ordinal_then_input_ordinal",
    "bulk_saturated": True, "interactive_batch": 1, "interactive_tokens": 32,
    "interactive_qps": 2, "query_oracle": "visible_epoch_aware",
    "valid_query_outcomes": ["exact_expected", "valid_epoch_result_change"],
    "degraded_query_outcomes": list(OUTCOMES[2:]),
    "resident_lifetime": "daemon_lifetime",
    "pool_reclaim": "signal_all_before_joining_any",
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


def integer(value: object, label: str, minimum: int = 0) -> int:
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


def close(actual: object, expected: float, label: str) -> None:
    observed = number(actual, label)
    if not math.isclose(observed, expected, rel_tol=0.0, abs_tol=0.003):
        fail(f"{label}: inconsistent aggregate")


def validate(report_value: object) -> None:
    root_keys = {
        "schema_version", "artifact_id", "issue", "source", "revision", "platform",
        "run", "fixed_workload", "arms", "comparisons", "decision", "privacy", "claims",
    }
    report = closed(report_value, root_keys, "report")
    try:
        encoded = json.dumps(report, allow_nan=False, ensure_ascii=True, separators=(",", ":"))
    except (TypeError, ValueError) as error:
        fail(f"report: not finite JSON: {error}")
    if len(encoded.encode()) > 64 * 1024:
        fail("report: exceeds 64 KiB")
    if any(signal in encoded for signal in ("/Users/", "file://", "\\Users\\", "Bearer ")):
        fail("report: contains private path or credential signal")
    identity = {
        "schema_version": "resume-ir.resident-embedding-pool.v1",
        "artifact_id": "resident-embedding-pool-issue-341",
        "issue": "#341", "source": "public_synthetic_daemon_sessions",
    }
    if any(report[key] != value for key, value in identity.items()):
        fail("report: identity mismatch")
    if not isinstance(report["revision"], str) or re.fullmatch(r"[0-9a-f]{40}", report["revision"]) is None:
        fail("revision: expected exact lowercase commit")

    expected_platform = {
        "os": "macos", "architecture": "arm64", "machine": "M4",
        "governor": "H2_Aggressive",
        "memory_measurement": "process_tree_private_or_anonymous_peak_mib",
    }
    if closed(report["platform"], set(expected_platform), "platform") != expected_platform:
        fail("platform: identity mismatch")

    run_keys = {
        "kind", "seed", "quiet_preflight_seconds", "blocks", "sessions",
        "sessions_per_arm", "independent_release_daemon_sessions", "williams_balanced",
        "warmup_seconds", "measurement_seconds", "all_sessions_completed",
        "thermal_guard_passed", "host_load_guard_passed", "process_cleanup_passed",
    }
    run = closed(report["run"], run_keys, "run")
    kind = run["kind"]
    if kind not in {"smoke", "formal_public_matrix"} or run["seed"] != 20260803:
        fail("run: identity mismatch")
    if run["independent_release_daemon_sessions"] is not True or run["williams_balanced"] is not True:
        fail("run: daemon-session design mismatch")
    shape = (
        number(run["quiet_preflight_seconds"], "run.quiet_preflight_seconds", positive=True),
        integer(run["blocks"], "run.blocks", 1),
        integer(run["sessions"], "run.sessions", 1),
        integer(run["sessions_per_arm"], "run.sessions_per_arm", 1),
        number(run["warmup_seconds"], "run.warmup_seconds", positive=True),
        number(run["measurement_seconds"], "run.measurement_seconds", positive=True),
    )
    expected_shape = (1.0, 1, 3, 1, 1.0, 1.0) if kind == "smoke" else (120.0, 10, 30, 10, 30.0, 60.0)
    if shape != expected_shape:
        fail("run: workload shape mismatch")
    run_guards = [
        boolean(run[key], f"run.{key}")
        for key in (
            "all_sessions_completed", "thermal_guard_passed",
            "host_load_guard_passed", "process_cleanup_passed",
        )
    ]
    if closed(report["fixed_workload"], set(FIXED_WORKLOAD), "fixed_workload") != FIXED_WORKLOAD:
        fail("fixed_workload: mismatch")

    arms = closed(report["arms"], set(ARMS), "arms")
    arm_identity = {
        "i3_bulk1x4_b4": (4, 1, 2),
        "i3_bulk2x2_b4": (2, 2, 3),
        "i3_bulk2x3_b4": (3, 2, 3),
    }
    arm_outcomes: dict[str, bool] = {}
    arm_correctness: dict[str, bool] = {}
    completed_sessions = 0
    sessions_per_arm = int(run["sessions_per_arm"])
    measurement_seconds = float(run["measurement_seconds"])
    for name in ARMS:
        arm_keys = {
            "topology", "interactive_threads", "bulk_threads", "bulk_resident_count",
            "resident_count", "sessions", "bulk", "interactive", "resources", "correctness",
        }
        arm = closed(arms[name], arm_keys, f"arms.{name}")
        observed_identity = (
            arm["bulk_threads"], arm["bulk_resident_count"], arm["resident_count"],
        )
        if arm["topology"] != "interactive_plus_bulk_pool" or arm["interactive_threads"] != 3 or observed_identity != arm_identity[name]:
            fail(f"arms.{name}: topology mismatch")
        sessions = integer(arm["sessions"], f"arms.{name}.sessions", 1)
        if sessions > sessions_per_arm or (run["all_sessions_completed"] and sessions != sessions_per_arm):
            fail(f"arms.{name}: session count mismatch")
        completed_sessions += sessions

        bulk = closed(
            arm["bulk"],
            {"completed_batches", "completed_inputs", "mean_throughput_inputs_per_second"},
            f"arms.{name}.bulk",
        )
        batches = integer(bulk["completed_batches"], f"arms.{name}.bulk.completed_batches", 1)
        if integer(bulk["completed_inputs"], f"arms.{name}.bulk.completed_inputs", 1) != batches * 4:
            fail(f"arms.{name}.bulk: complete Batch 4 counts do not reconcile")
        number(bulk["mean_throughput_inputs_per_second"], f"arms.{name}.bulk.throughput", positive=True)

        interactive_keys = {"samples", "outcomes", "p50_ms", "p95_ms", "p99_ms", "max_resident_queue_wait_ms"}
        interactive = closed(arm["interactive"], interactive_keys, f"arms.{name}.interactive")
        samples = integer(interactive["samples"], f"arms.{name}.interactive.samples", 1)
        if samples != round(measurement_seconds * 2) * sessions:
            fail(f"arms.{name}.interactive: fixed 2 QPS sample count mismatch")
        outcomes = closed(interactive["outcomes"], set(OUTCOMES), f"arms.{name}.interactive.outcomes")
        outcome_counts = {key: integer(outcomes[key], f"arms.{name}.interactive.outcomes.{key}") for key in OUTCOMES}
        if sum(outcome_counts.values()) != samples:
            fail(f"arms.{name}.interactive: outcome counts do not reconcile")
        arm_outcomes[name] = all(outcome_counts[key] == 0 for key in DEGRADED_OUTCOMES)
        latencies = [
            number(interactive[key], f"arms.{name}.interactive.{key}", positive=True)
            for key in ("p50_ms", "p95_ms", "p99_ms")
        ]
        if latencies != sorted(latencies):
            fail(f"arms.{name}.interactive: latency percentiles are not monotonic")
        number(interactive["max_resident_queue_wait_ms"], f"arms.{name}.interactive.max_resident_queue_wait_ms")
        resources = closed(
            arm["resources"], {"process_tree_private_or_anonymous_peak_mib"},
            f"arms.{name}.resources",
        )
        number(resources["process_tree_private_or_anonymous_peak_mib"], f"arms.{name}.resources.peak", positive=True)
        correctness = closed(arm["correctness"], CORRECTNESS, f"arms.{name}.correctness")
        arm_correctness[name] = all(boolean(value, f"arms.{name}.correctness") for value in correctness.values())

    sessions_complete = completed_sessions == int(run["sessions"])
    run_valid = all(run_guards) and sessions_complete
    comparisons = report["comparisons"]
    if not isinstance(comparisons, list) or len(comparisons) != 2:
        fail("comparisons: expected exactly two candidates")
    comparison_keys = {
        "control", "candidate", "paired_blocks", "bulk_improvement_percent",
        "bulk_paired_ci95_low_percent", "bulk_paired_ci95_high_percent",
        "query_p95_regression_percent", "query_p99_regression_percent",
        "max_interactive_resident_queue_wait_ms",
        "max_process_tree_private_or_anonymous_peak_mib", "outcome_guard_pass",
        "correctness_pass", "gates",
    }
    gate_keys = {
        "bulk_at_least_15_percent", "bulk_ci_positive", "query_p95_within_5_percent",
        "query_p99_within_10_percent", "direct_queue_wait_within_200_ms",
        "resource_within_1536_mib", "outcomes_exact", "correctness_exact", "accepted",
    }
    accepted: list[str] = []
    blocks = int(run["blocks"])
    for index, candidate in enumerate(CANDIDATES):
        comparison = closed(comparisons[index], comparison_keys, f"comparisons.{index}")
        if comparison["control"] != ARMS[0] or comparison["candidate"] != candidate:
            fail("comparisons: order or identity mismatch")
        paired_blocks = integer(comparison["paired_blocks"], f"comparisons.{candidate}.paired_blocks", 1)
        if paired_blocks > blocks or (run["all_sessions_completed"] and paired_blocks != blocks):
            fail(f"comparisons.{candidate}: paired block count mismatch")
        improvement = number(comparison["bulk_improvement_percent"], f"comparisons.{candidate}.bulk_improvement")
        ci_low = number(comparison["bulk_paired_ci95_low_percent"], f"comparisons.{candidate}.ci_low")
        ci_high = number(comparison["bulk_paired_ci95_high_percent"], f"comparisons.{candidate}.ci_high")
        if not ci_low <= improvement <= ci_high:
            fail(f"comparisons.{candidate}: confidence interval is inconsistent")
        control_interactive = arms[ARMS[0]]["interactive"]
        candidate_interactive = arms[candidate]["interactive"]
        p95_regression = number(comparison["query_p95_regression_percent"], f"comparisons.{candidate}.p95_regression")
        p99_regression = number(comparison["query_p99_regression_percent"], f"comparisons.{candidate}.p99_regression")
        close(p95_regression, (float(candidate_interactive["p95_ms"]) / float(control_interactive["p95_ms"]) - 1.0) * 100.0, f"comparisons.{candidate}.p95_regression")
        close(p99_regression, (float(candidate_interactive["p99_ms"]) / float(control_interactive["p99_ms"]) - 1.0) * 100.0, f"comparisons.{candidate}.p99_regression")
        direct_queue = max(
            float(control_interactive["max_resident_queue_wait_ms"]),
            float(candidate_interactive["max_resident_queue_wait_ms"]),
        )
        memory_peak = max(
            float(arms[ARMS[0]]["resources"]["process_tree_private_or_anonymous_peak_mib"]),
            float(arms[candidate]["resources"]["process_tree_private_or_anonymous_peak_mib"]),
        )
        close(comparison["max_interactive_resident_queue_wait_ms"], direct_queue, f"comparisons.{candidate}.direct_queue")
        close(comparison["max_process_tree_private_or_anonymous_peak_mib"], memory_peak, f"comparisons.{candidate}.memory_peak")
        outcome_guard = arm_outcomes[ARMS[0]] and arm_outcomes[candidate]
        correctness_pass = arm_correctness[ARMS[0]] and arm_correctness[candidate]
        if comparison["outcome_guard_pass"] is not outcome_guard or comparison["correctness_pass"] is not correctness_pass:
            fail(f"comparisons.{candidate}: guard summary mismatch")
        expected_gates = {
            "bulk_at_least_15_percent": improvement >= 15.0,
            "bulk_ci_positive": ci_low > 0.0,
            "query_p95_within_5_percent": p95_regression <= 5.0,
            "query_p99_within_10_percent": p99_regression <= 10.0,
            "direct_queue_wait_within_200_ms": direct_queue <= 200.0,
            "resource_within_1536_mib": memory_peak <= 1536.0,
            "outcomes_exact": outcome_guard,
            "correctness_exact": correctness_pass,
        }
        expected_gates["accepted"] = kind == "formal_public_matrix" and run_valid and paired_blocks == blocks and all(expected_gates.values())
        if closed(comparison["gates"], gate_keys, f"comparisons.{candidate}.gates") != expected_gates:
            fail(f"comparisons.{candidate}: gate decision mismatch")
        if expected_gates["accepted"]:
            accepted.append(candidate)

    decision = closed(report["decision"], {"status", "winner", "private_matrix_eligible"}, "decision")
    if kind == "smoke":
        passed = run_valid and all(arm_outcomes.values()) and all(arm_correctness.values())
        expected_decision = {"status": "smoke_pass" if passed else "smoke_failed", "winner": None, "private_matrix_eligible": False}
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
    elif mutation == "identity": candidate["issue"] = "#319"
    elif mutation == "path_signal": candidate["artifact_id"] = "/Users/private/report"
    elif mutation == "oversized": candidate["artifact_id"] = "x" * (64 * 1024)
    elif mutation == "nan": candidate["comparisons"][0]["bulk_improvement_percent"] = float("nan")
    elif mutation == "run_shape": candidate["run"]["quiet_preflight_seconds"] = 30
    elif mutation == "arm_identity": candidate["arms"][ARMS[0]]["bulk_resident_count"] = 2
    elif mutation == "batch_count": candidate["arms"][ARMS[0]]["bulk"]["completed_inputs"] += 1
    elif mutation == "query_count": candidate["arms"][ARMS[0]]["interactive"]["samples"] -= 1
    elif mutation == "degraded_outcome":
        outcomes = candidate["arms"][ARMS[0]]["interactive"]["outcomes"]
        outcomes["exact_expected"] -= 1
        outcomes["overload"] += 1
    elif mutation == "percentile_order": candidate["arms"][ARMS[0]]["interactive"]["p95_ms"] = 1
    elif mutation == "comparison_order": candidate["comparisons"].reverse()
    elif mutation == "ci_order": candidate["comparisons"][0]["bulk_paired_ci95_low_percent"] = 50
    elif mutation == "regression": candidate["comparisons"][0]["query_p95_regression_percent"] = 0
    elif mutation == "direct_queue": candidate["comparisons"][0]["max_interactive_resident_queue_wait_ms"] += 1
    elif mutation == "resource": candidate["comparisons"][0]["max_process_tree_private_or_anonymous_peak_mib"] += 1
    elif mutation == "outcome_guard": candidate["comparisons"][0]["outcome_guard_pass"] = False
    elif mutation == "correctness": candidate["arms"][CANDIDATES[0]]["correctness"]["order_exact"] = False
    elif mutation == "gate": candidate["comparisons"][0]["gates"]["accepted"] = False
    elif mutation == "smoke_acceptance":
        candidate["run"].update({"kind": "smoke", "quiet_preflight_seconds": 1, "blocks": 1, "sessions": 3, "sessions_per_arm": 1, "warmup_seconds": 1, "measurement_seconds": 1})
        for arm in candidate["arms"].values():
            arm["sessions"] = 1
            arm["interactive"]["samples"] = 2
            arm["interactive"]["outcomes"].update({key: 0 for key in OUTCOMES})
            arm["interactive"]["outcomes"]["exact_expected"] = 2
        for comparison in candidate["comparisons"]:
            comparison["paired_blocks"] = 1
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
    for case_value in cases:
        case = closed(case_value, {"name", "mutation"}, "invalid fixture")
        try:
            validate(mutate(valid, str(case["mutation"])))
        except ValueError:
            continue
        fail(f"invalid fixture accepted: {case['name']}")
    for path in paths or []:
        validate(load(pathlib.Path(path)))
    suffix = f" and {len(paths)} report(s)" if paths else ""
    print(f"resident embedding pool contract check passed ({len(cases)} negative cases{suffix})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"check-resident-embedding-pool.py failed: {error}", file=sys.stderr)
        raise SystemExit(1)
