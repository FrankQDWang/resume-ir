#!/usr/bin/env python3
"""Run the bounded Issue #290 resident ONNX prepacking trade-off experiment."""

from __future__ import annotations

import argparse
import json
import math
import os
import random
import re
import select
import signal
import statistics
import subprocess
import tempfile
import time
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import Callable


STREAM_SCHEMA = "resume-ir.embedding-stream.v1"
REPORT_SCHEMA = "resume-ir.embedding-prepacking-witness.v2"
MODEL_ID = "intfloat-multilingual-e5-small-qint8-r1"
DIMENSION = 384
PAIR_COUNT = 24
MEASURED_REQUESTS = 4
EXPECTED_INPUTS = 4
EXPECTED_ACTIVE_TOKENS = 2_048
MAX_FRAME_BYTES = 4 * 1024 * 1024
MAX_REPORT_BYTES = 64 * 1024
MIN_ONNX_IMPROVEMENT_PCT = 10.0
MAX_READY_REGRESSION_PCT = 10.0
MAX_READY_REGRESSION_MS = 1_000.0
H0_FOOTPRINT_LIMIT_BYTES = 1_024 * 1024 * 1024
H2_MEMORY_LIMIT_BYTES = 3_072 * 1024 * 1024
QUALITY_QUERY_COUNT = 20
QUALITY_PASSAGE_COUNT = 40
QUALITY_TOP_K = 10
MIN_VECTOR_COSINE = 0.99999
MAX_ABSOLUTE_DELTA = 1e-4
MAX_MEAN_ABSOLUTE_DELTA = 1e-6
MIN_TOP_K_OVERLAP = 0.995
MIN_PER_QUERY_TOP_K_OVERLAP = 0.90
MIN_TOP_1_AGREEMENT = 0.995
MAX_NDCG_DROP = 0.002
MAX_VECTOR_NORM_ERROR = 1e-4
PRIVACY = {
    "contains_raw_resume_text": False,
    "contains_raw_query": False,
    "contains_candidate_results": False,
    "contains_local_paths": False,
    "contains_vectors": False,
    "contains_token_content": False,
    "contains_runtime_or_model_bytes": False,
    "contains_pids": False,
    "contains_raw_profiler_data": False,
}


class WitnessError(RuntimeError):
    """A fixed, public-safe witness failure."""


@dataclass(frozen=True)
class Variant:
    label: str
    binary: Path
    revision: str


@dataclass(frozen=True)
class LaunchResult:
    ready_ms: float
    onnx_us: tuple[int, ...]
    vectors: tuple[tuple[tuple[float, ...], ...], ...]


@dataclass(frozen=True)
class QualityResult:
    vectors: tuple[tuple[float, ...], ...]
    token_buckets: tuple[int, ...]
    telemetry_signatures: tuple[tuple[int, int, int], ...]


@dataclass(frozen=True)
class ResourceResult:
    warm_rss_bytes: int
    peak_rss_bytes: int
    peak_physical_footprint_bytes: int


def read_exact(stream: object, size: int, timeout: float) -> bytes:
    descriptor = stream.fileno()  # type: ignore[attr-defined]
    deadline = time.monotonic() + timeout
    chunks: list[bytes] = []
    while size:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([descriptor], [], [], remaining)[0]:
            raise WitnessError("resident_response_timeout")
        chunk = os.read(descriptor, size)
        if not chunk:
            raise WitnessError("resident_response_truncated")
        chunks.append(chunk)
        size -= len(chunk)
    return b"".join(chunks)


def read_frame(stream: object, timeout: float) -> dict[str, object]:
    size = int.from_bytes(read_exact(stream, 4, timeout), "big")
    if not 0 < size <= MAX_FRAME_BYTES:
        raise WitnessError("resident_frame_invalid")
    try:
        value = json.loads(read_exact(stream, size, timeout))
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise WitnessError("resident_payload_invalid") from None
    if not isinstance(value, dict):
        raise WitnessError("resident_payload_invalid")
    return value


def write_frame(stream: object, value: dict[str, object]) -> None:
    encoded = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
    if not 0 < len(encoded) <= MAX_FRAME_BYTES:
        raise WitnessError("request_frame_invalid")
    stream.write(len(encoded).to_bytes(4, "big"))  # type: ignore[attr-defined]
    stream.write(encoded)  # type: ignore[attr-defined]
    stream.flush()  # type: ignore[attr-defined]


def workload() -> list[dict[str, str]]:
    vocabularies = (
        "alpha beta gamma delta epsilon zeta eta theta",
        "iota kappa lambda mu nu xi omicron pi",
        "rho sigma tau upsilon phi chi psi omega",
        "one two three four five six seven eight",
    )
    return [
        {"role": "passage", "text": " ".join([words] * 160)}
        for words in vocabularies
    ]


def quality_workload() -> list[dict[str, str]]:
    topics = (
        "distributed systems rust backend",
        "machine learning python ranking",
        "产品经理 用户研究 数据分析",
        "软件工程 云平台 性能优化",
        "financial reporting risk controls",
        "mobile application ios swift",
        "自然语言处理 搜索 推荐系统",
        "security privacy identity access",
        "sales operations customer success",
        "database storage reliability sql",
    )
    repetitions = (1, 4, 12, 24, 80)
    inputs: list[dict[str, str]] = []
    for index in range(QUALITY_QUERY_COUNT + QUALITY_PASSAGE_COUNT):
        role = "query" if index < QUALITY_QUERY_COUNT else "passage"
        topic = topics[index % len(topics)]
        qualifier = f"synthetic profile {index % 7}"
        text = " ".join([f"{topic} {qualifier}"] * repetitions[index % 5])
        inputs.append({"role": role, "text": text})
    return inputs


def balanced_orders(seed: int, pairs: int = PAIR_COUNT) -> tuple[str, ...]:
    if pairs <= 0 or pairs % 2:
        raise WitnessError("pair_count_not_balanced")
    orders = ["AB"] * (pairs // 2) + ["BA"] * (pairs // 2)
    random.Random(seed).shuffle(orders)
    return tuple(orders)


def environment(runtime_dir: Path, intra_threads: int) -> dict[str, str]:
    result = os.environ.copy()
    result.update(
        {
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(runtime_dir),
            "RESUME_IR_EMBEDDING_MODEL_ID": MODEL_ID,
            "RESUME_IR_EMBEDDING_DIMENSION": str(DIMENSION),
            "RESUME_IR_EMBEDDING_INTRA_THREADS": str(intra_threads),
        }
    )
    return result


def validate_ready(response: dict[str, object]) -> None:
    if response != {
        "type": "ready",
        "schema_version": STREAM_SCHEMA,
        "model_id": MODEL_ID,
        "dimension": DIMENSION,
    }:
        raise WitnessError("ready_identity_mismatch")


def validate_result(
    response: dict[str, object],
    request_id: int,
    expected_inputs: int = EXPECTED_INPUTS,
    expected_active_tokens: int | None = EXPECTED_ACTIVE_TOKENS,
    expected_padded_tokens: int | None = EXPECTED_ACTIVE_TOKENS,
) -> tuple[int, tuple[tuple[float, ...], ...], tuple[int, int, int]]:
    vectors, telemetry = response.get("vectors"), response.get("telemetry")
    if (
        response.get("type") != "result"
        or response.get("schema_version") != STREAM_SCHEMA
        or response.get("request_id") != request_id
        or not isinstance(vectors, list)
        or len(vectors) != expected_inputs
        or not isinstance(telemetry, dict)
    ):
        raise WitnessError("result_contract_mismatch")
    checked: list[tuple[float, ...]] = []
    for vector in vectors:
        if (
            not isinstance(vector, list)
            or len(vector) != DIMENSION
            or any(
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or not math.isfinite(value)
                for value in vector
            )
        ):
            raise WitnessError("vector_contract_mismatch")
        checked_vector = tuple(float(value) for value in vector)
        norm = math.sqrt(sum(value * value for value in checked_vector))
        if abs(norm - 1.0) > MAX_VECTOR_NORM_ERROR:
            raise WitnessError("vector_normalization_mismatch")
        checked.append(checked_vector)
    telemetry_values = tuple(
        telemetry.get(key)
        for key in ("input_count", "active_token_count", "padded_token_count")
    )
    if (
        telemetry_values[0] != expected_inputs
        or not isinstance(telemetry_values[1], int)
        or isinstance(telemetry_values[1], bool)
        or telemetry_values[1] <= 0
        or not isinstance(telemetry_values[2], int)
        or isinstance(telemetry_values[2], bool)
        or telemetry_values[2] < telemetry_values[1]
        or (
            expected_active_tokens is not None
            and telemetry_values[1] != expected_active_tokens
        )
        or (
            expected_padded_tokens is not None
            and telemetry_values[2] != expected_padded_tokens
        )
    ):
        raise WitnessError("workload_token_contract_mismatch")
    onnx_us = telemetry.get("onnx_us")
    if not isinstance(onnx_us, int) or isinstance(onnx_us, bool) or onnx_us <= 0:
        raise WitnessError("onnx_telemetry_invalid")
    return (
        onnx_us,
        tuple(checked),
        (expected_inputs, int(telemetry_values[1]), int(telemetry_values[2])),
    )


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=3)


def launch_variant(
    variant: Variant,
    runtime_dir: Path,
    intra_threads: int,
    timeout: float,
    measured_requests: int = MEASURED_REQUESTS,
) -> LaunchResult:
    stderr = tempfile.TemporaryFile()
    started = time.perf_counter()
    try:
        process = subprocess.Popen(
            [str(variant.binary), "--resident"],
            env=environment(runtime_dir, intra_threads),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=stderr,
            start_new_session=True,
        )
    except OSError:
        stderr.close()
        raise WitnessError("resident_start_failed") from None
    try:
        if process.stdin is None or process.stdout is None:
            raise WitnessError("resident_pipe_unavailable")
        validate_ready(read_frame(process.stdout, timeout))
        ready_ms = (time.perf_counter() - started) * 1_000.0
        request = {
            "schema_version": STREAM_SCHEMA,
            "request_id": 1,
            "model_id": MODEL_ID,
            "dimension": DIMENSION,
            "inputs": workload(),
        }
        write_frame(process.stdin, request)
        validate_result(read_frame(process.stdout, timeout), 1)
        samples: list[int] = []
        vectors: list[tuple[tuple[float, ...], ...]] = []
        for request_id in range(2, measured_requests + 2):
            request["request_id"] = request_id
            write_frame(process.stdin, request)
            response = read_frame(process.stdout, timeout)
            onnx_us, response_vectors, _ = validate_result(response, request_id)
            samples.append(onnx_us)
            vectors.append(response_vectors)
        process.stdin.close()
        if process.wait(timeout=5) != 0:
            raise WitnessError("resident_exit_failed")
        return LaunchResult(ready_ms, tuple(samples), tuple(vectors))
    except (OSError, subprocess.SubprocessError):
        raise WitnessError("resident_io_failed") from None
    finally:
        stop_process(process)
        stderr.close()


def quality_variant(
    variant: Variant, runtime_dir: Path, timeout: float
) -> QualityResult:
    inputs = quality_workload()
    stderr = tempfile.TemporaryFile()
    try:
        process = subprocess.Popen(
            [str(variant.binary), "--resident"],
            env=environment(runtime_dir, 3),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=stderr,
            start_new_session=True,
        )
    except OSError:
        stderr.close()
        raise WitnessError("resident_start_failed") from None
    try:
        if process.stdin is None or process.stdout is None:
            raise WitnessError("resident_pipe_unavailable")
        validate_ready(read_frame(process.stdout, timeout))
        request_id = 1
        token_buckets: list[int] = []
        for representative in inputs[:5]:
            request = {
                "schema_version": STREAM_SCHEMA,
                "request_id": request_id,
                "model_id": MODEL_ID,
                "dimension": DIMENSION,
                "inputs": [representative],
            }
            write_frame(process.stdin, request)
            _, _, telemetry = validate_result(
                read_frame(process.stdout, timeout),
                request_id,
                expected_inputs=1,
                expected_active_tokens=None,
                expected_padded_tokens=None,
            )
            token_buckets.append(telemetry[1])
            request_id += 1
        if len(set(token_buckets)) != 5 or token_buckets != sorted(token_buckets):
            raise WitnessError("quality_token_buckets_invalid")

        vectors: list[tuple[float, ...]] = []
        signatures: list[tuple[int, int, int]] = []
        for offset in range(0, len(inputs), EXPECTED_INPUTS):
            batch = inputs[offset : offset + EXPECTED_INPUTS]
            request = {
                "schema_version": STREAM_SCHEMA,
                "request_id": request_id,
                "model_id": MODEL_ID,
                "dimension": DIMENSION,
                "inputs": batch,
            }
            write_frame(process.stdin, request)
            _, response_vectors, telemetry = validate_result(
                read_frame(process.stdout, timeout),
                request_id,
                expected_inputs=len(batch),
                expected_active_tokens=None,
                expected_padded_tokens=None,
            )
            vectors.extend(response_vectors)
            signatures.append(telemetry)
            request_id += 1
        process.stdin.close()
        if process.wait(timeout=5) != 0:
            raise WitnessError("resident_exit_failed")
        return QualityResult(tuple(vectors), tuple(token_buckets), tuple(signatures))
    except (OSError, subprocess.SubprocessError):
        raise WitnessError("resident_io_failed") from None
    finally:
        stop_process(process)
        stderr.close()


def exact_vectors_equal(control: LaunchResult, candidate: LaunchResult) -> bool:
    return control.vectors == candidate.vectors


def cosine_similarity(left: tuple[float, ...], right: tuple[float, ...]) -> float:
    if len(left) != len(right) or not left:
        raise WitnessError("vector_shape_mismatch")
    dot = sum(base * head for base, head in zip(left, right))
    left_norm = math.sqrt(sum(value * value for value in left))
    right_norm = math.sqrt(sum(value * value for value in right))
    if left_norm <= 0.0 or right_norm <= 0.0:
        raise WitnessError("vector_norm_invalid")
    return dot / (left_norm * right_norm)


def ranked_passages(
    query: tuple[float, ...], passages: tuple[tuple[float, ...], ...]
) -> tuple[int, ...]:
    scores = [cosine_similarity(query, passage) for passage in passages]
    return tuple(sorted(range(len(scores)), key=lambda index: (-scores[index], index)))


def control_referenced_ndcg(
    control: tuple[int, ...], candidate: tuple[int, ...], top_k: int
) -> float:
    relevance = {
        passage: top_k - rank for rank, passage in enumerate(control[:top_k])
    }

    def dcg(order: tuple[int, ...]) -> float:
        return sum(
            relevance.get(passage, 0) / math.log2(rank + 2)
            for rank, passage in enumerate(order[:top_k])
        )

    ideal = dcg(control)
    if ideal <= 0.0:
        raise WitnessError("ranking_reference_invalid")
    return dcg(candidate) / ideal


def quality_summary(
    control: QualityResult, candidate: QualityResult
) -> dict[str, object]:
    if (
        len(control.vectors) != QUALITY_QUERY_COUNT + QUALITY_PASSAGE_COUNT
        or len(candidate.vectors) != len(control.vectors)
        or control.token_buckets != candidate.token_buckets
        or control.telemetry_signatures != candidate.telemetry_signatures
    ):
        raise WitnessError("quality_contract_mismatch")
    cosine_values: list[float] = []
    absolute_deltas: list[float] = []
    exact_vectors = 0
    for base, head in zip(control.vectors, candidate.vectors):
        if base == head:
            exact_vectors += 1
        cosine_values.append(cosine_similarity(base, head))
        absolute_deltas.extend(abs(base_value - head_value) for base_value, head_value in zip(base, head))

    control_queries = control.vectors[:QUALITY_QUERY_COUNT]
    control_passages = control.vectors[QUALITY_QUERY_COUNT:]
    candidate_queries = candidate.vectors[:QUALITY_QUERY_COUNT]
    candidate_passages = candidate.vectors[QUALITY_QUERY_COUNT:]
    overlaps: list[float] = []
    ndcg_values: list[float] = []
    top_1_matches = 0
    for base_query, head_query in zip(control_queries, candidate_queries):
        base_ranking = ranked_passages(base_query, control_passages)
        head_ranking = ranked_passages(head_query, candidate_passages)
        base_top = set(base_ranking[:QUALITY_TOP_K])
        head_top = set(head_ranking[:QUALITY_TOP_K])
        overlaps.append(len(base_top & head_top) / QUALITY_TOP_K)
        ndcg_values.append(
            control_referenced_ndcg(base_ranking, head_ranking, QUALITY_TOP_K)
        )
        top_1_matches += int(base_ranking[0] == head_ranking[0])

    minimum_cosine = min(cosine_values)
    maximum_absolute_delta = max(absolute_deltas)
    mean_absolute_delta = sum(absolute_deltas) / len(absolute_deltas)
    aggregate_overlap = sum(overlaps) / len(overlaps)
    minimum_query_overlap = min(overlaps)
    top_1_agreement = top_1_matches / len(overlaps)
    mean_ndcg = sum(ndcg_values) / len(ndcg_values)
    ndcg_drop = 1.0 - mean_ndcg
    passed = (
        minimum_cosine >= MIN_VECTOR_COSINE
        and maximum_absolute_delta <= MAX_ABSOLUTE_DELTA
        and mean_absolute_delta <= MAX_MEAN_ABSOLUTE_DELTA
        and aggregate_overlap >= MIN_TOP_K_OVERLAP
        and minimum_query_overlap >= MIN_PER_QUERY_TOP_K_OVERLAP
        and top_1_agreement >= MIN_TOP_1_AGREEMENT
        and ndcg_drop <= MAX_NDCG_DROP
    )
    return {
        "query_count": QUALITY_QUERY_COUNT,
        "passage_count": QUALITY_PASSAGE_COUNT,
        "top_k": QUALITY_TOP_K,
        "observed_active_token_buckets": list(control.token_buckets),
        "exact_vector_count": exact_vectors,
        "vector_count": len(control.vectors),
        "minimum_cosine_similarity": minimum_cosine,
        "maximum_elementwise_absolute_delta": maximum_absolute_delta,
        "mean_elementwise_absolute_delta": mean_absolute_delta,
        "aggregate_top_k_overlap": aggregate_overlap,
        "minimum_per_query_top_k_overlap": minimum_query_overlap,
        "top_1_agreement": top_1_agreement,
        "control_referenced_ndcg_at_k": mean_ndcg,
        "control_referenced_ndcg_drop": ndcg_drop,
        "control_zero_result_queries": 0,
        "candidate_zero_result_queries": 0,
        "thresholds": {
            "minimum_cosine_similarity": MIN_VECTOR_COSINE,
            "maximum_elementwise_absolute_delta": MAX_ABSOLUTE_DELTA,
            "maximum_mean_elementwise_absolute_delta": MAX_MEAN_ABSOLUTE_DELTA,
            "minimum_aggregate_top_k_overlap": MIN_TOP_K_OVERLAP,
            "minimum_per_query_top_k_overlap": MIN_PER_QUERY_TOP_K_OVERLAP,
            "minimum_top_1_agreement": MIN_TOP_1_AGREEMENT,
            "maximum_control_referenced_ndcg_drop": MAX_NDCG_DROP,
        },
        "passed": passed,
    }


def median(values: list[float] | list[int]) -> float:
    if not values:
        raise WitnessError("empty_metric")
    return float(statistics.median(values))


def improvement_pct(control: float, candidate: float) -> float:
    if control <= 0 or candidate <= 0:
        raise WitnessError("invalid_metric")
    return (control - candidate) * 100.0 / control


def ready_gate(control: float, candidate: float) -> tuple[float, float, bool]:
    delta_ms = candidate - control
    delta_pct = delta_ms * 100.0 / control
    return (
        delta_ms,
        delta_pct,
        delta_ms <= MAX_READY_REGRESSION_MS
        and delta_pct <= MAX_READY_REGRESSION_PCT,
    )


def parse_footprint(output: str) -> tuple[int, int]:
    current = re.search(r"phys_footprint:\s+(\d+) B", output)
    peak = re.search(r"phys_footprint_peak:\s+(\d+) B", output)
    if current is None or peak is None:
        raise WitnessError("physical_footprint_unavailable")
    return int(current.group(1)), int(peak.group(1))


def rss_bytes(pid: int) -> int:
    try:
        output = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)],
            check=True,
            capture_output=True,
            text=True,
            timeout=5,
        ).stdout.strip()
        value = int(output) * 1_024
    except (OSError, ValueError, subprocess.SubprocessError):
        raise WitnessError("rss_unavailable") from None
    if value <= 0:
        raise WitnessError("rss_unavailable")
    return value


def physical_footprint(pid: int) -> tuple[int, int]:
    try:
        output = subprocess.run(
            ["footprint", "--pid", str(pid), "--format", "bytes", "--noCategories"],
            check=True,
            capture_output=True,
            text=True,
            timeout=20,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        raise WitnessError("physical_footprint_unavailable") from None
    return parse_footprint(output)


def resource_variant(
    variant: Variant, runtime_dir: Path, intra_threads: int, timeout: float
) -> ResourceResult:
    stderr = tempfile.TemporaryFile()
    try:
        process = subprocess.Popen(
            [str(variant.binary), "--resident"],
            env=environment(runtime_dir, intra_threads),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=stderr,
            start_new_session=True,
        )
    except OSError:
        stderr.close()
        raise WitnessError("resident_start_failed") from None
    rss_samples: list[int] = []
    footprint_peaks: list[int] = []
    try:
        if process.stdin is None or process.stdout is None:
            raise WitnessError("resident_pipe_unavailable")
        validate_ready(read_frame(process.stdout, timeout))
        request = {
            "schema_version": STREAM_SCHEMA,
            "request_id": 1,
            "model_id": MODEL_ID,
            "dimension": DIMENSION,
            "inputs": workload(),
        }
        write_frame(process.stdin, request)
        validate_result(read_frame(process.stdout, timeout), 1)
        warm_rss = rss_bytes(process.pid)
        for request_id in range(2, 10):
            request["request_id"] = request_id
            write_frame(process.stdin, request)
            validate_result(read_frame(process.stdout, timeout), request_id)
            rss_samples.append(rss_bytes(process.pid))
            _, peak = physical_footprint(process.pid)
            footprint_peaks.append(peak)
        return ResourceResult(
            warm_rss,
            max([warm_rss, *rss_samples]),
            max(footprint_peaks),
        )
    finally:
        stop_process(process)
        stderr.close()


def bootstrap_improvement_interval(
    control: list[float], candidate: list[float], seed: int, draws: int = 10_000
) -> tuple[float, float]:
    if len(control) != len(candidate) or not control:
        raise WitnessError("paired_samples_invalid")
    randomizer = random.Random(seed)
    gains: list[float] = []
    for _ in range(draws):
        indexes = [randomizer.randrange(len(control)) for _ in control]
        base = median([control[index] for index in indexes])
        head = median([candidate[index] for index in indexes])
        gains.append(improvement_pct(base, head))
    gains.sort()
    return gains[int(draws * 0.025)], gains[int(draws * 0.975) - 1]


def variant_summary(onnx: list[int], ready: list[float]) -> dict[str, object]:
    return {
        "launches": len(ready),
        "measured_requests": len(onnx),
        "onnx_us_median": median(onnx),
        "onnx_us_per_input_median": median(
            [value / EXPECTED_INPUTS for value in onnx]
        ),
        "onnx_us_per_active_token_median": median(
            [value / EXPECTED_ACTIVE_TOKENS for value in onnx]
        ),
        "ready_ms_median": median(ready),
    }


def resource_summary(value: ResourceResult) -> dict[str, int]:
    return {
        "warm_steady_rss_bytes": value.warm_rss_bytes,
        "lifecycle_peak_rss_bytes": value.peak_rss_bytes,
        "physical_footprint_peak_bytes": value.peak_physical_footprint_bytes,
    }


def build_report(
    control: Variant,
    candidate: Variant,
    seed: int,
    orders: tuple[str, ...],
    controls: list[LaunchResult],
    candidates: list[LaunchResult],
    quality: dict[str, object],
    resources: dict[str, ResourceResult],
) -> dict[str, object]:
    if len(controls) != PAIR_COUNT or len(candidates) != PAIR_COUNT:
        raise WitnessError("paired_sample_count_invalid")
    performance_vectors_equal = all(
        exact_vectors_equal(base, head)
        for base, head in zip(controls, candidates)
    )
    control_onnx = [value for item in controls for value in item.onnx_us]
    candidate_onnx = [value for item in candidates for value in item.onnx_us]
    control_summary = variant_summary(control_onnx, [item.ready_ms for item in controls])
    candidate_summary = variant_summary(
        candidate_onnx, [item.ready_ms for item in candidates]
    )
    improvements = {
        metric: improvement_pct(
            float(control_summary[metric]), float(candidate_summary[metric])
        )
        for metric in (
            "onnx_us_median",
            "onnx_us_per_input_median",
            "onnx_us_per_active_token_median",
        )
    }
    ready_ms, ready_pct, startup_pass = ready_gate(
        float(control_summary["ready_ms_median"]),
        float(candidate_summary["ready_ms_median"]),
    )
    h0_pass = (
        resources["candidate_h0"].peak_physical_footprint_bytes
        <= H0_FOOTPRINT_LIMIT_BYTES
    )
    performance_pass = all(
        value >= MIN_ONNX_IMPROVEMENT_PCT for value in improvements.values()
    )
    paired_control = [median(list(item.onnx_us)) for item in controls]
    paired_candidate = [median(list(item.onnx_us)) for item in candidates]
    confidence_interval = bootstrap_improvement_interval(
        paired_control, paired_candidate, seed
    )
    confidence_pass = confidence_interval[0] > 0.0
    h2_pass = (
        resources["candidate_h2"].peak_physical_footprint_bytes
        <= H2_MEMORY_LIMIT_BYTES
    )
    quality_pass = quality.get("passed") is True
    accepted = (
        quality_pass
        and startup_pass
        and h0_pass
        and h2_pass
        and performance_pass
        and confidence_pass
    )
    return {
        "schema_version": REPORT_SCHEMA,
        "issue": 290,
        "source": "public_synthetic_workload",
        "claim": "resident_gate_passed" if accepted else "resident_gate_failed",
        "seed": seed,
        "paired_blocks": len(orders),
        "sequence": {
            "ab_count": orders.count("AB"),
            "ba_count": orders.count("BA"),
        },
        "workload": {
            "batch_size": EXPECTED_INPUTS,
            "active_tokens_per_input": EXPECTED_ACTIVE_TOKENS // EXPECTED_INPUTS,
            "active_token_count": EXPECTED_ACTIVE_TOKENS,
            "padded_token_count": EXPECTED_ACTIVE_TOKENS,
            "excluded_warmups_per_launch": 1,
            "measured_requests_per_launch": MEASURED_REQUESTS,
            "intra_threads": 3,
        },
        "variants": {
            "control": {"revision": control.revision, **control_summary},
            "candidate": {"revision": candidate.revision, **candidate_summary},
        },
        "resources": {
            name: resource_summary(value) for name, value in resources.items()
        },
        "control_referenced_stability": quality,
        "gates": {
            "performance_vectors_elementwise_equal_informational": performance_vectors_equal,
            "onnx_improvement_pct": improvements,
            "minimum_onnx_improvement_pct": MIN_ONNX_IMPROVEMENT_PCT,
            "paired_onnx_improvement_bootstrap_95pct": list(confidence_interval),
            "paired_onnx_confidence_pass": confidence_pass,
            "startup_ready_regression_ms": ready_ms,
            "startup_ready_regression_pct": ready_pct,
            "startup_pass": startup_pass,
            "h0_footprint_limit_bytes": H0_FOOTPRINT_LIMIT_BYTES,
            "h0_footprint_pass": h0_pass,
            "h2_memory_limit_bytes": H2_MEMORY_LIMIT_BYTES,
            "h2_memory_pass": h2_pass,
            "control_referenced_stability_pass": quality_pass,
            "resident_performance_pass": performance_pass,
            "accepted": accepted,
        },
        "privacy": PRIVACY,
    }


def run_experiment(args: argparse.Namespace) -> dict[str, object]:
    control = Variant("control", args.control_bin, args.control_revision)
    candidate = Variant("candidate", args.candidate_bin, args.candidate_revision)
    quality = quality_summary(
        quality_variant(control, args.runtime_dir, args.timeout_seconds),
        quality_variant(candidate, args.runtime_dir, args.timeout_seconds),
    )
    orders = balanced_orders(args.seed)
    results: dict[str, list[LaunchResult]] = {"control": [], "candidate": []}
    variants = {"A": control, "B": candidate}
    for order in orders:
        pair: dict[str, LaunchResult] = {}
        for key in order:
            variant = variants[key]
            pair[variant.label] = launch_variant(
                variant, args.runtime_dir, 3, args.timeout_seconds
            )
        results["control"].append(pair["control"])
        results["candidate"].append(pair["candidate"])
    resources = {
        "control_h2": resource_variant(control, args.runtime_dir, 3, args.timeout_seconds),
        "candidate_h2": resource_variant(
            candidate, args.runtime_dir, 3, args.timeout_seconds
        ),
        "candidate_h0": resource_variant(
            candidate, args.runtime_dir, 1, args.timeout_seconds
        ),
    }
    return build_report(
        control,
        candidate,
        args.seed,
        orders,
        results["control"],
        results["candidate"],
        quality,
        resources,
    )


def encode_report(report: dict[str, object], private_markers: tuple[str, ...]) -> bytes:
    encoded = (json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    if len(encoded) > MAX_REPORT_BYTES:
        raise WitnessError("report_too_large")
    if any(marker and marker.encode() in encoded for marker in private_markers):
        raise WitnessError("report_privacy_boundary_failed")
    return encoded


class SelfTests(unittest.TestCase):
    def test_sequence_is_balanced_and_reproducible(self) -> None:
        first = balanced_orders(290)
        self.assertEqual(first, balanced_orders(290))
        self.assertEqual(first.count("AB"), 12)
        self.assertEqual(first.count("BA"), 12)

    def test_workload_is_bounded_batch_four(self) -> None:
        inputs = workload()
        self.assertEqual(len(inputs), EXPECTED_INPUTS)
        self.assertTrue(all(len(item["text"].encode()) < 65_536 for item in inputs))

    def test_quality_workload_covers_roles_and_five_length_tiers(self) -> None:
        inputs = quality_workload()
        self.assertEqual(len(inputs), QUALITY_QUERY_COUNT + QUALITY_PASSAGE_COUNT)
        self.assertEqual(
            sum(item["role"] == "query" for item in inputs), QUALITY_QUERY_COUNT
        )
        self.assertEqual(
            sum(item["role"] == "passage" for item in inputs),
            QUALITY_PASSAGE_COUNT,
        )
        self.assertEqual(len({len(item["text"]) for item in inputs[:5]}), 5)

    def test_normalized_metrics_and_startup_gate(self) -> None:
        summary = variant_summary([100, 120, 80], [900.0, 1_000.0, 1_100.0])
        self.assertEqual(summary["onnx_us_median"], 100.0)
        self.assertEqual(summary["onnx_us_per_input_median"], 25.0)
        self.assertEqual(summary["onnx_us_per_active_token_median"], 100 / 2_048)
        self.assertTrue(ready_gate(1_000.0, 1_100.0)[2])
        self.assertFalse(ready_gate(1_000.0, 1_100.1)[2])

    def test_vector_equality_is_elementwise_exact(self) -> None:
        base = LaunchResult(1.0, (1,), ((((1.0, 2.0),)),))
        same = LaunchResult(1.0, (1,), ((((1.0, 2.0),)),))
        changed = LaunchResult(1.0, (1,), ((((1.0, 2.0000001),)),))
        self.assertTrue(exact_vectors_equal(base, same))
        self.assertFalse(exact_vectors_equal(base, changed))

    def test_result_requires_finite_normalized_vectors(self) -> None:
        vector = [1.0, *([0.0] * (DIMENSION - 1))]
        response = {
            "type": "result",
            "schema_version": STREAM_SCHEMA,
            "request_id": 1,
            "vectors": [vector],
            "telemetry": {
                "input_count": 1,
                "active_token_count": 8,
                "padded_token_count": 8,
                "onnx_us": 1,
            },
        }
        validate_result(
            response,
            1,
            expected_inputs=1,
            expected_active_tokens=8,
            expected_padded_tokens=8,
        )
        response["vectors"] = [[2.0, *([0.0] * (DIMENSION - 1))]]
        with self.assertRaisesRegex(
            WitnessError, "vector_normalization_mismatch"
        ):
            validate_result(
                response,
                1,
                expected_inputs=1,
                expected_active_tokens=8,
                expected_padded_tokens=8,
            )

    def test_quality_summary_accepts_identical_control_rankings(self) -> None:
        vectors = tuple(
            (math.cos(index), math.sin(index))
            for index in range(QUALITY_QUERY_COUNT + QUALITY_PASSAGE_COUNT)
        )
        value = QualityResult(
            vectors,
            (8, 32, 96, 256, 512),
            tuple((4, 100 + index, 400) for index in range(15)),
        )
        summary = quality_summary(value, value)
        self.assertTrue(summary["passed"])
        self.assertEqual(summary["aggregate_top_k_overlap"], 1.0)
        self.assertEqual(summary["control_referenced_ndcg_drop"], 0.0)

    def test_quality_summary_rejects_material_vector_drift(self) -> None:
        vectors = tuple(
            (math.cos(index), math.sin(index))
            for index in range(QUALITY_QUERY_COUNT + QUALITY_PASSAGE_COUNT)
        )
        changed = list(vectors)
        changed[0] = (-changed[0][0], changed[0][1])
        telemetry = tuple((4, 100 + index, 400) for index in range(15))
        control = QualityResult(vectors, (8, 32, 96, 256, 512), telemetry)
        candidate = QualityResult(
            tuple(changed), (8, 32, 96, 256, 512), telemetry
        )
        self.assertFalse(quality_summary(control, candidate)["passed"])

    def test_footprint_parser_and_limit(self) -> None:
        current, peak = parse_footprint(
            "phys_footprint: 100 B\nphys_footprint_peak: 200 B\n"
        )
        self.assertEqual((current, peak), (100, 200))
        self.assertLess(peak, H0_FOOTPRINT_LIMIT_BYTES)

    def test_bootstrap_paired_interval_is_reproducible(self) -> None:
        control = [100.0, 102.0, 104.0, 106.0, 108.0]
        candidate = [90.0, 91.0, 92.0, 93.0, 94.0]
        first = bootstrap_improvement_interval(control, candidate, 290, draws=1_000)
        self.assertEqual(
            first,
            bootstrap_improvement_interval(control, candidate, 290, draws=1_000),
        )
        self.assertGreater(first[0], 0.0)

    def test_report_is_bounded_and_privacy_fail_closed(self) -> None:
        report = {"schema_version": REPORT_SCHEMA, "privacy": PRIVACY}
        self.assertLess(len(encode_report(report, ("/private/root",))), MAX_REPORT_BYTES)
        with self.assertRaisesRegex(WitnessError, "report_privacy_boundary_failed"):
            encode_report({"value": "/private/root"}, ("/private/root",))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--control-bin", type=Path)
    parser.add_argument("--candidate-bin", type=Path)
    parser.add_argument("--control-revision")
    parser.add_argument("--candidate-revision")
    parser.add_argument("--runtime-dir", type=Path)
    parser.add_argument("--seed", type=int, default=290)
    parser.add_argument("--timeout-seconds", type=float, default=60.0)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    if args.self_test:
        return args
    required = (
        "control_bin",
        "candidate_bin",
        "control_revision",
        "candidate_revision",
        "runtime_dir",
        "out",
    )
    if any(getattr(args, name) is None for name in required):
        parser.error("control, candidate, runtime, revision, and output arguments are required")
    if args.timeout_seconds <= 0:
        parser.error("timeout-seconds must be positive")
    for revision in (args.control_revision, args.candidate_revision):
        if not isinstance(revision, str) or re.fullmatch(r"[0-9a-f]{40}", revision) is None:
            parser.error("revisions must be exact lowercase 40-character Git SHAs")
    for name in ("control_bin", "candidate_bin", "runtime_dir"):
        try:
            setattr(args, name, getattr(args, name).resolve(strict=True))
        except OSError:
            parser.error(f"{name.replace('_', '-')} must resolve")
    args.out = args.out.resolve(strict=False)
    return args


def main() -> int:
    args = parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(SelfTests)
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    try:
        report = run_experiment(args)
        private_markers = (
            str(args.control_bin),
            str(args.candidate_bin),
            str(args.runtime_dir),
            str(args.out),
        )
        encoded = encode_report(report, private_markers)
        args.out.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        args.out.write_bytes(encoded)
        args.out.chmod(0o600)
        print(encoded.decode(), end="")
        return 0 if report["gates"]["accepted"] else 2  # type: ignore[index]
    except WitnessError as error:
        print(json.dumps({"schema_version": REPORT_SCHEMA, "error": str(error)}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
