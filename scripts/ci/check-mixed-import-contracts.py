#!/usr/bin/env python3
"""Validate frozen mixed-import benchmark fixtures and public-safe reports."""

from __future__ import annotations

import copy
import hashlib
import hmac
import json
import math
import pathlib
import re
import runpy
import sys
import tomllib
from collections import Counter
from collections.abc import Mapping


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "perf" / "fixtures" / "mixed-import"
SCHEMA = ROOT / "perf" / "mixed-import-report.schema.json"
PUBLIC_FIXTURE = FIXTURES / "public-synthetic-benchmark.json"
VALID_REPORT = FIXTURES / "valid-public-report.json"
INVALID_CASES = FIXTURES / "invalid-cases.json"
PUBLIC_SYNTHETIC_HMAC_KEY = b"resume-ir-public-synthetic-freeze-v1"
REPORT_VERSION = "resume-ir.mixed-import-report.v1"
REPORT_KEYS = {
    "schema_version", "benchmark_layer", "claim", "freeze", "visibility",
    "sample_counts", "label_counts", "extension_buckets", "size_buckets",
    "directory_depth_buckets", "classification_counts", "index_admission",
    "timings_ms", "throughput", "resource_budget", "privacy",
}
STATUSES = {"resume_candidate", "non_resume", "needs_review", "ocr_backlog", "failed"}
FORBIDDEN_REPORT_KEYS = {
    "raw_path", "filename", "body_text", "raw_label_details", "raw_file_hash",
    "raw_query", "candidate_results", "token", "diagnostics_package",
    "private_manifest", "combined_layers",
}
PRIVACY_FALSE_FIELDS = {
    "contains_raw_resume_text", "contains_raw_query_text", "contains_candidate_results",
    "contains_local_paths", "contains_filenames", "contains_label_details",
    "contains_raw_file_hashes", "contains_tokens", "contains_diagnostics_package",
    "contains_private_manifest",
}
NEGATIVE_CASE_NAMES = {
    "raw_path", "filename", "body_text", "raw_label_details", "raw_file_hash",
    "raw_query", "candidate_results", "token", "diagnostics_package",
    "private_manifest", "layer_mixing", "mutable_after_freeze",
    "holdout_visible_during_calibration", "precision_below_threshold",
    "contamination_above_threshold",
}


def fail(message: str) -> None:
    raise ValueError(message)


def load_json(path: pathlib.Path) -> object:
    with path.open("rb") as handle:
        return json.load(handle)


def require_mapping(value: object, path: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        fail(f"{path}: expected object")
    return value


def require_list(value: object, path: str) -> list[object]:
    if not isinstance(value, list):
        fail(f"{path}: expected array")
    return value


def exact_keys(value: Mapping[str, object], expected: set[str], path: str) -> None:
    if set(value) != expected:
        fail(f"{path}: keys mismatch")


def number(value: object, path: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        fail(f"{path}: expected finite number")
    return float(value)


def ratio(value: object, path: str) -> float:
    result = number(value, path)
    if not 0 <= result <= 1:
        fail(f"{path}: expected ratio in [0, 1]")
    return result


def count(value: object, path: str, *, positive: bool = False) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < int(positive):
        fail(f"{path}: expected {'positive' if positive else 'non-negative'} integer")
    return value


def validate_schema(schema: Mapping[str, object]) -> None:
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        fail("mixed-import schema: wrong draft")
    if schema.get("additionalProperties") is not False:
        fail("mixed-import schema: root must reject additional properties")
    properties = require_mapping(schema.get("properties"), "mixed-import schema.properties")
    required = set(require_list(schema.get("required"), "mixed-import schema.required"))
    if set(properties) != REPORT_KEYS or required != REPORT_KEYS:
        fail("mixed-import schema: root fields drifted from checker")
    version = require_mapping(properties.get("schema_version"), "mixed-import schema.version")
    if version.get("const") != REPORT_VERSION:
        fail("mixed-import schema: version mismatch")


def validate_buckets(value: object, total: int, path: str) -> dict[str, int]:
    items = require_list(value, path)
    if not 1 <= len(items) <= 16:
        fail(f"{path}: expected 1..16 buckets")
    result: dict[str, int] = {}
    for index, item in enumerate(items):
        entry = require_mapping(item, f"{path}[{index}]")
        exact_keys(entry, {"name", "count"}, f"{path}[{index}]")
        name = entry.get("name")
        if not isinstance(name, str) or not re.fullmatch(r"[a-z0-9_]{1,32}", name) or name in result:
            fail(f"{path}[{index}].name: invalid or duplicate")
        result[name] = count(entry.get("count"), f"{path}[{index}].count")
    if sum(result.values()) != total:
        fail(f"{path}: counts do not sum to sample total")
    return result


def walk_forbidden_keys(value: object, path: str = "report") -> None:
    if isinstance(value, Mapping):
        for key, child in value.items():
            if key in FORBIDDEN_REPORT_KEYS:
                fail(f"{path}.{key}: forbidden public detail")
            walk_forbidden_keys(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk_forbidden_keys(child, f"{path}[{index}]")


def validate_report(report: Mapping[str, object], matrix: Mapping[str, object]) -> None:
    if len(json.dumps(report, separators=(",", ":")).encode()) > 32_768:
        fail("report: exceeds bounded public size")
    exact_keys(report, REPORT_KEYS, "report")
    walk_forbidden_keys(report)
    mixed = require_mapping(matrix.get("mixed_import_correctness"), "matrix.mixed_import_correctness")
    if report.get("schema_version") != REPORT_VERSION or report.get("claim") != "no_claim":
        fail("report: schema_version or claim mismatch")
    layer = report.get("benchmark_layer")
    if layer not in mixed.get("benchmark_layers", []):
        fail("report.benchmark_layer: invalid")

    freeze = require_mapping(report.get("freeze"), "report.freeze")
    exact_keys(freeze, {"identity_scheme", "opaque_manifest_id", "frozen", "mutation_after_freeze_allowed"}, "report.freeze")
    if freeze.get("identity_scheme") != mixed.get("freeze_identity_scheme"):
        fail("report.freeze.identity_scheme mismatch")
    if not isinstance(freeze.get("opaque_manifest_id"), str) or not re.fullmatch(r"[a-f0-9]{64}", freeze["opaque_manifest_id"]):
        fail("report.freeze.opaque_manifest_id: expected lowercase hex64")
    if freeze.get("frozen") is not True or freeze.get("mutation_after_freeze_allowed") is not False:
        fail("report.freeze: benchmark must be immutable")

    visibility = require_mapping(report.get("visibility"), "report.visibility")
    exact_keys(visibility, {"aggregate_only", "per_file_details", "calibration_labels_visible_to_tuner", "blind_holdout_visible_during_calibration"}, "report.visibility")
    if visibility.get("aggregate_only") is not True or visibility.get("per_file_details") is not False:
        fail("report.visibility: public output must be aggregate only")
    if visibility.get("blind_holdout_visible_during_calibration") is not False:
        fail("report.visibility: blind holdout leaked into calibration")
    if (layer == "private_calibration") != (visibility.get("calibration_labels_visible_to_tuner") is True):
        fail("report.visibility: calibration visibility/layer mismatch")

    samples = require_mapping(report.get("sample_counts"), "report.sample_counts")
    exact_keys(samples, {"total"}, "report.sample_counts")
    total = count(samples.get("total"), "report.sample_counts.total", positive=True)
    labels = require_mapping(report.get("label_counts"), "report.label_counts")
    exact_keys(labels, {"known_resume", "known_non_resume", "unknown"}, "report.label_counts")
    label_total = sum(count(labels.get(key), f"report.label_counts.{key}") for key in labels)
    if label_total != total:
        fail("report.label_counts: counts do not sum to sample total")
    for field in ["extension_buckets", "size_buckets", "directory_depth_buckets"]:
        validate_buckets(report.get(field), total, f"report.{field}")

    classifications = require_mapping(report.get("classification_counts"), "report.classification_counts")
    exact_keys(classifications, STATUSES, "report.classification_counts")
    if sum(count(classifications.get(key), f"report.classification_counts.{key}") for key in STATUSES) != total:
        fail("report.classification_counts: counts do not sum to sample total")
    admission = require_mapping(report.get("index_admission"), "report.index_admission")
    admission_keys = {"searchable_count", "indexed_total", "indexed_true_resumes", "indexed_resume_precision", "contamination_count", "expected_true_resumes", "resume_completeness", "precision_regression", "completeness_improvement_claimed"}
    exact_keys(admission, admission_keys, "report.index_admission")
    indexed = count(admission.get("indexed_total"), "report.index_admission.indexed_total", positive=True)
    indexed_true = count(admission.get("indexed_true_resumes"), "report.index_admission.indexed_true_resumes")
    searchable = count(admission.get("searchable_count"), "report.index_admission.searchable_count")
    contamination = count(admission.get("contamination_count"), "report.index_admission.contamination_count")
    expected_true = count(admission.get("expected_true_resumes"), "report.index_admission.expected_true_resumes", positive=True)
    precision = ratio(admission.get("indexed_resume_precision"), "report.index_admission.indexed_resume_precision")
    completeness = ratio(admission.get("resume_completeness"), "report.index_admission.resume_completeness")
    regression = ratio(admission.get("precision_regression"), "report.index_admission.precision_regression")
    if indexed != classifications.get("resume_candidate") or searchable != indexed:
        fail("report.index_admission: only resume_candidate may be searchable/indexed")
    if contamination != indexed - indexed_true or not math.isclose(precision, indexed_true / indexed):
        fail("report.index_admission: precision/contamination formula mismatch")
    if expected_true != labels.get("known_resume") or not math.isclose(completeness, indexed_true / expected_true):
        fail("report.index_admission: completeness formula mismatch")
    if precision < number(mixed.get("indexed_resume_precision_min"), "matrix indexed precision"):
        fail("report.index_admission: precision below threshold")
    if contamination > count(mixed.get("contamination_count_max"), "matrix contamination"):
        fail("report.index_admission: contamination above threshold")
    if regression > number(mixed.get("precision_non_regression_tolerance"), "matrix precision tolerance"):
        fail("report.index_admission: precision regression above tolerance")
    if not isinstance(admission.get("completeness_improvement_claimed"), bool):
        fail("report.index_admission.completeness_improvement_claimed: expected boolean")

    timings = require_mapping(report.get("timings_ms"), "report.timings_ms")
    exact_keys(timings, {"clean_wall", "mixed_wall", "mixed_overhead_pct", "stage_timings"}, "report.timings_ms")
    clean = number(timings.get("clean_wall"), "report.timings_ms.clean_wall")
    mixed_wall = number(timings.get("mixed_wall"), "report.timings_ms.mixed_wall")
    overhead = number(timings.get("mixed_overhead_pct"), "report.timings_ms.mixed_overhead_pct")
    if clean <= 0 or mixed_wall <= 0 or not math.isclose(overhead, (mixed_wall - clean) * 100 / clean):
        fail("report.timings_ms: wall-time overhead mismatch")
    stages = require_list(timings.get("stage_timings"), "report.timings_ms.stage_timings")
    if not 5 <= len(stages) <= 6:
        fail("report.timings_ms.stage_timings: expected 5..6 stages")
    stage_names = set()
    for index, stage in enumerate(stages):
        entry = require_mapping(stage, f"report.timings_ms.stage_timings[{index}]")
        exact_keys(entry, {"name", "value"}, f"report.timings_ms.stage_timings[{index}]")
        name = entry.get("name")
        if name not in {"scan", "parse", "normalize_sectionize", "classify", "index", "ocr"} or name in stage_names:
            fail("report.timings_ms.stage_timings: invalid or duplicate stage")
        stage_names.add(name)
        if number(entry.get("value"), f"report stage {name}") < 0:
            fail("report.timings_ms.stage_timings: negative value")

    throughput = require_mapping(report.get("throughput"), "report.throughput")
    exact_keys(throughput, {"content_bytes_read", "docs_per_second", "mib_per_second"}, "report.throughput")
    count(throughput.get("content_bytes_read"), "report.throughput.content_bytes_read")
    if number(throughput.get("docs_per_second"), "report.throughput.docs_per_second") < 0 or number(throughput.get("mib_per_second"), "report.throughput.mib_per_second") < 0:
        fail("report.throughput: values must be non-negative")
    resources = require_mapping(report.get("resource_budget"), "report.resource_budget")
    exact_keys(resources, {"h_tier", "aggregate_mb", "writer_heap_mb", "parser_workers"}, "report.resource_budget")
    profiles = require_mapping(matrix.get("governor_profiles"), "matrix.governor_profiles")
    profile = require_mapping(profiles.get(resources.get("h_tier")), "matrix.governor_profiles selected tier")
    for field in ["aggregate_mb", "writer_heap_mb", "parser_workers"]:
        count(resources.get(field), f"report.resource_budget.{field}", positive=True)
    if (resources.get("aggregate_mb"), resources.get("writer_heap_mb"), resources.get("parser_workers")) != (profile.get("max_private_or_anonymous_mb"), profile.get("writer_heap_mb"), profile.get("parser_concurrency")):
        fail("report.resource_budget: values do not match H-tier")
    privacy = require_mapping(report.get("privacy"), "report.privacy")
    exact_keys(privacy, PRIVACY_FALSE_FIELDS | {"aggregate_only"}, "report.privacy")
    if privacy.get("aggregate_only") is not True or any(privacy.get(field) is not False for field in PRIVACY_FALSE_FIELDS):
        fail("report.privacy: public evidence boundary violated")


def canonical_hmac(samples: list[object]) -> str:
    payload = json.dumps(samples, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return hmac.new(PUBLIC_SYNTHETIC_HMAC_KEY, payload, hashlib.sha256).hexdigest()


def size_bucket(content: str) -> str:
    size = len(content.encode())
    if size == 0:
        return "empty"
    if size < 128:
        return "bytes_1_127"
    if size < 1024:
        return "bytes_128_1023"
    return "bytes_1024_plus"


def validate_public_fixture(value: Mapping[str, object], matrix: Mapping[str, object]) -> list[Mapping[str, object]]:
    exact_keys(value, {"schema_version", "fixture_role", "synthetic_only", "contains_real_personal_data", "freeze", "samples"}, "public fixture")
    if value.get("schema_version") != "resume-ir.public-synthetic-mixed-benchmark.v1" or value.get("fixture_role") != "frozen_classifier_independent_smoke":
        fail("public fixture: identity mismatch")
    if value.get("synthetic_only") is not True or value.get("contains_real_personal_data") is not False:
        fail("public fixture: must be synthetic and public-safe")
    freeze = require_mapping(value.get("freeze"), "public fixture.freeze")
    exact_keys(freeze, {"identity_scheme", "opaque_manifest_id", "frozen", "mutation_after_freeze_allowed"}, "public fixture.freeze")
    mixed = require_mapping(matrix.get("mixed_import_correctness"), "matrix.mixed_import_correctness")
    expected_paths = {
        "report_schema_path": "perf/mixed-import-report.schema.json",
        "contract_checker_target": "scripts/ci/check-mixed-import-contracts.py",
        "public_synthetic_fixture_path": "perf/fixtures/mixed-import/public-synthetic-benchmark.json",
        "valid_report_fixture_path": "perf/fixtures/mixed-import/valid-public-report.json",
        "negative_cases_fixture_path": "perf/fixtures/mixed-import/invalid-cases.json",
    }
    if any(mixed.get(key) != expected for key, expected in expected_paths.items()) or mixed.get("report_contract_implemented") is not True:
        fail("matrix.mixed_import_correctness: implemented contract paths mismatch")
    if freeze.get("identity_scheme") != mixed.get("freeze_identity_scheme") or freeze.get("frozen") is not True or freeze.get("mutation_after_freeze_allowed") is not False:
        fail("public fixture.freeze: invalid freeze boundary")
    samples = require_list(value.get("samples"), "public fixture.samples")
    if not 5 <= len(samples) <= 64:
        fail("public fixture.samples: expected 5..64 samples")
    if len(samples) != mixed.get("public_synthetic_sample_count"):
        fail("public fixture.samples: frozen sample count mismatch")
    checked: list[Mapping[str, object]] = []
    sample_ids = set()
    for index, item in enumerate(samples):
        sample = require_mapping(item, f"public fixture.samples[{index}]")
        exact_keys(sample, {"sample_id", "virtual_relative_path", "extension", "ground_truth", "parser_outcome", "expected_status", "content"}, f"public fixture.samples[{index}]")
        sample_id = sample.get("sample_id")
        if not isinstance(sample_id, str) or not re.fullmatch(r"public-[0-9]{3}", sample_id) or sample_id in sample_ids:
            fail("public fixture: invalid or duplicate sample_id")
        sample_ids.add(sample_id)
        relative = sample.get("virtual_relative_path")
        if not isinstance(relative, str) or relative.startswith(("/", "~")) or "\\" in relative:
            fail("public fixture: path must be synthetic relative POSIX")
        path = pathlib.PurePosixPath(relative)
        if not path.name or ".." in path.parts or path.suffix.lower().lstrip(".") != sample.get("extension"):
            fail("public fixture: unsafe path or extension mismatch")
        truth = sample.get("ground_truth")
        outcome = sample.get("parser_outcome")
        status = sample.get("expected_status")
        expected = "ocr_backlog" if outcome == "ocr_required" else "failed" if outcome == "failed" else "resume_candidate" if truth == "resume" else "non_resume" if truth == "non_resume" else "needs_review"
        if truth not in {"resume", "non_resume", "unknown"} or outcome not in {"text_extracted", "ocr_required", "failed"} or status != expected:
            fail("public fixture: ground truth/parser/status mismatch")
        content = sample.get("content")
        if not isinstance(content, str) or "@" in content or re.search(r"\b[0-9]{7,}\b", content):
            fail("public fixture: content must remain synthetic and contact-free")
        checked.append(sample)
    if freeze.get("opaque_manifest_id") != canonical_hmac(samples):
        fail("public fixture.freeze.opaque_manifest_id mismatch")
    return checked


def bucket_entries(counter: Counter[str]) -> dict[str, int]:
    return dict(sorted(counter.items()))


def validate_public_pair(report: Mapping[str, object], samples: list[Mapping[str, object]]) -> None:
    total = len(samples)
    labels = Counter(str(sample["ground_truth"]) for sample in samples)
    statuses = Counter(str(sample["expected_status"]) for sample in samples)
    extensions = Counter(str(sample["extension"]) for sample in samples)
    sizes = Counter(size_bucket(str(sample["content"])) for sample in samples)
    depths = Counter(f"depth_{len(pathlib.PurePosixPath(str(sample['virtual_relative_path'])).parts) - 1}" for sample in samples)
    if report.get("benchmark_layer") != "public_synthetic":
        fail("public pair: report layer mismatch")
    if require_mapping(report["freeze"], "report.freeze").get("opaque_manifest_id") != canonical_hmac(list(samples)):
        fail("public pair: freeze identity mismatch")
    if report.get("sample_counts") != {"total": total}:
        fail("public pair: sample count mismatch")
    if report.get("label_counts") != {"known_resume": labels["resume"], "known_non_resume": labels["non_resume"], "unknown": labels["unknown"]}:
        fail("public pair: label counts mismatch")
    for field, expected in [("extension_buckets", extensions), ("size_buckets", sizes), ("directory_depth_buckets", depths)]:
        if validate_buckets(report.get(field), total, f"report.{field}") != bucket_entries(expected):
            fail(f"public pair: {field} mismatch")
    if report.get("classification_counts") != {status: statuses[status] for status in STATUSES}:
        fail("public pair: classification counts mismatch")
    content_bytes = sum(len(str(sample["content"]).encode()) for sample in samples)
    throughput = require_mapping(report.get("throughput"), "report.throughput")
    mixed_wall = number(require_mapping(report.get("timings_ms"), "report.timings_ms").get("mixed_wall"), "report.timings_ms.mixed_wall")
    if throughput.get("content_bytes_read") != content_bytes:
        fail("public pair: content byte count mismatch")
    if not math.isclose(number(throughput.get("docs_per_second"), "docs_per_second"), total * 1000 / mixed_wall):
        fail("public pair: docs_per_second mismatch")
    if not math.isclose(number(throughput.get("mib_per_second"), "mib_per_second"), content_bytes * 1000 / mixed_wall / 1_048_576, rel_tol=1e-9):
        fail("public pair: mib_per_second mismatch")
    serialized = json.dumps(report, ensure_ascii=False)
    for sample in samples:
        forbidden = [str(sample["sample_id"]), str(sample["virtual_relative_path"]), pathlib.PurePosixPath(str(sample["virtual_relative_path"])).name]
        if any(item and item in serialized for item in forbidden):
            fail("public pair: per-file detail leaked into report")


def apply_mutation(value: dict[str, object], dotted_path: str, replacement: object) -> None:
    parts = dotted_path.split(".")
    target: dict[str, object] = value
    for part in parts[:-1]:
        child = target.get(part)
        if not isinstance(child, dict):
            fail(f"negative fixture path {dotted_path}: invalid parent")
        target = child
    target[parts[-1]] = replacement


def validate_negative_cases(value: Mapping[str, object], base: Mapping[str, object], matrix: Mapping[str, object], samples: list[Mapping[str, object]]) -> None:
    exact_keys(value, {"schema_version", "cases"}, "negative cases")
    if value.get("schema_version") != "resume-ir.mixed-import-negative-cases.v1":
        fail("negative cases: schema_version mismatch")
    cases = require_list(value.get("cases"), "negative cases.cases")
    observed = set()
    for index, item in enumerate(cases):
        case = require_mapping(item, f"negative cases[{index}]")
        exact_keys(case, {"name", "path", "value"}, f"negative cases[{index}]")
        name = case.get("name")
        path = case.get("path")
        if not isinstance(name, str) or not isinstance(path, str) or name in observed:
            fail("negative cases: invalid name/path")
        observed.add(name)
        candidate = copy.deepcopy(dict(base))
        apply_mutation(candidate, path, case.get("value"))
        try:
            validate_report(candidate, matrix)
            validate_public_pair(candidate, samples)
        except ValueError:
            continue
        fail(f"negative case unexpectedly passed: {name}")
    if observed != NEGATIVE_CASE_NAMES:
        fail("negative cases: required leakage/precision coverage mismatch")


def main() -> int:
    with (ROOT / "perf" / "acceptance-matrix.toml").open("rb") as handle:
        matrix = tomllib.load(handle)
    schema = require_mapping(load_json(SCHEMA), "mixed-import schema")
    fixture = require_mapping(load_json(PUBLIC_FIXTURE), "public fixture")
    report = require_mapping(load_json(VALID_REPORT), "valid report")
    negative = require_mapping(load_json(INVALID_CASES), "negative cases")
    validate_schema(schema)
    samples = validate_public_fixture(fixture, matrix)
    validate_report(report, matrix)
    validate_public_pair(report, samples)
    validate_negative_cases(negative, report, matrix, samples)
    local_tool = runpy.run_path(str(ROOT / "scripts" / "local" / "prepare-mixed-import-benchmark.py"))
    smoke = local_tool.get("run_synthetic_smoke")
    if not callable(smoke):
        fail("mixed import benchmark smoke: missing callable")
    smoke_result = smoke()
    if not isinstance(smoke_result, dict) or smoke_result.get("sample_count", 0) < 64:
        fail("mixed import benchmark smoke: invalid aggregate result")
    print("mixed import contract check passed "
          f"({len(samples)} frozen synthetic samples, {len(NEGATIVE_CASE_NAMES)} negative cases, local freezer smoke)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as exc:
        print(f"mixed import contract check failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
