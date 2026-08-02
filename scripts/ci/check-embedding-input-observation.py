#!/usr/bin/env python3
"""Validate the bounded #312 embedding-input observation report and fixtures."""

from __future__ import annotations

import copy
import json
import math
import pathlib
import sys
from collections.abc import Mapping

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "perf" / "embedding-input-observation.schema.json"
FIXTURES = ROOT / "perf" / "fixtures" / "embedding-input-observation"
VALID = FIXTURES / "valid-public-report.json"
INVALID = FIXTURES / "invalid-cases.json"
FAMILIES = {"profile", "experience", "skill", "project", "education", "certificate", "contact", "other", "unassigned"}
BUDGETS = {"512", "384", "256"}
COVERAGE = {"complete_loss", "partial_below_half", "partial_at_least_half", "complete_retained"}


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


def count(value: object, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        fail(f"{label}: expected nonnegative integer")
    return value


def ratio(value: object, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{label}: expected finite ratio")
    result = float(value)
    if not math.isfinite(result) or not 0.0 <= result <= 1.0:
        fail(f"{label}: expected finite ratio")
    return result


def exact_ratio(numerator: int, denominator: int) -> float:
    return numerator / denominator if denominator else 0.0


def require_close(actual: float, expected: float, label: str) -> None:
    if not math.isclose(actual, expected, rel_tol=0.0, abs_tol=1e-12):
        fail(f"{label}: inconsistent ratio")


def validate(report: object) -> None:
    root_keys = {"schema_version", "artifact_id", "scope", "production_identity", "documents", "pre_truncation_active_tokens", "aggregate_active_token_work", "section_coverage", "priority_coverage_512", "triggers", "decision", "privacy", "claims"}
    report = closed(report, root_keys, "report")
    try:
        encoded = json.dumps(report, allow_nan=False, ensure_ascii=True, separators=(",", ":"))
    except (TypeError, ValueError) as error:
        fail(f"report: not closed finite JSON: {error}")
    if len(encoded.encode("utf-8")) > 64 * 1024:
        fail("report: exceeds 64 KiB")
    if any(signal in encoded for signal in ("/Users/", "file://", "\\Users\\")):
        fail("report: contains path signal")
    constants = {
        "schema_version": "resume-ir.embedding-input-observation.v1",
        "artifact_id": "embedding-input-observation-issue-312",
        "scope": "private local clean-text token observation; bounded redacted aggregate only",
    }
    if any(report[key] != value for key, value in constants.items()):
        fail("report: identity mismatch")
    identity = closed(report["production_identity"], {"runtime_pack_id", "model_id", "upstream_revision", "tokenizer_sha256", "prefix", "truncation"}, "production_identity")
    expected_identity = {
        "runtime_pack_id": "intfloat-multilingual-e5-small-qint8-r1",
        "model_id": "intfloat-multilingual-e5-small-qint8-r1",
        "upstream_revision": "614241f622f53c4eeff9890bdc4f31cfecc418b3",
        "tokenizer_sha256": "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
        "prefix": "passage",
        "truncation": "right",
    }
    if identity != expected_identity:
        fail("production_identity: mismatch")
    documents = closed(report["documents"], {"selected", "observed", "excluded_oversize", "failed"}, "documents")
    selected, observed, excluded, failed_count = (count(documents[key], f"documents.{key}") for key in ("selected", "observed", "excluded_oversize", "failed"))
    if selected != observed + excluded + failed_count:
        fail("documents: counts do not reconcile")
    tokens = closed(report["pre_truncation_active_tokens"], {"p50", "p75", "p90", "p95", "p99", "buckets", "exceed_256", "exceed_384", "exceed_512", "exceed_512_ratio"}, "tokens")
    quantiles = [count(tokens[key], f"tokens.{key}") for key in ("p50", "p75", "p90", "p95", "p99")]
    if quantiles != sorted(quantiles):
        fail("tokens: quantiles are not monotonic")
    bucket_keys = {"le_256", "257_384", "385_512", "513_768", "769_1024", "gt_1024"}
    buckets = closed(tokens["buckets"], bucket_keys, "tokens.buckets")
    bucket_counts = {key: count(value, f"tokens.buckets.{key}") for key, value in buckets.items()}
    if sum(bucket_counts.values()) != observed:
        fail("tokens.buckets: counts do not reconcile")
    expected_exceeds = {
        "exceed_256": observed - bucket_counts["le_256"],
        "exceed_384": bucket_counts["385_512"] + bucket_counts["513_768"] + bucket_counts["769_1024"] + bucket_counts["gt_1024"],
        "exceed_512": bucket_counts["513_768"] + bucket_counts["769_1024"] + bucket_counts["gt_1024"],
    }
    for key, expected in expected_exceeds.items():
        if count(tokens[key], f"tokens.{key}") != expected:
            fail(f"tokens.{key}: inconsistent count")
    saturation = ratio(tokens["exceed_512_ratio"], "tokens.exceed_512_ratio")
    require_close(saturation, exact_ratio(expected_exceeds["exceed_512"], observed), "tokens.exceed_512_ratio")
    work = closed(report["aggregate_active_token_work"], {"budget_512", "budget_384", "budget_256", "reduction_384_vs_512", "reduction_256_vs_512"}, "work")
    work_512, work_384, work_256 = (count(work[key], f"work.{key}") for key in ("budget_512", "budget_384", "budget_256"))
    if not work_512 >= work_384 >= work_256:
        fail("work: budgets are not monotonic")
    require_close(ratio(work["reduction_384_vs_512"], "work.reduction_384_vs_512"), exact_ratio(work_512 - work_384, work_512), "work.reduction_384_vs_512")
    require_close(ratio(work["reduction_256_vs_512"], "work.reduction_256_vs_512"), exact_ratio(work_512 - work_256, work_512), "work.reduction_256_vs_512")
    families = closed(report["section_coverage"], FAMILIES, "section_coverage")
    for family, value in families.items():
        coverage = closed(value, {"present_documents", "budgets"}, f"section_coverage.{family}")
        present = count(coverage["present_documents"], f"section_coverage.{family}.present_documents")
        if present > observed:
            fail(f"section_coverage.{family}: presence exceeds observed")
        for budget, bucket in closed(coverage["budgets"], BUDGETS, f"section_coverage.{family}.budgets").items():
            values = closed(bucket, COVERAGE, f"section_coverage.{family}.{budget}")
            if sum(count(item, f"section_coverage.{family}.{budget}.{key}") for key, item in values.items()) != present:
                fail(f"section_coverage.{family}.{budget}: counts do not reconcile")
    priority = closed(report["priority_coverage_512"], {"documents_present", "documents_below_half", "below_half_ratio"}, "priority")
    priority_present = count(priority["documents_present"], "priority.documents_present")
    priority_low = count(priority["documents_below_half"], "priority.documents_below_half")
    if priority_low > priority_present or priority_present > observed:
        fail("priority: counts are inconsistent")
    priority_ratio = ratio(priority["below_half_ratio"], "priority.below_half_ratio")
    require_close(priority_ratio, exact_ratio(priority_low, priority_present), "priority.below_half_ratio")
    expected_triggers = {
        "minimum_documents": observed >= 1_000,
        "saturation_over_512": saturation >= 0.25,
        "priority_loss_at_512": priority_present > 0 and priority_ratio >= 0.10,
        "work_reduction_at_384": ratio(work["reduction_384_vs_512"], "work.reduction_384_vs_512") >= 0.10,
    }
    expected_triggers["all"] = all(expected_triggers.values())
    triggers = closed(report["triggers"], set(expected_triggers), "triggers")
    if triggers != expected_triggers:
        fail("triggers: decision inputs are inconsistent")
    if report["decision"] != ("l1_eligible" if expected_triggers["all"] else "lost"):
        fail("decision: inconsistent with triggers")
    privacy_keys = {"contains_raw_text", "contains_token_ids", "contains_per_document_rows", "contains_paths", "contains_names", "contains_direct_raw_hashes"}
    if any(value is not False for value in closed(report["privacy"], privacy_keys, "privacy").values()):
        fail("privacy: all leak flags must be false")
    if report["claims"] != ["observation_only", "no_product_speedup", "no_quality_claim", "no_release_claim"]:
        fail("claims: unsupported claim set")


def mutate(report: object, mutation: str) -> object:
    candidate = copy.deepcopy(report)
    if mutation == "unknown_field": candidate["private"] = "payload"
    elif mutation == "negative_count": candidate["documents"]["observed"] = -1
    elif mutation == "nan_ratio": candidate["priority_coverage_512"]["below_half_ratio"] = float("nan")
    elif mutation == "document_mismatch": candidate["documents"]["selected"] += 1
    elif mutation == "bucket_mismatch": candidate["pre_truncation_active_tokens"]["buckets"]["le_256"] += 1
    elif mutation == "coverage_mismatch": candidate["section_coverage"]["skill"]["budgets"]["512"]["complete_loss"] += 1
    elif mutation == "trigger_mismatch": candidate["triggers"]["saturation_over_512"] = False
    elif mutation == "decision_mismatch": candidate["decision"] = "lost"
    elif mutation == "privacy_true": candidate["privacy"]["contains_raw_text"] = True
    elif mutation == "path_signal": candidate["scope"] = "/Users/private/resume.pdf"
    elif mutation == "oversized": candidate["scope"] = "x" * (64 * 1024)
    else: fail(f"invalid fixture: unknown mutation {mutation}")
    return candidate


def main() -> int:
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
    print(f"embedding input observation contract check passed ({len(cases)} negative cases)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as error:
        print(f"check-embedding-input-observation.py failed: {error}", file=sys.stderr)
        raise SystemExit(1)
