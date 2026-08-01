#!/usr/bin/env python3
"""Capture bounded public-synthetic resident ONNX operator attribution."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import runpy
import statistics
import subprocess
import tempfile
import time
import unittest
from collections import Counter, defaultdict
from pathlib import Path

STREAM_SCHEMA = "resume-ir.embedding-stream.v1"
REPORT_SCHEMA = "resume-ir.onnx-operator-profile.v1"
MODEL_ID = "intfloat-multilingual-e5-small-qint8-r1"
DIMENSION = 384
MAX_REPORT_BYTES = 64 * 1024
MAX_TRACE_BYTES = 256 * 1024 * 1024
PRIMARY_TOKEN_COUNT = 512
TOKEN_BUCKETS = (8, 32, 96, 256, 512)
FAMILY_THRESHOLD = 0.20
PRIMARY_CAPTURES = 5
MEASURED_REQUESTS = 20
WARMUP_SECONDS = 30.0
CROSS_CHECK_SECONDS = 20
TIMEOUT_SECONDS = 90.0
PRIVACY_FIELDS = ("raw_resume_text", "raw_query", "candidate_results", "local_paths", "vectors",
                  "token_content", "runtime_or_model_bytes", "pids", "raw_node_names",
                  "raw_symbols", "raw_profiler_data")
PRIVACY = {f"contains_{name}": False for name in PRIVACY_FIELDS}
FRAMING = runpy.run_path(str(Path(__file__).with_name("embedding-batch-benchmark.py")))
class WitnessError(RuntimeError): pass
def read_frame(stream: object, timeout: float) -> dict[str, object]:
    try:
        return FRAMING["read_frame"](stream, timeout)  # type: ignore[operator]
    except RuntimeError:
        raise WitnessError("resident_payload_invalid") from None

def write_frame(stream: object, value: dict[str, object]) -> None:
    try:
        FRAMING["write_frame"](stream, value)  # type: ignore[operator]
    except RuntimeError:
        raise WitnessError("request_frame_invalid") from None

def environment(runtime_dir: Path, profile_prefix: Path | None = None) -> dict[str, str]:
    result = os.environ.copy()
    result.update(
        {
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(runtime_dir),
            "RESUME_IR_EMBEDDING_MODEL_ID": MODEL_ID,
            "RESUME_IR_EMBEDDING_DIMENSION": str(DIMENSION),
            "RESUME_IR_EMBEDDING_INTRA_THREADS": "3",
        }
    )
    if profile_prefix is not None:
        result["RESUME_IR_EMBEDDING_PROFILE_OUTPUT_PREFIX"] = str(profile_prefix)
    return result

class Resident:
    def __init__(
        self, binary: Path, runtime_dir: Path, timeout: float, profile_prefix: Path | None = None
    ) -> None:
        self.timeout = timeout
        mode = "--resident-profile" if profile_prefix is not None else "--resident"
        try:
            self.stderr = tempfile.TemporaryFile()
            self.process = subprocess.Popen(
                [str(binary), mode],
                env=environment(runtime_dir, profile_prefix),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=self.stderr,
            )
        except OSError:
            raise WitnessError("resident_start_failed") from None
        if self.process.stdin is None or self.process.stdout is None:
            self.stop()
            raise WitnessError("resident_pipe_unavailable")
        if read_frame(self.process.stdout, timeout) != {
            "type": "ready",
            "schema_version": STREAM_SCHEMA,
            "model_id": MODEL_ID,
            "dimension": DIMENSION,
        }:
            self.stop()
            raise WitnessError("ready_identity_mismatch")
        self.request_id = 0

    def request(self, inputs: list[dict[str, str]]) -> tuple[int, tuple[tuple[float, ...], ...], int]:
        self.request_id += 1
        write_frame(
            self.process.stdin,
            {
                "schema_version": STREAM_SCHEMA,
                "request_id": self.request_id,
                "model_id": MODEL_ID,
                "dimension": DIMENSION,
                "inputs": inputs,
            },
        )
        response = read_frame(self.process.stdout, self.timeout)
        vectors, telemetry = response.get("vectors"), response.get("telemetry")
        if (
            response.get("type") != "result"
            or response.get("schema_version") != STREAM_SCHEMA
            or response.get("request_id") != self.request_id
            or not isinstance(vectors, list)
            or len(vectors) != len(inputs)
            or not isinstance(telemetry, dict)
        ):
            raise WitnessError("result_contract_mismatch")
        checked: list[tuple[float, ...]] = []
        for vector in vectors:
            if (
                not isinstance(vector, list)
                or len(vector) != DIMENSION
                or any(not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value) for value in vector)
            ):
                raise WitnessError("vector_contract_mismatch")
            normalized = tuple(float(value) for value in vector)
            if abs(math.sqrt(sum(value * value for value in normalized)) - 1.0) > 1e-4:
                raise WitnessError("vector_normalization_mismatch")
            checked.append(normalized)
        active = telemetry.get("active_token_count")
        padded = telemetry.get("padded_token_count")
        onnx_us = telemetry.get("onnx_us")
        if (
            telemetry.get("input_count") != len(inputs)
            or not isinstance(active, int)
            or isinstance(active, bool)
            or not isinstance(padded, int)
            or isinstance(padded, bool)
            or padded < active
            or not isinstance(onnx_us, int)
            or isinstance(onnx_us, bool)
            or onnx_us <= 0
        ):
            raise WitnessError("telemetry_contract_mismatch")
        return active, tuple(checked), onnx_us

    def close(self) -> None:
        self.process.stdin.close()
        try:
            if self.process.wait(timeout=20) != 0:
                raise WitnessError("resident_exit_failed")
        except subprocess.TimeoutExpired:
            raise WitnessError("resident_exit_timeout") from None

    def stop(self) -> None:
        if getattr(self, "process", None) is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        if getattr(self, "stderr", None) is not None:
            self.stderr.close()
            self.stderr = None

def repeated_inputs(text: str) -> list[dict[str, str]]:
    return [{"role": "passage", "text": text} for _ in range(4)]

def calibrate_workloads(binary: Path, runtime_dir: Path, timeout: float) -> dict[int, list[dict[str, str]]]:
    resident = Resident(binary, runtime_dir, timeout)
    result: dict[int, list[dict[str, str]]] = {}
    try:
        for target in TOKEN_BUCKETS:
            low, high, found = 1, 1024, None
            while low <= high:
                count = (low + high) // 2
                inputs = repeated_inputs(" ".join(["alpha"] * count))
                active, _, _ = resident.request(inputs)
                per_input = active // 4
                if active % 4:
                    raise WitnessError("token_calibration_unbalanced")
                if per_input == target:
                    found = inputs
                    break
                if per_input < target:
                    low = count + 1
                else:
                    high = count - 1
            if found is None:
                raise WitnessError("token_calibration_failed")
            result[target] = found
        resident.close()
        return result
    finally:
        resident.stop()

def warm_and_measure(resident: Resident, inputs: list[dict[str, str]], tokens: int, warmup: float, measured: int) -> tuple[tuple[tuple[float, ...], ...], list[int]]:
    deadline = time.monotonic() + warmup
    while time.monotonic() < deadline:
        resident.request(inputs)
        time.sleep(min(1.0, max(0.0, deadline - time.monotonic())))
    vectors = None
    samples: list[int] = []
    for _ in range(measured):
        active, current, onnx_us = resident.request(inputs)
        if active != len(inputs) * tokens:
            raise WitnessError("measured_token_contract_invalid")
        vectors = current
        samples.append(onnx_us)
    if vectors is None:
        raise WitnessError("measured_request_missing")
    return vectors, samples

def operator_family(name: str) -> str:
    if name in {"MatMul", "MatMulInteger", "QLinearMatMul", "Gemm"}:
        return "matrix"
    if name in {"DynamicQuantizeLinear", "DynamicQuantizeMatMul", "MatMulIntegerToFloat", "QuantizeLinear", "DequantizeLinear"}:
        return "dynamic_quantization"
    if name in {"Attention", "MultiHeadAttention", "Softmax"}:
        return "attention"
    if name in {"LayerNormalization", "SkipLayerNormalization", "SimplifiedLayerNormalization"}:
        return "normalization"
    if name in {"Cast", "Concat", "Expand", "Flatten", "Gather", "Reshape", "Shape", "Squeeze", "Transpose", "Unsqueeze"}:
        return "shape_data_movement"
    return "other"

def finite_number(value: object) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value) or value < 0:
        raise WitnessError("trace_numeric_field_invalid")
    return float(value)

def parse_trace(events: object, measured: int) -> dict[str, object]:
    if not isinstance(events, list):
        raise WitnessError("trace_root_invalid")
    model_runs: list[tuple[float, float]] = []
    nodes: list[tuple[float, float, str, str]] = []
    for event in events:
        if not isinstance(event, dict):
            raise WitnessError("trace_event_invalid")
        category, name = event.get("cat"), event.get("name")
        if category not in {"Session", "Node"}:
            continue
        start, duration = finite_number(event.get("ts")), finite_number(event.get("dur"))
        if category == "Session" and name == "model_run":
            model_runs.append((start, duration))
        elif category == "Node":
            args = event.get("args")
            if not isinstance(args, dict):
                raise WitnessError("trace_node_args_invalid")
            operation, provider = args.get("op_name"), args.get("provider")
            if (
                not isinstance(operation, str)
                or re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", operation) is None
                or not isinstance(provider, str)
                or re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", provider) is None
            ):
                raise WitnessError("trace_node_identity_invalid")
            nodes.append((start, duration, operation, provider))
    model_runs.sort()
    if len(model_runs) < measured:
        raise WitnessError("trace_measured_runs_missing")
    windows = model_runs[-measured:]
    selected = [node for node in nodes if any(node[0] >= start and node[0] + node[1] <= start + duration for start, duration in windows)]
    if not selected:
        raise WitnessError("trace_measured_nodes_missing")
    by_operator: dict[tuple[str, str], list[float | int]] = defaultdict(lambda: [0.0, 0])
    by_family: dict[str, float] = defaultdict(float)
    for _, duration, operation, provider in selected:
        by_operator[(operation, provider)][0] += duration
        by_operator[(operation, provider)][1] += 1
        by_family[operator_family(operation)] += duration
    node_duration = sum(float(value[0]) for value in by_operator.values())
    model_duration = sum(duration for _, duration in windows)
    if node_duration <= 0 or model_duration <= 0:
        raise WitnessError("trace_duration_invalid")
    top_operators = sorted(by_operator.items(), key=lambda item: (-float(item[1][0]), item[0]))[:20]
    families = sorted(by_family.items(), key=lambda item: (-item[1], item[0]))
    return {
        "measured_requests": measured,
        "model_run_duration_us": model_duration,
        "node_duration_us": node_duration,
        "non_node_residual_us": max(0.0, model_duration - node_duration),
        "top_operators": [
            {"operator_type": key[0], "provider": key[1], "duration_us": value[0], "calls": value[1], "node_share": float(value[0]) / node_duration} for key, value in top_operators
        ],
        "families": [
            {"family": family, "duration_us": duration, "node_share": duration / node_duration} for family, duration in families
        ],
    }

def read_trace(path: Path, measured: int) -> dict[str, object]:
    try:
        if path.stat().st_size > MAX_TRACE_BYTES:
            raise WitnessError("trace_size_exceeded")
        return parse_trace(json.loads(path.read_bytes()), measured)
    except (OSError, json.JSONDecodeError):
        raise WitnessError("trace_read_failed") from None

def profile_capture(binary: Path, runtime_dir: Path, inputs: list[dict[str, str]], tokens: int, timeout: float, warmup: float, measured: int) -> tuple[dict[str, object], tuple[tuple[float, ...], ...], list[int]]:
    with tempfile.TemporaryDirectory(prefix="resume-ir-operator-profile-") as raw:
        root = Path(raw)
        prefix = root / "operator-profile"
        resident = Resident(binary, runtime_dir, timeout, prefix)
        try:
            vectors, samples = warm_and_measure(resident, inputs, tokens, warmup, measured)
            resident.close()
            traces = list(root.glob("operator-profile*.json"))
            if len(traces) != 1:
                raise WitnessError("profile_output_count_invalid")
            return read_trace(traces[0], measured), vectors, samples
        finally:
            resident.stop()

def normal_control(binary: Path, runtime_dir: Path, inputs: list[dict[str, str]], tokens: int, timeout: float, warmup: float, measured: int) -> tuple[tuple[tuple[float, ...], ...], list[int]]:
    resident = Resident(binary, runtime_dir, timeout)
    try:
        result = warm_and_measure(resident, inputs, tokens, warmup, measured)
        resident.close()
        return result
    finally:
        resident.stop()

def family_shares(capture: dict[str, object]) -> dict[str, float]:
    return {str(item["family"]): float(item["node_share"]) for item in capture["families"]}  # type: ignore[index]

def decide(primary: list[dict[str, object]], sensitivity: dict[int, dict[str, object]], cross_check: dict[str, object]) -> dict[str, object]:
    top = [str(capture["families"][0]["family"]) for capture in primary]  # type: ignore[index]
    family, count = Counter(top).most_common(1)[0]
    median_share = statistics.median(family_shares(capture).get(family, 0.0) for capture in primary)
    bucket_top_two = all(family in [str(item["family"]) for item in capture["families"][:2]] for capture in sensitivity.values())  # type: ignore[index]
    accepted = count >= 4 and median_share >= FAMILY_THRESHOLD and bucket_top_two and cross_check.get("conflicts") is False
    directions = {
        "matrix": "same_model_ort_version_ab",
        "dynamic_quantization": "model_artifact_quantization_matrix",
        "attention": "offline_transformer_optimizer",
        "normalization": "offline_transformer_optimizer",
        "shape_data_movement": "graph_internal_pooling_normalization",
        "other": "no_automatic_follow_up",
    }
    residual_share = statistics.median(float(capture["non_node_residual_us"]) / float(capture["model_run_duration_us"]) for capture in primary)
    cross_family = cross_check.get("symbol_family")
    residual_direction = {"allocator": "cpu_arena_ab", "scheduling": "intra_op_thread_sweep"}.get(cross_family)
    if not accepted and residual_share >= FAMILY_THRESHOLD and residual_direction:
        return {
            "outcome": "bottleneck_selected", "family": f"{cross_family}_residual",
            "first_rank_captures": 0, "median_node_share": median_share,
            "median_non_node_residual_share": residual_share,
            "top_two_in_all_token_buckets": bucket_top_two, "follow_up_direction": residual_direction,
        }
    return {
        "outcome": "bottleneck_selected" if accepted else "inconclusive", "family": family,
        "first_rank_captures": count, "median_node_share": median_share,
        "median_non_node_residual_share": residual_share,
        "top_two_in_all_token_buckets": bucket_top_two, "follow_up_direction": directions[family] if accepted else None,
    }

def symbol_family(text: str) -> str | None:
    lowered = text.lower()
    counts = {
        "matrix": len(re.findall(r"mlas|matmul|gemm", lowered)),
        "allocator": len(re.findall(r"malloc|calloc|realloc|operator new", lowered)),
        "scheduling": len(re.findall(r"pthread|workqueue|threadpool|scheduler", lowered)),
        "onnx_runtime": len(re.findall(r"onnxruntime|sequentialexecutor|execute", lowered)),
    }
    family, count = max(counts.items(), key=lambda item: item[1])
    return family if count else None

def drive_sampler(resident: Resident, inputs: list[dict[str, str]], sampler: subprocess.Popen[bytes], timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while sampler.poll() is None and time.monotonic() < deadline:
        resident.request(inputs)
    if sampler.poll() is None:
        sampler.terminate()
        sampler.wait(timeout=5)

def cross_check(binary: Path, runtime_dir: Path, inputs: list[dict[str, str]], timeout: float, seconds: int, ort_family: str) -> dict[str, object]:
    resident = Resident(binary, runtime_dir, timeout)
    try:
        resident.request(inputs)
        with tempfile.TemporaryDirectory(prefix="resume-ir-time-profile-") as raw:
            root = Path(raw)
            trace = root / "time-profile.trace"
            sampler = subprocess.Popen(
                ["xcrun", "xctrace", "record", "--template", "Time Profiler", "--attach", str(resident.process.pid), "--time-limit", f"{seconds}s", "--output", str(trace), "--no-prompt"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            drive_sampler(resident, inputs, sampler, seconds + 15)
            family = None
            method = "xctrace_time_profiler"
            if sampler.returncode == 0:
                exported = subprocess.run(
                    ["xcrun", "xctrace", "export", "--input", str(trace), "--xpath", '/trace-toc/run[@number="1"]/data/table'],
                    capture_output=True,
                    timeout=30,
                    check=False,
                )
                if exported.returncode == 0:
                    family = symbol_family(exported.stdout.decode(errors="ignore"))
            if family is None:
                method = "sample_fallback"
                sample_output = root / "sample.txt"
                fallback = subprocess.Popen(
                    ["sample", str(resident.process.pid), str(seconds), "1", "-file", str(sample_output)],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                drive_sampler(resident, inputs, fallback, seconds + 15)
                if fallback.returncode == 0 and sample_output.exists():
                    family = symbol_family(sample_output.read_text(errors="ignore"))
            resident.close()
            if family is None:
                return {"method": method, "symbol_family": "unavailable", "conflicts": True}
            conflicts = family in {"allocator", "scheduling"} and family != ort_family
            return {"method": method, "symbol_family": family, "conflicts": conflicts}
    finally:
        resident.stop()

def encode_report(report: dict[str, object], private_markers: tuple[str, ...]) -> bytes:
    encoded = (json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    if len(encoded) > MAX_REPORT_BYTES:
        raise WitnessError("report_size_exceeded")
    if any(marker and marker.encode() in encoded for marker in private_markers):
        raise WitnessError("report_privacy_boundary_failed")
    return encoded

def run_experiment(args: argparse.Namespace) -> dict[str, object]:
    workloads = calibrate_workloads(args.binary, args.runtime_dir, TIMEOUT_SECONDS)
    control_vectors, control_times = normal_control(args.binary, args.runtime_dir, workloads[PRIMARY_TOKEN_COUNT], PRIMARY_TOKEN_COUNT, TIMEOUT_SECONDS, WARMUP_SECONDS, MEASURED_REQUESTS)
    primary: list[dict[str, object]] = []
    profile_times: list[int] = []
    for _ in range(PRIMARY_CAPTURES):
        capture, vectors, samples = profile_capture(args.binary, args.runtime_dir, workloads[PRIMARY_TOKEN_COUNT], PRIMARY_TOKEN_COUNT, TIMEOUT_SECONDS, WARMUP_SECONDS, MEASURED_REQUESTS)
        if vectors != control_vectors:
            raise WitnessError("profile_vector_parity_failed")
        primary.append(capture)
        profile_times.extend(samples)
    sensitivity: dict[int, dict[str, object]] = {PRIMARY_TOKEN_COUNT: primary[0]}
    for bucket in TOKEN_BUCKETS[:-1]:
        capture, _, _ = profile_capture(args.binary, args.runtime_dir, workloads[bucket], bucket, TIMEOUT_SECONDS, WARMUP_SECONDS, MEASURED_REQUESTS)
        sensitivity[bucket] = capture
    provisional_family = Counter(str(item["families"][0]["family"]) for item in primary).most_common(1)[0][0]  # type: ignore[index]
    system_cross_check = cross_check(args.binary, args.runtime_dir, workloads[PRIMARY_TOKEN_COUNT], TIMEOUT_SECONDS, CROSS_CHECK_SECONDS, provisional_family)
    decision = decide(primary, sensitivity, system_cross_check)
    overhead = (statistics.median(profile_times) - statistics.median(control_times)) * 100.0 / statistics.median(control_times)
    return {
        "schema_version": REPORT_SCHEMA,
        "issue": 293,
        "source": "public_synthetic_resident_embedding",
        "claim": "hotspot_attribution_only",
        "revision": args.revision,
        "workload": {"batch_size": 4, "primary_active_tokens_per_input": PRIMARY_TOKEN_COUNT,
                     "token_buckets": list(TOKEN_BUCKETS), "intra_threads": 3,
                     "warmup_seconds": WARMUP_SECONDS, "measured_requests_per_capture": MEASURED_REQUESTS,
                     "primary_captures": PRIMARY_CAPTURES},
        "primary_captures": primary,
        "sensitivity": {str(key): value for key, value in sorted(sensitivity.items())},
        "exact_profiled_unprofiled_vectors": True,
        "profiler_overhead_pct_informational": overhead,
        "profiler_overhead_allows_performance_claim": overhead <= 3.0,
        "system_cross_check": system_cross_check,
        "decision": decision,
        "privacy": PRIVACY,
    }

class SelfTests(unittest.TestCase):
    def sample_trace(self, operators: list[tuple[str, float]]) -> list[dict[str, object]]:
        events: list[dict[str, object]] = [{"cat": "Session", "name": "model_run", "ts": 0, "dur": 100}]
        start = 1.0
        for operation, duration in operators:
            events.append({"cat": "Node", "name": "private-node-name", "ts": start, "dur": duration, "args": {"op_name": operation, "provider": "CPUExecutionProvider"}})
            start += duration
        return events

    def test_trace_aggregates_operators_families_and_residual(self) -> None:
        report = parse_trace(self.sample_trace([("DynamicQuantizeMatMul", 60), ("MatMulInteger", 20)]), 1)
        self.assertEqual((report["measured_requests"], report["non_node_residual_us"]), (1, 20.0))
        self.assertEqual(report["families"][0]["family"], "dynamic_quantization")  # type: ignore[index]
        self.assertNotIn("private-node-name", json.dumps(report))

    def test_trace_excludes_warmup_windows(self) -> None:
        events = self.sample_trace([("Gemm", 80)])
        events.extend({"cat": "Session", "name": "model_run", "ts": 200, "dur": 100} for _ in range(1))
        events.append({"cat": "Node", "name": "second", "ts": 201, "dur": 90, "args": {"op_name": "Softmax", "provider": "CPUExecutionProvider"}})
        self.assertEqual(parse_trace(events, 1)["families"][0]["family"], "attention")  # type: ignore[index]

    def test_trace_rejects_invalid_numeric_and_identity_fields(self) -> None:
        invalid = self.sample_trace([("MatMul", 50)])
        invalid[1]["dur"] = float("nan")
        with self.assertRaisesRegex(WitnessError, "trace_numeric_field_invalid"):
            parse_trace(invalid, 1)
        invalid = self.sample_trace([("raw path/operator", 50)])
        with self.assertRaisesRegex(WitnessError, "trace_node_identity_invalid"):
            parse_trace(invalid, 1)

    def test_operator_report_is_capped_at_twenty(self) -> None:
        self.assertEqual(len(parse_trace(self.sample_trace([(f"Op{index}", 1) for index in range(30)]), 1)["top_operators"]), 20)

    def test_decision_requires_stable_share_buckets_and_cross_check(self) -> None:
        capture = parse_trace(self.sample_trace([("MatMulInteger", 70), ("Softmax", 10)]), 1)
        buckets = {bucket: capture for bucket in TOKEN_BUCKETS}
        self.assertEqual(decide([capture] * 5, buckets, {"conflicts": False})["outcome"], "bottleneck_selected")
        self.assertEqual(decide([capture] * 5, buckets, {"conflicts": True})["outcome"], "inconclusive")

    def test_report_is_bounded_and_privacy_fail_closed(self) -> None:
        encode_report({"privacy": PRIVACY}, ("/private/root",))
        with self.assertRaisesRegex(WitnessError, "report_privacy_boundary_failed"):
            encode_report({"value": "/private/root"}, ("/private/root",))

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--binary", type=Path, required=not "--self-test" in os.sys.argv)
    parser.add_argument("--revision", required=not "--self-test" in os.sys.argv)
    parser.add_argument("--runtime-dir", type=Path, required=not "--self-test" in os.sys.argv)
    parser.add_argument("--out", type=Path, required=not "--self-test" in os.sys.argv)
    args = parser.parse_args()
    if args.self_test:
        return args
    if re.fullmatch(r"[0-9a-f]{40}", args.revision or "") is None:
        parser.error("revision must be an exact lowercase 40-character Git SHA")
    for name in ("binary", "runtime_dir"):
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
        encoded = encode_report(report, (str(args.binary), str(args.runtime_dir), str(args.out)))
        args.out.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        args.out.write_bytes(encoded)
        args.out.chmod(0o600)
        print(encoded.decode(), end="")
        return 0 if report["decision"]["outcome"] == "bottleneck_selected" else 2  # type: ignore[index]
    except WitnessError as error:
        print(json.dumps({"schema_version": REPORT_SCHEMA, "error": str(error)}))
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
