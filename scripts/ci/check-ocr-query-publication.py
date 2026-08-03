#!/usr/bin/env python3
"""Validate the bounded public #342 OCR query-publication diagnosis report."""

from __future__ import annotations

import copy
import json
import math
import pathlib
import re
import sys
from collections import Counter
from collections.abc import Mapping

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "perf" / "ocr-query-publication.schema.json"
FIXTURES = ROOT / "perf" / "fixtures" / "ocr-query-publication"
VALID = FIXTURES / "valid-public-report.json"
INVALID = FIXTURES / "invalid-cases.json"
MODES = ("fulltext", "semantic", "hybrid")
OUTCOMES = (
    "exact_expected", "valid_epoch_result_change", "deadline_partial",
    "semantic_partial", "overload", "http_error", "protocol_error",
    "transport_error", "cancelled",
)
VALID_COMPLETIONS = {"exact_expected", "valid_epoch_result_change"}
WIRE_COMPLETIONS = VALID_COMPLETIONS | {
    "deadline_partial", "semantic_partial", "cancelled",
}
STAGES = ("parse", "prefilter", "bm25", "ann", "fusion", "bulk_hydrate", "snippet")
PRIVACY = {
    "contains_private_paths", "contains_filenames", "contains_resume_text",
    "contains_query_text", "contains_candidate_results",
    "contains_document_or_version_ids", "contains_token_ids", "contains_vectors",
    "contains_pids", "contains_logs_or_traces", "contains_direct_raw_hashes",
    "contains_databases_or_indexes", "contains_model_or_runtime_bytes",
}
SCHEDULE = (
    ("hybrid", "fulltext", "semantic"),
    ("fulltext", "semantic", "hybrid"),
    ("semantic", "hybrid", "fulltext"),
)
FIXED_WORKLOAD = {
    "model_id": "intfloat-multilingual-e5-small-qint8-r1",
    "dimension": 384,
    "resident_topology": "shared_i3",
    "resident_threads": 3,
    "interactive_batch": 1,
    "publication_batch_bound": 4,
    "ocr_jobs_per_tick": 1,
    "query_modes": list(MODES),
    "top_k": 1,
    "deadline_ms": 10_000,
    "oracle": "stable_dominant_synthetic_anchor",
    "publication_source": "rasterized_public_synthetic_pdf",
    "mode_order_schedule": [list(row) for row in SCHEDULE],
    "client_excess_ms": 20,
    "unclassified_excess_ms": 15,
    "minimum_first_to_warm_ratio": 1.5,
    "required_signal_epochs": 2,
}
OBSERVABILITY = {
    "first_definition": "first_mode_local_request_after_visible_epoch_advance",
    "warm_definition": "immediate_same_mode_repeat_in_same_epoch",
    "residual_label": "unclassified_query_service_embedding_or_queue_wall",
    "direct_queue_wait_telemetry_available": False,
    "queue_wait_claimed": False,
    "mode_order_position_reported": True,
}
CLAIMS = [
    "diagnostic_only", "no_product_speedup", "no_query_hot_path_acceptance",
    "no_private_or_release_claim",
]


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


def close(actual: object, expected: float, label: str, tolerance: float = 0.003) -> None:
    observed = number(actual, label)
    if not math.isclose(observed, expected, rel_tol=0.0, abs_tol=tolerance):
        fail(f"{label}: inconsistent aggregate")


def validate_resources(value: object, label: str) -> None:
    keys = {
        "process_tree_rss_mib", "normalized_one_minute_load",
        "status_embedding_queue_depth_before", "status_embedding_queue_depth_after",
        "ocr_queue_depth_after",
    }
    resource = closed(value, keys, label)
    number(resource["process_tree_rss_mib"], f"{label}.process_tree_rss_mib")
    number(resource["normalized_one_minute_load"], f"{label}.normalized_load")
    for key in keys - {"process_tree_rss_mib", "normalized_one_minute_load"}:
        integer(resource[key], f"{label}.{key}")


def validate_sample(value: object, label: str) -> dict[str, object]:
    sample = closed(
        value,
        {"outcome", "client_wall_ms", "server_latency_ms", "stage_latency_ms", "unclassified_wall_ms"},
        label,
    )
    outcome = sample["outcome"]
    if outcome not in OUTCOMES:
        fail(f"{label}.outcome: invalid class")
    client = number(sample["client_wall_ms"], f"{label}.client_wall_ms", positive=True)
    if outcome not in WIRE_COMPLETIONS:
        if any(sample[key] is not None for key in ("server_latency_ms", "stage_latency_ms", "unclassified_wall_ms")):
            fail(f"{label}: untrusted error response retained server timing")
        return {"outcome": outcome, "client": client, "server": None, "unclassified": None}
    server = number(sample["server_latency_ms"], f"{label}.server_latency_ms")
    stages = closed(sample["stage_latency_ms"], set(STAGES), f"{label}.stage_latency_ms")
    stage_total = sum(number(stages[key], f"{label}.stage_latency_ms.{key}") for key in STAGES)
    unclassified = number(sample["unclassified_wall_ms"], f"{label}.unclassified_wall_ms")
    close(unclassified, max(server - stage_total, 0.0), f"{label}.unclassified_wall_ms")
    return {"outcome": outcome, "client": client, "server": server, "unclassified": unclassified}


def validate_group(value: object, label: str, *, ordinal: int | None) -> dict[str, dict[str, dict[str, object]]]:
    keys = {"mode_order", "samples", "resources"} | ({"ordinal"} if ordinal is not None else set())
    group = closed(value, keys, label)
    if ordinal is not None and integer(group["ordinal"], f"{label}.ordinal", 1) != ordinal:
        fail(f"{label}.ordinal: sequence mismatch")
    order = group["mode_order"]
    if not isinstance(order, list) or tuple(order) != (SCHEDULE[ordinal - 1] if ordinal else MODES):
        fail(f"{label}.mode_order: frozen order mismatch")
    samples = closed(group["samples"], set(MODES), f"{label}.samples")
    result: dict[str, dict[str, dict[str, object]]] = {}
    for mode in MODES:
        pair = closed(samples[mode], {"first", "warm"}, f"{label}.samples.{mode}")
        result[mode] = {
            phase: validate_sample(pair[phase], f"{label}.samples.{mode}.{phase}")
            for phase in ("first", "warm")
        }
    validate_resources(group["resources"], f"{label}.resources")
    return result


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    rank = max(math.ceil(len(ordered) * fraction) - 1, 0)
    return ordered[rank]


def summary(values: list[float]) -> dict[str, float]:
    if not values:
        fail("aggregate: empty latency list")
    return {
        "p50": round(percentile(values, 0.50), 3),
        "p95": round(percentile(values, 0.95), 3),
        "p99": round(percentile(values, 0.99), 3),
        "max": round(max(values), 3),
    }


def validate_summary(value: object, expected: Mapping[str, float], label: str) -> None:
    observed = closed(value, {"p50", "p95", "p99", "max"}, label)
    for key, expected_value in expected.items():
        close(observed[key], expected_value, f"{label}.{key}")
    numbers = [number(observed[key], f"{label}.{key}") for key in ("p50", "p95", "p99", "max")]
    if numbers != sorted(numbers):
        fail(f"{label}: percentiles are not monotonic")


def expected_aggregate(
    mode: str,
    control: dict[str, dict[str, dict[str, object]]],
    epochs: list[dict[str, dict[str, dict[str, object]]]],
) -> tuple[dict[str, object], int]:
    control_pair = control[mode]
    control_delta = float(control_pair["first"]["client"]) - float(control_pair["warm"]["client"])
    control_unclassified = float(control_pair["first"]["unclassified"]) - float(control_pair["warm"]["unclassified"])
    first_values, warm_values, deltas, excesses, unclassified_excesses = [], [], [], [], []
    complete_pairs, signal_epochs = 0, 0
    for epoch in epochs:
        first, warm = epoch[mode]["first"], epoch[mode]["warm"]
        first_client, warm_client = float(first["client"]), float(warm["client"])
        delta = first_client - warm_client
        excess = delta - control_delta
        first_values.append(first_client)
        warm_values.append(warm_client)
        deltas.append(delta)
        excesses.append(excess)
        complete = first["outcome"] in VALID_COMPLETIONS and warm["outcome"] in VALID_COMPLETIONS
        if complete:
            complete_pairs += 1
            unclassified_excess = (
                float(first["unclassified"]) - float(warm["unclassified"]) - control_unclassified
            )
            signalled = (
                excess >= FIXED_WORKLOAD["client_excess_ms"]
                and first_client / warm_client >= FIXED_WORKLOAD["minimum_first_to_warm_ratio"]
                and unclassified_excess >= FIXED_WORKLOAD["unclassified_excess_ms"]
            )
        else:
            unclassified_excess = 0.0
            signalled = True
        unclassified_excesses.append(unclassified_excess)
        signal_epochs += int(signalled)
    expected = {
        "epoch_pairs": len(epochs),
        "complete_epoch_pairs": complete_pairs,
        "first_client_ms": summary(first_values),
        "warm_client_ms": summary(warm_values),
        "first_minus_warm_ms": summary(deltas),
        "excess_over_control_ms": summary(excesses),
        "unclassified_excess_over_control_ms": summary(unclassified_excesses),
        "signal_epochs": signal_epochs,
    }
    return expected, signal_epochs


def validate_aggregate(value: object, expected: Mapping[str, object], label: str) -> None:
    keys = {
        "epoch_pairs", "complete_epoch_pairs", "first_client_ms", "warm_client_ms",
        "first_minus_warm_ms", "excess_over_control_ms",
        "unclassified_excess_over_control_ms", "signal_epochs",
    }
    aggregate = closed(value, keys, label)
    for key in ("epoch_pairs", "complete_epoch_pairs", "signal_epochs"):
        if integer(aggregate[key], f"{label}.{key}") != expected[key]:
            fail(f"{label}.{key}: inconsistent count")
    for key in keys - {"epoch_pairs", "complete_epoch_pairs", "signal_epochs"}:
        validate_summary(aggregate[key], expected[key], f"{label}.{key}")  # type: ignore[arg-type]


def validate(report_value: object) -> None:
    root_keys = {
        "schema_version", "artifact_id", "issue", "source", "revision", "platform",
        "run", "fixed_workload", "observability", "stable_control", "epochs",
        "outcomes", "mode_aggregates", "diagnosis", "privacy", "claims",
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
        "schema_version": "resume-ir.ocr-query-publication.v1",
        "artifact_id": "ocr-query-publication-issue-342",
        "issue": "#342",
        "source": "public_synthetic_repeated_ocr_publication",
    }
    if any(report[key] != value for key, value in identity.items()):
        fail("report: identity mismatch")
    if not isinstance(report["revision"], str) or re.fullmatch(r"[0-9a-f]{40}", report["revision"]) is None:
        fail("revision: expected exact lowercase commit")
    expected_platform = {
        "os": "macos", "architecture": "arm64", "machine": "M4",
        "governor": "H2_Aggressive", "memory_measurement": "process_tree_rss_mib",
    }
    if closed(report["platform"], set(expected_platform), "platform") != expected_platform:
        fail("platform: identity mismatch")
    run_keys = {
        "kind", "seed", "publication_count", "query_attempts", "all_publications_observed",
        "epoch_steps_exact", "oracle_corruption_probe_passed", "transport_probe_passed",
        "protocol_probe_passed", "process_cleanup_passed", "host_load_guard_passed",
    }
    run = closed(report["run"], run_keys, "run")
    if run["kind"] not in {"smoke", "formal_public_witness"} or run["seed"] != 20260803:
        fail("run: identity mismatch")
    publications = integer(run["publication_count"], "run.publication_count", 1)
    if publications != (1 if run["kind"] == "smoke" else 3):
        fail("run: publication shape mismatch")
    query_attempts = integer(run["query_attempts"], "run.query_attempts", 1)
    guards = [
        boolean(run[key], f"run.{key}")
        for key in (
            "all_publications_observed", "epoch_steps_exact",
            "oracle_corruption_probe_passed", "transport_probe_passed",
            "protocol_probe_passed", "process_cleanup_passed",
        )
    ]
    boolean(run["host_load_guard_passed"], "run.host_load_guard_passed")
    if closed(report["fixed_workload"], set(FIXED_WORKLOAD), "fixed_workload") != FIXED_WORKLOAD:
        fail("fixed_workload: mismatch")
    if closed(report["observability"], set(OBSERVABILITY), "observability") != OBSERVABILITY:
        fail("observability: direct queue wait must not be invented")
    control = validate_group(report["stable_control"], "stable_control", ordinal=None)
    if any(sample["outcome"] != "exact_expected" for pairs in control.values() for sample in pairs.values()):
        fail("stable_control: expected exact hot-generation responses")
    epochs_value = report["epochs"]
    if not isinstance(epochs_value, list) or len(epochs_value) != publications:
        fail("epochs: publication count mismatch")
    epochs = [
        validate_group(value, f"epochs.{index}", ordinal=index + 1)
        for index, value in enumerate(epochs_value)
    ]
    samples = [sample for group in [control, *epochs] for pairs in group.values() for sample in pairs.values()]
    expected_attempts = (publications + 1) * len(MODES) * 2
    if query_attempts != expected_attempts or len(samples) != expected_attempts:
        fail("run.query_attempts: sample count mismatch")
    observed_counts = Counter(str(sample["outcome"]) for sample in samples)
    outcome = closed(
        report["outcomes"],
        {"attempted", "counts", "completed_valid", "degraded_or_failed", "count_conserved"},
        "outcomes",
    )
    if integer(outcome["attempted"], "outcomes.attempted", 1) != expected_attempts:
        fail("outcomes.attempted: mismatch")
    counts = closed(outcome["counts"], set(OUTCOMES), "outcomes.counts")
    for key in OUTCOMES:
        if integer(counts[key], f"outcomes.counts.{key}") != observed_counts[key]:
            fail(f"outcomes.counts.{key}: mismatch")
    completed_valid = observed_counts["exact_expected"] + observed_counts["valid_epoch_result_change"]
    if integer(outcome["completed_valid"], "outcomes.completed_valid") != completed_valid:
        fail("outcomes.completed_valid: valid result changes are not failures")
    if integer(outcome["degraded_or_failed"], "outcomes.degraded_or_failed") != expected_attempts - completed_valid:
        fail("outcomes.degraded_or_failed: mismatch")
    if outcome["count_conserved"] is not True or sum(observed_counts.values()) != expected_attempts:
        fail("outcomes: count conservation failed")
    aggregates = closed(report["mode_aggregates"], set(MODES), "mode_aggregates")
    signal_counts: dict[str, int] = {}
    for mode in MODES:
        expected, signal_counts[mode] = expected_aggregate(mode, control, epochs)
        validate_aggregate(aggregates[mode], expected, f"mode_aggregates.{mode}")
    fulltext_signal = signal_counts["fulltext"] >= FIXED_WORKLOAD["required_signal_epochs"]
    semantic_signal = any(
        signal_counts[mode] >= FIXED_WORKLOAD["required_signal_epochs"]
        for mode in ("semantic", "hybrid")
    )
    core_valid = all(guards)
    if run["kind"] == "smoke":
        all_exact = observed_counts["exact_expected"] == expected_attempts
        expected_diagnosis = {
            "status": "smoke_pass" if core_valid and all_exact else "smoke_failed",
            "selected_next_action": None,
            "fulltext_signal": False,
            "semantic_or_hybrid_signal": False,
        }
    elif not core_valid:
        expected_diagnosis = {
            "status": "inconclusive", "selected_next_action": None,
            "fulltext_signal": False, "semantic_or_hybrid_signal": False,
        }
    else:
        action = (
            "combined_bounded_fix_issue" if fulltext_signal and semantic_signal
            else "generation_publication_fix_issue" if fulltext_signal
            else "resident_isolation_rerun" if semantic_signal
            else "no_reproduced_product_defect"
        )
        expected_diagnosis = {
            "status": "diagnosed" if fulltext_signal or semantic_signal else "not_reproduced",
            "selected_next_action": action,
            "fulltext_signal": fulltext_signal,
            "semantic_or_hybrid_signal": semantic_signal,
        }
    diagnosis_keys = {
        "status", "selected_next_action", "fulltext_signal", "semantic_or_hybrid_signal",
        "signal_epoch_counts", "outcome_integrity_passed", "no_speedup_claim",
        "no_query_hot_path_acceptance_claim",
    }
    diagnosis = closed(report["diagnosis"], diagnosis_keys, "diagnosis")
    for key, expected in expected_diagnosis.items():
        if diagnosis[key] != expected:
            fail(f"diagnosis.{key}: inconsistent decision")
    reported_signals = closed(diagnosis["signal_epoch_counts"], set(MODES), "diagnosis.signal_epoch_counts")
    for mode in MODES:
        if integer(reported_signals[mode], f"diagnosis.signal_epoch_counts.{mode}") != signal_counts[mode]:
            fail(f"diagnosis.signal_epoch_counts.{mode}: mismatch")
    expected_integrity = observed_counts["protocol_error"] == 0
    if diagnosis["outcome_integrity_passed"] is not expected_integrity:
        fail("diagnosis.outcome_integrity_passed: mismatch")
    if diagnosis["no_speedup_claim"] is not True or diagnosis["no_query_hot_path_acceptance_claim"] is not True:
        fail("diagnosis: unsupported acceptance claim")
    if any(value is not False for value in closed(report["privacy"], PRIVACY, "privacy").values()):
        fail("privacy: all leak flags must be false")
    if report["claims"] != CLAIMS:
        fail("claims: unsupported claim set")


def mutate(report: object, mutation: str) -> object:
    candidate = copy.deepcopy(report)
    if mutation == "unknown_field": candidate["private"] = "payload"
    elif mutation == "identity": candidate["issue"] = "#341"
    elif mutation == "path_signal": candidate["artifact_id"] = "/Users/private/report"
    elif mutation == "oversized": candidate["artifact_id"] = "x" * (64 * 1024)
    elif mutation == "nan": candidate["epochs"][0]["samples"]["hybrid"]["first"]["client_wall_ms"] = float("nan")
    elif mutation == "run_shape": candidate["run"]["publication_count"] = 2
    elif mutation == "query_attempts": candidate["run"]["query_attempts"] -= 1
    elif mutation == "control_outcome": candidate["stable_control"]["samples"]["fulltext"]["first"]["outcome"] = "valid_epoch_result_change"
    elif mutation == "epoch_order": candidate["epochs"][0]["ordinal"] = 2
    elif mutation == "duplicate_mode": candidate["epochs"][0]["mode_order"][1] = "hybrid"
    elif mutation == "residual": candidate["epochs"][0]["samples"]["hybrid"]["first"]["unclassified_wall_ms"] += 1
    elif mutation == "error_timing": candidate["epochs"][0]["samples"]["hybrid"]["first"]["outcome"] = "transport_error"
    elif mutation == "outcome_count": candidate["outcomes"]["counts"]["exact_expected"] -= 1
    elif mutation == "valid_result_failure": candidate["outcomes"]["degraded_or_failed"] += 1
    elif mutation == "aggregate": candidate["mode_aggregates"]["semantic"]["signal_epochs"] -= 1
    elif mutation == "decision": candidate["diagnosis"]["selected_next_action"] = "no_reproduced_product_defect"
    elif mutation == "queue_wait_claim": candidate["observability"]["queue_wait_claimed"] = True
    elif mutation == "privacy": candidate["privacy"]["contains_query_text"] = True
    elif mutation == "claims": candidate["claims"][0] = "query_hot_path_accepted"
    else: fail(f"invalid fixture: unknown mutation {mutation}")
    return candidate


def main(paths: list[str] | None = None) -> int:
    schema = mapping(load(SCHEMA), "schema")
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema" or schema.get("additionalProperties") is not False:
        fail("schema: root must be closed draft 2020-12")
    valid = load(VALID)
    validate(valid)
    valid_change = copy.deepcopy(valid)
    valid_change["epochs"][0]["samples"]["hybrid"]["warm"]["outcome"] = "valid_epoch_result_change"
    valid_change["outcomes"]["counts"]["exact_expected"] -= 1
    valid_change["outcomes"]["counts"]["valid_epoch_result_change"] += 1
    validate(valid_change)
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
    print(f"OCR query-publication contract check passed ({len(cases)} negative cases{suffix})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"check-ocr-query-publication.py failed: {error}", file=sys.stderr)
        raise SystemExit(1)
