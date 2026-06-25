#!/usr/bin/env python3
"""Validate public performance-goal contracts and schema fixtures.

This is intentionally standard-library only. It is a CI guard for public
contract files, not a replacement for a full JSON Schema implementation.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tomllib
from collections.abc import Mapping


ROOT = pathlib.Path(__file__).resolve().parents[2]
PERF = ROOT / "perf"
VALID_FIXTURES = PERF / "fixtures" / "valid"
INVALID_FIXTURES = PERF / "fixtures" / "invalid"
FOCUSED_CHECKS = [
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-loop-state.py",
    "scripts/ci/check-experiment-report.py",
    "scripts/ci/check-pr-budget.py",
    "scripts/ci/check-benchmark-lanes.py",
    "scripts/ci/check-private-evidence-redaction.py",
    "scripts/ci/check-gate-integrity.py",
    "scripts/ci/check-goal-complete.py",
]

HEX64 = set("0123456789abcdef")
REQUIRED_BUCKETS = [
    "single_term",
    "and_2",
    "and_3_5",
    "and_6_16",
    "field_filter",
    "hybrid",
    "semantic",
    "extreme",
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


def require_non_empty_string(value: object, path: str) -> None:
    if not isinstance(value, str) or not value:
        fail(f"{path}: expected non-empty string")


def require_enum(value: object, allowed: list[str] | set[str], path: str) -> None:
    if value not in allowed:
        fail(f"{path}: invalid value {value!r}")


def require_bool_fields(value: Mapping[str, object], fields: list[str], expected: bool, path: str) -> None:
    for field in fields:
        require_bool(value.get(field), expected, f"{path}.{field}")


def require_main_reachable_commit(value: object, path: str) -> None:
    if not isinstance(value, str) or not value:
        fail(f"{path}: expected main-reachable git commit")
    if value == "working-tree":
        fail(f"{path}: expected main-reachable git commit, got working-tree")


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
    all_of = require_list(schema.get("allOf"), f"{path}.allOf")
    if not all_of:
        fail(f"{path}: must use conditional schema rules")
    defs = require_mapping(schema.get("$defs"), f"{path}.$defs")
    for required_def in ["contract_pins", "privacy"]:
        if required_def not in defs:
            fail(f"{path}: missing $defs.{required_def}")


def validate_matrix(matrix: Mapping[str, object]) -> None:
    if matrix.get("schema_version") != "resume-ir.perf.acceptance-matrix.v2":
        fail("perf/acceptance-matrix.toml: expected v2 schema")
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
    if matrix.get("gui_redlines", {}).get("visible_rows_min") != 20:
        fail("matrix.gui_redlines.visible_rows_min must be 20")
    if matrix.get("gui_redlines", {}).get("visible_rows_max") != 60:
        fail("matrix.gui_redlines.visible_rows_max must be 60")

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
    require_number_at_least(gui.get("visible_rows"), matrix["gui_redlines"]["visible_rows_min"], f"{path}.gui_manual.visible_rows")
    require_number_at_most(gui.get("visible_rows"), matrix["gui_redlines"]["visible_rows_max"], f"{path}.gui_manual.visible_rows")
    require_number_at_most(gui.get("input_to_paint_p95_ms"), matrix["gui_redlines"]["input_to_paint_p95_ms"], f"{path}.gui_manual.input_to_paint_p95_ms")
    require_number_at_most(gui.get("frame_time_p95_ms"), matrix["gui_redlines"]["frame_time_p95_ms"], f"{path}.gui_manual.frame_time_p95_ms")
    require_number_at_most(gui.get("scroll_dropped_frame_pct"), matrix["gui_redlines"]["scroll_dropped_frame_pct_max"], f"{path}.gui_manual.scroll_dropped_frame_pct")
    require_bool(gui.get("journey_checklist_passed"), True, f"{path}.gui_manual.journey_checklist_passed")


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
        validate_thresholds(report, path)
    else:
        fail(f"{path}.evidence_lane invalid")


def validate_loop_state(value: object, matrix: Mapping[str, object], path: str) -> None:
    state = require_mapping(value, path)
    if state.get("schema_version") != "resume-ir.loop-state-report.v2":
        fail(f"{path}.schema_version mismatch")
    if state.get("goal_id") != "resume-ir.performance-gui-loop.2026-06":
        fail(f"{path}.goal_id mismatch")
    validate_contract_pins(state.get("contract_pins"), f"{path}.contract_pins")
    validate_privacy(state, trace_required=False, path=path)
    allowed_paths = require_list(state.get("allowed_paths"), f"{path}.allowed_paths")
    if not allowed_paths:
        fail(f"{path}.allowed_paths: must not be empty")
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
    if path.name.startswith("loop") or "loop-state" in path.name:
        validate_loop_state(value, matrix, str(path.relative_to(ROOT)))
    else:
        validate_experiment_report(value, matrix, str(path.relative_to(ROOT)))


def run_focused_checks() -> None:
    for rel in FOCUSED_CHECKS:
        completed = subprocess.run([sys.executable, str(ROOT / rel)], check=False)
        if completed.returncode != 0:
            fail(f"{rel}: focused check failed with exit code {completed.returncode}")


def main() -> int:
    matrix = load_toml(PERF / "acceptance-matrix.toml")
    validate_matrix(matrix)
    validate_schema_file(
        require_mapping(load_json(PERF / "experiment-report.schema.json"), "experiment schema"),
        "perf/experiment-report.schema.json",
        "resume-ir.experiment-report.v2",
    )
    validate_schema_file(
        require_mapping(load_json(PERF / "loop-state.schema.json"), "loop schema"),
        "perf/loop-state.schema.json",
        "resume-ir.loop-state-report.v2",
    )
    validate_loop_state(load_json(PERF / "current-loop-state.json"), matrix, "perf/current-loop-state.json")

    for path in sorted(VALID_FIXTURES.glob("*.json")):
        validate_fixture(path, matrix)

    invalid_count = 0
    for path in sorted(INVALID_FIXTURES.glob("*.json")):
        invalid_count += 1
        try:
            validate_fixture(path, matrix)
        except ValueError:
            continue
        fail(f"{path.relative_to(ROOT)}: invalid fixture unexpectedly passed")
    if invalid_count == 0:
        fail("no invalid fixtures found")

    run_focused_checks()

    print("performance contract check passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"performance contract check failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
