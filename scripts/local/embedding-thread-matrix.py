#!/usr/bin/env python3
"""Run the bounded Issue #305 resident ONNX intra-op thread matrix."""
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
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any
SCHEMA = "resume-ir.embedding-thread-matrix.v1"
STREAM = "resume-ir.embedding-stream.v1"
MODEL_ID = "intfloat-multilingual-e5-small-qint8-r1"
DIMENSION = 384
THREADS = (1, 2, 3, 4, 6)
SEED = 20260802
PRIMARY = "b4_512"
BUCKETS = (PRIMARY, "b4_32", "b4_96", "b4_256", "b1_32_query")
COUNTS = {PRIMARY: 20, "b4_32": 10, "b4_96": 10, "b4_256": 10, "b1_32_query": 50}
TOKENS = {PRIMARY: 512, "b4_32": 32, "b4_96": 96, "b4_256": 256, "b1_32_query": 32}
WARMUP_SECONDS = 30.0
TIMEOUT_SECONDS = 90.0
MAX_REPORT_BYTES = 64 * 1024
MIB = 1024 * 1024
PRIVACY = {f"contains_{name}": False for name in (
    "raw_resume_text raw_query candidate_results local_paths vectors token_content "
    "runtime_or_model_bytes pids raw_symbols raw_profiler_data").split()}
HELPER_ROOT = Path(__file__).resolve().parent
PROF = runpy.run_path(str(HELPER_ROOT / "embedding-onnx-operator-profile.py"))
PRE = runpy.run_path(str(HELPER_ROOT / "embedding-prepacking-benchmark.py"))
class WitnessError(RuntimeError):
    """A fixed, public-safe witness failure."""
@dataclass
class SessionResult:
    ready_ms: float
    onnx: dict[str, list[int]]
    wall: dict[str, list[int]]
    process_cpu_us: int
    rss_bytes: int
    footprint_bytes: int
    last_vectors: dict[str, tuple[tuple[float, ...], ...]]
@dataclass
class CandidateAggregate:
    sessions: list[SessionResult]
    primary_onnx_by_block: list[float]
    primary_wall_by_block: list[float]
def williams(values: tuple[Any, ...]) -> list[list[Any]]:
    if len(values) < 2 or len(values) % 2 == 0:
        raise WitnessError("williams_requires_odd_cardinality")
    offsets = [0]
    for index in range(1, len(values)):
        offsets.append((index + 1) // 2 if index % 2 else -(index // 2))
    rows = [[values[(start + offset) % len(values)] for offset in offsets] for start in range(len(values))]
    return rows + [list(reversed(row)) for row in rows]
def percentile(values: list[int] | list[float], quantile: float) -> float:
    if not values or not 0 < quantile <= 1 or any(not math.isfinite(float(value)) or value < 0 for value in values):
        raise WitnessError("metric_invalid")
    ordered = sorted(float(value) for value in values)
    return ordered[max(0, math.ceil(len(ordered) * quantile) - 1)]
def parse_cpu_time(text: str) -> int:
    match = re.fullmatch(r"(?:(\d+)-)?(\d+):(\d+):(\d+(?:\.\d+)?)|(\d+):(\d+(?:\.\d+)?)", text)
    if match is None:
        raise WitnessError("process_cpu_unavailable")
    if match.group(5) is not None:
        seconds = int(match.group(5)) * 60 + float(match.group(6))
    else:
        seconds = (((int(match.group(1) or 0) * 24 + int(match.group(2))) * 60
                    + int(match.group(3))) * 60 + float(match.group(4)))
    return round(seconds * 1_000_000)

def cpu_time_us(pid: int) -> int:
    try:
        text = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], check=True,
                              capture_output=True, text=True, timeout=5).stdout.strip()
        value = parse_cpu_time(text)
    except (OSError, subprocess.SubprocessError):
        raise WitnessError("process_cpu_unavailable") from None
    if value < 0:
        raise WitnessError("process_cpu_unavailable")
    return value
class Resident:
    def __init__(self, binary: Path, runtime: Path, threads: int, *, ordinary: bool = False,
                 profile_prefix: Path | None = None) -> None:
        environment = os.environ.copy()
        environment.update({
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(runtime),
            "RESUME_IR_EMBEDDING_MODEL_ID": MODEL_ID,
            "RESUME_IR_EMBEDDING_DIMENSION": str(DIMENSION),
            "RESUME_IR_EMBEDDING_INTRA_THREADS": "3",
        })
        if ordinary:
            mode = "--resident"
        else:
            mode = "--resident-thread-profile" if profile_prefix else "--resident-thread-matrix"
            environment["RESUME_IR_EMBEDDING_THREAD_EXPERIMENT_INTRA_THREADS"] = str(threads)
        if profile_prefix:
            environment["RESUME_IR_EMBEDDING_PROFILE_OUTPUT_PREFIX"] = str(profile_prefix)
        self.stderr = tempfile.TemporaryFile()
        started = time.perf_counter_ns()
        try:
            self.process = subprocess.Popen([str(binary), mode], env=environment,
                                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=self.stderr, start_new_session=True)
        except OSError:
            self.stderr.close()
            raise WitnessError("resident_start_failed") from None
        self.request_id = 0
        if self.process.stdin is None or self.process.stdout is None:
            self.stop()
            raise WitnessError("resident_pipe_unavailable")
        ready = PROF["read_frame"](self.process.stdout, TIMEOUT_SECONDS)
        if ready != {"type": "ready", "schema_version": STREAM, "model_id": MODEL_ID, "dimension": DIMENSION}:
            self.stop()
            raise WitnessError("ready_identity_mismatch")
        self.ready_ms = (time.perf_counter_ns() - started) / 1_000_000

    def request(self, inputs: list[dict[str, str]]) -> tuple[int, tuple[tuple[float, ...], ...], tuple[int, int, int], int]:
        self.request_id += 1
        request = {"schema_version": STREAM, "request_id": self.request_id, "model_id": MODEL_ID,
                   "dimension": DIMENSION, "inputs": inputs}
        started = time.perf_counter_ns()
        PROF["write_frame"](self.process.stdin, request)
        response = PROF["read_frame"](self.process.stdout, TIMEOUT_SECONDS)
        wall_us = (time.perf_counter_ns() - started) // 1_000
        try:
            onnx_us, vectors, signature = PRE["validate_result"](
                response, self.request_id, expected_inputs=len(inputs),
                expected_active_tokens=None, expected_padded_tokens=None,
            )
        except RuntimeError:
            raise WitnessError("resident_result_invalid") from None
        return onnx_us, vectors, signature, wall_us

    def close(self) -> None:
        self.process.stdin.close()
        try:
            if self.process.wait(timeout=20) != 0:
                raise WitnessError("resident_exit_failed")
        except subprocess.TimeoutExpired:
            raise WitnessError("resident_exit_timeout") from None

    def stop(self) -> None:
        if getattr(self, "process", None) is not None:
            PRE["stop_process"](self.process)
        if getattr(self, "stderr", None) is not None:
            self.stderr.close()
            self.stderr = None
def calibrate_query(binary: Path, runtime: Path) -> list[dict[str, str]]:
    resident = Resident(binary, runtime, 3, ordinary=True)
    try:
        low, high = 1, 1024
        while low <= high:
            count = (low + high) // 2
            inputs = [{"role": "query", "text": " ".join(["alpha"] * count)}]
            _, _, signature, _ = resident.request(inputs)
            if signature[1] == 32:
                return inputs
            if signature[1] < 32:
                low = count + 1
            else:
                high = count - 1
        raise WitnessError("query_token_calibration_failed")
    finally:
        resident.stop()
def warm(resident: Resident, inputs: list[dict[str, str]], seconds: float = WARMUP_SECONDS) -> None:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        resident.request(inputs)
        time.sleep(min(1.0, max(0.0, deadline - time.monotonic())))
def run_session(binary: Path, runtime: Path, threads: int, workloads: dict[str, list[dict[str, str]]],
                order: list[str], *, ordinary: bool = False, warmup: float = WARMUP_SECONDS) -> SessionResult:
    resident = Resident(binary, runtime, threads, ordinary=ordinary)
    onnx = {bucket: [] for bucket in order}
    wall = {bucket: [] for bucket in order}
    last_vectors: dict[str, tuple[tuple[float, ...], ...]] = {}
    try:
        warm(resident, workloads[PRIMARY], warmup)
        for bucket in order:
            expected_active = len(workloads[bucket]) * TOKENS[bucket]
            for _ in range(COUNTS[bucket]):
                onnx_us, vectors, signature, wall_us = resident.request(workloads[bucket])
                if signature[1] != expected_active:
                    raise WitnessError("measured_token_contract_invalid")
                onnx[bucket].append(onnx_us)
                wall[bucket].append(wall_us)
                last_vectors[bucket] = vectors
        process_cpu = cpu_time_us(resident.process.pid)
        rss = PRE["rss_bytes"](resident.process.pid)
        footprint = PRE["physical_footprint"](resident.process.pid)[1]
        resident.close()
        return SessionResult(resident.ready_ms, onnx, wall, process_cpu, rss, footprint, last_vectors)
    finally:
        resident.stop()
def quality(binary: Path, runtime: Path, threads: int, *, ordinary: bool = False) -> Any:
    resident = Resident(binary, runtime, threads, ordinary=ordinary)
    vectors: list[tuple[float, ...]] = []
    signatures: list[tuple[int, int, int]] = []
    buckets: list[int] = []
    try:
        inputs = PRE["quality_workload"]()
        for offset in range(0, len(inputs), 4):
            _, current, signature, _ = resident.request(inputs[offset:offset + 4])
            vectors.extend(current)
            signatures.append(signature)
            buckets.append(signature[1] // signature[0])
        resident.close()
        return PRE["QualityResult"](tuple(vectors), tuple(buckets), tuple(signatures))
    finally:
        resident.stop()
def aggregate(sessions: list[SessionResult]) -> CandidateAggregate:
    return CandidateAggregate(
        sessions,
        [statistics.median(session.onnx[PRIMARY]) for session in sessions],
        [statistics.median(session.wall[PRIMARY]) for session in sessions],
    )
def improvement(control: float, candidate: float) -> float:
    if control <= 0 or candidate <= 0:
        raise WitnessError("metric_invalid")
    return (control - candidate) * 100.0 / control
def candidate_summary(value: CandidateAggregate) -> dict[str, object]:
    buckets: dict[str, object] = {}
    for bucket in BUCKETS:
        onnx = [sample for session in value.sessions for sample in session.onnx[bucket]]
        wall = [sample for session in value.sessions for sample in session.wall[bucket]]
        inputs = 1 if bucket == "b1_32_query" else 4
        active = inputs * TOKENS[bucket]
        buckets[bucket] = {"requests": len(onnx), "onnx_us_p50": percentile(onnx, 0.50),
                           "onnx_us_p95": percentile(onnx, 0.95),
                           "wall_us_p50": percentile(wall, 0.50),
                           "wall_us_p95": percentile(wall, 0.95),
                           "onnx_us_per_input_p50": percentile(onnx, 0.50) / inputs,
                           "onnx_us_per_active_token_p50": percentile(onnx, 0.50) / active}
    return {"sessions": len(value.sessions), "buckets": buckets,
            "ready_ms_median": statistics.median(session.ready_ms for session in value.sessions),
            "process_cpu_us_median": statistics.median(session.process_cpu_us for session in value.sessions),
            "rss_bytes_peak": max(session.rss_bytes for session in value.sessions),
            "physical_footprint_bytes_peak": max(session.footprint_bytes for session in value.sessions)}
def decide(values: dict[int, CandidateAggregate], quality_gates: dict[int, dict[str, object]],
           mode_overhead_pct: float, mode_exact: bool) -> dict[str, object]:
    control = values[3]
    control_primary = statistics.median(control.primary_onnx_by_block)
    control_ready = statistics.median(session.ready_ms for session in control.sessions)
    passing: list[int] = []
    gates: dict[str, object] = {}
    for threads in THREADS:
        if threads == 3:
            continue
        candidate = values[threads]
        onnx_ci = PRE["bootstrap_improvement_interval"](
            control.primary_onnx_by_block, candidate.primary_onnx_by_block, SEED + threads)
        wall_ci = PRE["bootstrap_improvement_interval"](
            control.primary_wall_by_block, candidate.primary_wall_by_block, SEED + 100 + threads)
        gain = improvement(control_primary, statistics.median(candidate.primary_onnx_by_block))
        sensitivity = max(
            -improvement(
                statistics.median(sample for session in control.sessions for sample in session.onnx[bucket]),
                statistics.median(sample for session in candidate.sessions for sample in session.onnx[bucket]),
            ) for bucket in ("b4_32", "b4_96", "b4_256")
        )
        batch1_regression = -improvement(
            percentile([sample for session in control.sessions for sample in session.wall["b1_32_query"]], 0.95),
            percentile([sample for session in candidate.sessions for sample in session.wall["b1_32_query"]], 0.95),
        )
        ready = statistics.median(session.ready_ms for session in candidate.sessions)
        ready_delta = ready - control_ready
        ready_pass = ready_delta <= 1_000 and ready_delta * 100 / control_ready <= 10
        resource_pass = (
            max(session.footprint_bytes for session in values[1].sessions) <= 512 * MIB
            and max(session.footprint_bytes for session in candidate.sessions) <= 1_536 * MIB
        )
        accepted = (
            gain >= 10 and onnx_ci[0] > 0 and wall_ci[0] > 0 and sensitivity <= 3
            and batch1_regression <= 5 and mode_overhead_pct <= 1 and mode_exact
            and quality_gates[threads]["passed"] is True and ready_pass and resource_pass
        )
        gates[str(threads)] = {"primary_onnx_improvement_pct": gain,
                               "onnx_bootstrap_95pct": onnx_ci, "wall_bootstrap_95pct": wall_ci,
                               "maximum_sensitivity_regression_pct": sensitivity,
                               "batch1_wall_p95_regression_pct": batch1_regression,
                               "quality_pass": quality_gates[threads]["passed"],
                               "ready_pass": ready_pass, "resource_pass": resource_pass,
                               "accepted": accepted}
        if accepted:
            passing.append(threads)
    passing.sort(key=lambda threads: statistics.median(values[threads].primary_onnx_by_block))
    if not passing:
        return {"outcome": "lost", "winner": None, "gates": gates}
    if len(passing) > 1:
        fastest, runner_up = passing[:2]
        tie_ci = PRE["bootstrap_improvement_interval"](
            values[runner_up].primary_onnx_by_block, values[fastest].primary_onnx_by_block, SEED + 500)
        if tie_ci[0] <= 0:
            return {"outcome": "inconclusive", "winner": None, "gates": gates,
                    "fastest_vs_runner_up_95pct": tie_ci}
    return {"outcome": "won", "winner": passing[0], "gates": gates}
def profile_winner(binary: Path, runtime: Path, threads: int, inputs: list[dict[str, str]]) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="resume-ir-thread-profile-") as raw:
        root = Path(raw)
        root.chmod(0o700)
        plain = Resident(binary, runtime, threads)
        try:
            warm(plain, inputs)
            control = None
            for _ in range(20):
                _, control, _, _ = plain.request(inputs)
            plain.close()
        finally:
            plain.stop()
        captures: list[dict[str, object]] = []
        for index in range(5):
            prefix = root / f"capture-{index}"
            resident = Resident(binary, runtime, threads, profile_prefix=prefix)
            try:
                warm(resident, inputs)
                current = None
                for _ in range(20):
                    _, current, _, _ = resident.request(inputs)
                resident.close()
                traces = list(root.glob(f"capture-{index}*.json"))
                if len(traces) != 1 or current != control:
                    raise WitnessError("winner_profile_control_failed")
                captures.append(PROF["read_trace"](traces[0], 20))
            finally:
                resident.stop()
        top = [str(capture["families"][0]["family"]) for capture in captures]
        dynamic_top = top.count("dynamic_quantization")
        dynamic_share = statistics.median(
            next((float(item["node_share"]) for item in capture["families"]
                  if item["family"] == "dynamic_quantization"), 0.0) for capture in captures)
        cross = cross_check(binary, runtime, threads, inputs, root)
        return {"captures": 5, "exact_profiled_unprofiled_vectors": True,
                "dynamic_quantization_top_captures": dynamic_top,
                "dynamic_quantization_share_median": dynamic_share, "cross_check": cross,
                "passed": dynamic_top >= 4 and cross["conflicts"] is False}
def cross_check(binary: Path, runtime: Path, threads: int, inputs: list[dict[str, str]], root: Path) -> dict[str, object]:
    resident = Resident(binary, runtime, threads)
    method, family = "xctrace_time_profiler", None
    try:
        resident.request(inputs)
        trace = root / "time-profile.trace"
        sampler = subprocess.Popen(["xcrun", "xctrace", "record", "--template", "Time Profiler",
                                    "--attach", str(resident.process.pid), "--time-limit", "20s",
                                    "--output", str(trace), "--no-prompt"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        PROF["drive_sampler"](resident, inputs, sampler, 35)
        if sampler.returncode == 0:
            exported = subprocess.run(["xcrun", "xctrace", "export", "--input", str(trace),
                                       "--xpath", '/trace-toc/run[@number="1"]/data/table'],
                                      capture_output=True, timeout=30, check=False)
            if exported.returncode == 0:
                family = PROF["symbol_family"](exported.stdout.decode(errors="ignore"))
        if family is None:
            method = "sample_fallback"
            sample = root / "sample.txt"
            fallback = subprocess.Popen(["sample", str(resident.process.pid), "20", "1", "-file",
                                         str(sample)], stdout=subprocess.DEVNULL,
                                        stderr=subprocess.DEVNULL)
            PROF["drive_sampler"](resident, inputs, fallback, 35)
            if fallback.returncode == 0 and sample.exists():
                family = PROF["symbol_family"](sample.read_text(errors="ignore"))
        resident.close()
    finally:
        resident.stop()
    value = family or "unavailable"
    return {"method": method, "symbol_family": value,
            "conflicts": value in {"unavailable", "allocator", "scheduling"}}
def encode(report: dict[str, object], denied: tuple[str, ...]) -> bytes:
    try:
        raw = (json.dumps(report, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n").encode()
    except (TypeError, ValueError):
        raise WitnessError("report_numeric_invalid") from None
    if len(raw) > MAX_REPORT_BYTES:
        raise WitnessError("report_size_exceeded")
    if any(value and value.encode() in raw for value in denied) or any(
        marker in raw for marker in (b"/Users/", b"/private/", b"raw_trace", b"token_ids")
    ):
        raise WitnessError("report_privacy_boundary_failed")
    return raw
def run_experiment(args: argparse.Namespace) -> dict[str, object]:
    calibrated = PROF["calibrate_workloads"](args.binary, args.runtime_dir, TIMEOUT_SECONDS)
    workloads = {PRIMARY: calibrated[512], "b4_32": calibrated[32], "b4_96": calibrated[96],
                 "b4_256": calibrated[256], "b1_32_query": calibrate_query(args.binary, args.runtime_dir)}
    thread_rows, bucket_rows = williams(THREADS), williams(BUCKETS)
    sessions: dict[int, list[SessionResult]] = {threads: [] for threads in THREADS}
    for block, row in enumerate(thread_rows):
        for position, threads in enumerate(row):
            session_index = block * len(THREADS) + position
            sessions[threads].append(run_session(
                args.binary, args.runtime_dir, threads, workloads,
                bucket_rows[(session_index + SEED) % len(bucket_rows)],
            ))
    values = {threads: aggregate(result) for threads, result in sessions.items()}
    control_quality = quality(args.binary, args.runtime_dir, 3, ordinary=True)
    qualities: dict[int, dict[str, object]] = {}
    for threads in THREADS:
        summary = PRE["quality_summary"](control_quality, quality(args.binary, args.runtime_dir, threads))
        summary["passed"] = bool(summary["passed"] and summary["exact_vector_count"] == summary["vector_count"])
        qualities[threads] = summary
    ordinary_controls, experiment_controls, exact = [], [], True
    for block in range(5):
        pair: dict[str, SessionResult] = {}
        for label in (("ordinary", "experiment") if block % 2 == 0 else ("experiment", "ordinary")):
            pair[label] = run_session(
                args.binary, args.runtime_dir, 3, workloads, [PRIMARY],
                ordinary=label == "ordinary",
            )
        ordinary_controls.append(statistics.median(pair["ordinary"].onnx[PRIMARY]))
        experiment_controls.append(statistics.median(pair["experiment"].onnx[PRIMARY]))
        exact = exact and pair["ordinary"].last_vectors[PRIMARY] == pair["experiment"].last_vectors[PRIMARY]
    mode_overhead = statistics.median(
        (experiment - ordinary) * 100 / ordinary
        for ordinary, experiment in zip(ordinary_controls, experiment_controls)
    )
    decision = decide(values, qualities, mode_overhead, exact)
    winner_profile = None
    if decision["outcome"] == "won":
        winner_profile = profile_winner(args.binary, args.runtime_dir, int(decision["winner"]), workloads[PRIMARY])
        if winner_profile["passed"] is not True:
            decision = {**decision, "outcome": "inconclusive", "winner": None,
                        "winner_profile_conflict": True}
    return {"schema_version": SCHEMA, "issue": 305,
            "source": "public_synthetic_resident_embedding", "claim": "candidate_selection_only",
            "revision": args.revision,
            "workload": {"seed": SEED, "candidates": list(THREADS), "blocks": 10,
                         "matrix_sessions": 50, "mode_control_sessions": 10,
                         "warmup_seconds": WARMUP_SECONDS, "measured_requests": COUNTS},
            "variants": {str(threads): candidate_summary(values[threads]) for threads in THREADS},
            "mode_control": {"pairs": 5, "median_overhead_pct": mode_overhead,
                             "overhead_pass": mode_overhead <= 1, "exact_vectors": exact},
            "quality": {str(threads): summary for threads, summary in qualities.items()},
            "decision": decision, "winner_profile": winner_profile, "privacy": PRIVACY}
class SelfTests(unittest.TestCase):
    def fake(self, primary: float, sensitivity: float | None = None, batch1: float = 100.0) -> CandidateAggregate:
        value = primary if sensitivity is None else sensitivity
        sessions = [SessionResult(100, {PRIMARY: [int(primary)], "b4_32": [int(value)],
                    "b4_96": [int(value)], "b4_256": [int(value)], "b1_32_query": [int(batch1)]},
                    {PRIMARY: [int(primary)], "b4_32": [int(value)], "b4_96": [int(value)],
                     "b4_256": [int(value)], "b1_32_query": [int(batch1)]}, 1, 1, 1, {})
                    for _ in range(10)]
        return aggregate(sessions)

    def test_williams_balances_positions_and_ordered_carryover(self) -> None:
        rows = williams(THREADS)
        self.assertEqual(len(rows), 10)
        for position in range(5):
            self.assertEqual(Counter(row[position] for row in rows), Counter({value: 2 for value in THREADS}))
        pairs = Counter((row[index], row[index + 1]) for row in rows for index in range(4))
        self.assertEqual(set(pairs.values()), {2})

    def test_decision_lost_unique_and_tied(self) -> None:
        quality_gates = {threads: {"passed": True} for threads in THREADS}
        values = {threads: self.fake(100) for threads in THREADS}
        self.assertEqual(decide(values, quality_gates, 0, True)["outcome"], "lost")
        values[1] = self.fake(80)
        self.assertEqual(decide(values, quality_gates, 0, True)["winner"], 1)
        values[2] = self.fake(80)
        self.assertEqual(decide(values, quality_gates, 0, True)["outcome"], "inconclusive")

    def test_report_rejects_nan_paths_and_oversize(self) -> None:
        with self.assertRaisesRegex(WitnessError, "numeric"):
            encode({"value": float("nan")}, ())
        with self.assertRaisesRegex(WitnessError, "privacy"):
            encode({"value": "/private/root"}, ())
        with self.assertRaisesRegex(WitnessError, "size"):
            encode({"value": "x" * MAX_REPORT_BYTES}, ())

    def test_cpu_time_parser_contract_is_strict(self) -> None:
        self.assertEqual(parse_cpu_time("1:02.50"), 62_500_000)
        self.assertEqual(parse_cpu_time("1-02:03:04.5"), 93_784_500_000)
        self.assertEqual(percentile([1, 2, 3, 4], 0.95), 4)
        with self.assertRaisesRegex(WitnessError, "metric_invalid"):
            percentile([float("nan")], 0.5)
def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--runtime-dir", type=Path)
    parser.add_argument("--revision")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    if args.self_test:
        return args
    if not all((args.binary, args.runtime_dir, args.revision, args.out)):
        parser.error("binary, runtime-dir, revision, and out are required")
    if re.fullmatch(r"[0-9a-f]{40}", args.revision) is None:
        parser.error("revision must be an exact lowercase Git SHA")
    try:
        args.binary = args.binary.resolve(strict=True)
        args.runtime_dir = args.runtime_dir.resolve(strict=True)
    except OSError:
        parser.error("binary and runtime-dir must resolve")
    args.out = args.out.resolve(strict=False)
    return args
def main() -> int:
    args = parse_args()
    if args.self_test:
        suite = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromTestCase(case)
                                   for case in (SelfTests, PRE["SelfTests"], PROF["SelfTests"]))
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    try:
        report = run_experiment(args)
        encoded = encode(report, (str(args.binary), str(args.runtime_dir), str(args.out)))
        args.out.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        args.out.write_bytes(encoded)
        args.out.chmod(0o600)
        print(encoded.decode(), end="")
        return 0
    except (WitnessError, RuntimeError, ValueError) as error:
        print(json.dumps({"schema_version": SCHEMA, "error": str(error)}))
        return 1
if __name__ == "__main__":
    raise SystemExit(main())
