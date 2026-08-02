#!/usr/bin/env python3
"""Run the frozen public #319 resident embedding role-isolation matrix."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import platform
import random
import re
import runpy
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OCR_HELPERS = runpy.run_path(str(ROOT / "scripts/local/ocr-on-import-attribution.py"))
PRESSURE_HELPERS = runpy.run_path(str(ROOT / "scripts/local/run-mixed-import-variance.py"))
STREAM_HELPERS = runpy.run_path(str(ROOT / "scripts/local/embedding-batch-benchmark.py"))
RESOURCE_HELPERS = runpy.run_path(str(ROOT / "scripts/local/embedding-prepacking-benchmark.py"))
CONTRACT = runpy.run_path(str(ROOT / "scripts/ci/check-resident-embedding-role-isolation.py"))
HttpTransport = OCR_HELPERS["HttpTransport"]
ManagedProcess = OCR_HELPERS["ManagedProcess"]
group_exists = OCR_HELPERS["group_exists"]
SystemPressureMonitor = PRESSURE_HELPERS["SystemPressureMonitor"]
read_frame = STREAM_HELPERS["read_frame"]
write_frame = STREAM_HELPERS["write_frame"]
percentile = STREAM_HELPERS["percentile"]
physical_footprint = RESOURCE_HELPERS["physical_footprint"]

SCHEMA = "resume-ir.resident-embedding-role-isolation.v1"
OBSERVER_SCHEMA = "resume-ir.resident-role-isolation-observer.v1"
STREAM_SCHEMA = "resume-ir.embedding-stream.v1"
IPC_PROTOCOL = "resume-ir.daemon-ipc.v5"
MODEL_ID = "intfloat-multilingual-e5-small-qint8-r1"
DIMENSION = 384
SEED = 20260802
MAX_REPORT_BYTES = 64 * 1024
MAX_LOAD_DRIFT = 0.25
ARMS = tuple(CONTRACT["ARMS"])
ARM_IDENTITY = {
    "shared_i3_b4": ("shared", 3, 1),
    "split_i3_bulk3_b4": ("split", 3, 2),
    "split_i3_bulk4_b4": ("split", 4, 2),
}
WILLIAMS_ROWS = (
    ARMS,
    (ARMS[0], ARMS[2], ARMS[1]),
    (ARMS[1], ARMS[0], ARMS[2]),
    (ARMS[1], ARMS[2], ARMS[0]),
    (ARMS[2], ARMS[0], ARMS[1]),
    (ARMS[2], ARMS[1], ARMS[0]),
)
FORMAL_SCHEDULE = WILLIAMS_ROWS + tuple(WILLIAMS_ROWS[index] for index in (0, 1, 2, 5))
FIXED_WORKLOAD = dict(CONTRACT["FIXED_WORKLOAD"])
PRIVACY = {key: False for key in CONTRACT["PRIVACY"]}

class ExperimentError(RuntimeError):
    """A fixed, public-safe experiment failure code."""

@dataclass(frozen=True)
class Inputs:
    daemon: Path; embedding: Path; runtime_dir: Path; classifier: Path
    revision: str; output: Path; timeout: float

@dataclass
class SessionResult:
    block: int; arm: str; batches: int; inputs: int; throughput: float
    latencies: list[float]; successes: int; memory_mib: float
    correctness: dict[str, bool]; load_guard: bool

class Resident:
    def __init__(self, binary: Path, runtime_dir: Path, mode: str, threads: int, timeout: float):
        environment = os.environ.copy()
        environment.update({
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(runtime_dir),
            "RESUME_IR_EMBEDDING_MODEL_ID": MODEL_ID,
            "RESUME_IR_EMBEDDING_DIMENSION": str(DIMENSION),
            "RESUME_IR_EMBEDDING_INTRA_THREADS": str(threads),
        })
        self.timeout = timeout
        try:
            self.process = subprocess.Popen(
                [str(binary), mode], env=environment, stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True,
            )
        except OSError:
            raise ExperimentError("resident_start_failed") from None
        if self.process.stdin is None or self.process.stdout is None:
            self.close()
            raise ExperimentError("resident_pipes_missing")
        ready = read_frame(self.process.stdout, timeout)
        if ready != {"type": "ready", "schema_version": STREAM_SCHEMA,
                     "model_id": MODEL_ID, "dimension": DIMENSION}:
            self.close()
            raise ExperimentError("resident_ready_invalid")
        self.request_id = 0

    def embed(self, texts: list[str], role: str) -> tuple[list[list[float]], int]:
        self.request_id += 1
        request = {
            "schema_version": STREAM_SCHEMA,
            "request_id": self.request_id,
            "model_id": MODEL_ID,
            "dimension": DIMENSION,
            "inputs": [{"role": role, "text": text} for text in texts],
        }
        write_frame(self.process.stdin, request)
        response = read_frame(self.process.stdout, self.timeout)
        vectors, telemetry = response.get("vectors"), response.get("telemetry")
        if (response.get("type") != "result" or response.get("request_id") != self.request_id
                or not isinstance(vectors, list) or len(vectors) != len(texts)
                or not isinstance(telemetry, dict)):
            raise ExperimentError("resident_result_invalid")
        if any(not isinstance(vector, list) or len(vector) != DIMENSION for vector in vectors):
            raise ExperimentError("resident_result_invalid")
        active = telemetry.get("active_token_count")
        if not isinstance(active, int):
            raise ExperimentError("resident_telemetry_invalid")
        return vectors, active

    def close(self) -> None:
        if self.process.poll() is None:
            if self.process.stdin is not None:
                self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()

    def __enter__(self) -> "Resident":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()
def passage(index: int) -> str:
    header = (
        f"Synthetic Candidate {index:05d}\nProfessional Summary\n"
        "Software engineer building deterministic local systems.\nWork Experience\n"
        "Developed reliable Rust and Python services with measured performance.\n"
        "Education\nSynthetic Institute\nSkills\nRust Python SQL systems performance\n"
    )
    return header + ("throughputdocument scheduling isolation benchmark evidence " * 180)
def calibrate_query(resident: Resident) -> str:
    for word in ("isolationneedle", "syntheticneedle", "q"):
        for count in range(1, 129):
            candidate = " ".join([word] * count)
            _, active = resident.embed([candidate], "query")
            if active == 32:
                return candidate
            if active > 32:
                break
    raise ExperimentError("query_32_token_calibration_failed")
def vector_preflight(inputs: Inputs) -> tuple[list[str], str, dict[str, dict[str, bool]]]:
    texts = [passage(index) for index in range(4)]
    with Resident(inputs.embedding, inputs.runtime_dir, "--resident", 3, inputs.timeout) as control:
        control_vectors, active = control.embed(texts, "passage")
        query = calibrate_query(control)
        control_query, query_active = control.embed([query], "query")
    if active != 4 * 512 or query_active != 32:
        raise ExperimentError("frozen_token_shape_failed")
    result = {}
    for arm in ARMS:
        if arm == ARMS[0]:
            vectors, query_vectors = control_vectors, control_query
        else:
            threads = ARM_IDENTITY[arm][1]
            with Resident(inputs.embedding, inputs.runtime_dir,
                          "--resident-role-isolation-experiment", threads,
                          inputs.timeout) as candidate:
                vectors, arm_active = candidate.embed(texts, "passage")
                query_vectors, arm_query_active = candidate.embed([query], "query")
            if arm_active != 4 * 512 or arm_query_active != 32:
                raise ExperimentError("frozen_token_shape_failed")
        result[arm] = {
            "vectors_elementwise_exact": vectors == control_vectors and query_vectors == control_query,
            "counts_exact": len(vectors) == 4 and len(query_vectors) == 1,
            "order_exact": all(vector == control_vectors[index] for index, vector in enumerate(vectors)),
        }
    return texts, query, result
def read_small_json(path: Path) -> dict[str, object]:
    body = path.read_bytes()
    if len(body) > 64 * 1024:
        raise ExperimentError("owner_file_too_large")
    value = json.loads(body)
    if not isinstance(value, dict):
        raise ExperimentError("owner_file_invalid")
    return value
def wait_ready(process: object, data: Path, timeout: float) -> object:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.process.poll() is not None:  # type: ignore[attr-defined]
            raise ExperimentError("daemon_exited_before_ready")
        try:
            endpoints = read_small_json(data / "ipc.endpoints.json")
            auth = read_small_json(data / "ipc.auth")
            endpoint, token = endpoints.get("status"), auth.get("token")
            if not isinstance(endpoint, str) or not isinstance(token, str):
                raise ExperimentError("owner_file_invalid")
            transport = HttpTransport(endpoint, token, timeout)
            status = transport.get_json("/status")
        except (FileNotFoundError, json.JSONDecodeError, RuntimeError):
            time.sleep(0.05)
            continue
        runtimes, capabilities = status.get("optional_runtimes"), status.get("capabilities")
        runtime_ready = isinstance(runtimes, dict) and all(
            isinstance(runtimes.get(key), dict) and runtimes[key].get("state") == "available"
            for key in ("embedding", "classifier")
        )
        capability_ready = isinstance(capabilities, dict) and all(
            isinstance(capabilities.get(key), dict) and capabilities[key].get("state") == "available"
            for key in ("text_import", "index_publication", "keyword_search",
                        "semantic_search", "hybrid_search")
        )
        if status.get("status") == "ok" and runtime_ready and capability_ready:
            roots_status, roots = transport.request("GET", "/source-roots")
            if roots_status == 200 and isinstance(roots.get("roots"), list):
                return transport
        time.sleep(0.05)
    raise ExperimentError("capability_attestation_timeout")
def start_import(transport: object, root: Path) -> None:
    status, body = transport.post_json("/imports", {  # type: ignore[attr-defined]
        "roots": [str(root)], "root_preset": None,
        "profile": "explicit", "max_files": None,
    })
    task_ids = body.get("task_ids")
    if (status != 202 or body.get("schema_version") != "daemon.import.v1"
            or body.get("status") != "accepted" or body.get("accepted_roots") != 1
            or not isinstance(task_ids, list) or len(task_ids) != 1
            or not isinstance(task_ids[0], str)):
        raise ExperimentError("import_not_accepted")
def query_request(query: str, request_id: str, deadline_ms: int = 10_000,
                  cancel_token: str | None = None) -> dict[str, object]:
    request = {
        "schema_version": "resume-ir.ipc-request.v3",
        "request_id": request_id,
        "client_capability": "benchmark",
        "deadline_ms": deadline_ms,
        "payload": {"query": query, "mode": "hybrid", "top_k": 1, "filters": {}},
    }
    if cancel_token is not None:
        request["cancel_token"] = cancel_token
    return request
def response_signature(status: int, body: dict[str, object], request_id: str) -> tuple[str, ...] | None:
    results = body.get("results")
    if (status != 200 or body.get("schema_version") != "resume-ir.search-response.v3"
            or body.get("request_id") != request_id or body.get("query_mode") != "hybrid"
            or body.get("search_index") != "available" or not isinstance(results, list)
            or body.get("result_count") != len(results) or len(results) != 1):
        return None
    selection = results[0].get("selection") if isinstance(results[0], dict) else None
    if not isinstance(selection, dict):
        return None
    doc, version = selection.get("doc_id"), selection.get("version_id")
    if not isinstance(doc, str) or not isinstance(version, str):
        return None
    return doc, version
def send_query(transport: object, query: str, request_id: str,
               deadline_ms: int = 10_000, cancel_token: str | None = None
               ) -> tuple[int, dict[str, object], float]:
    started = time.perf_counter()
    status, body = transport.post_json(  # type: ignore[attr-defined]
        "/search", query_request(query, request_id, deadline_ms, cancel_token)
    )
    return status, body, max((time.perf_counter() - started) * 1000.0, 0.001)
def wait_baseline(transport: object, query: str, timeout: float, tag: str) -> tuple[str, ...]:
    deadline, attempt = time.monotonic() + timeout, 0
    while time.monotonic() < deadline:
        status = transport.get_json("/status")  # type: ignore[attr-defined]
        if (status.get("searchable_documents", 0) < 1
                or status.get("indexed_documents", 0) < 1
                or status.get("index_health") != "ready"):
            time.sleep(0.1)
            continue
        attempt += 1
        status, body, _ = send_query(transport, query, f"baseline-{tag}-{attempt}")
        signature = response_signature(status, body, f"baseline-{tag}-{attempt}")
        if signature is not None:
            return signature
        time.sleep(0.5)
    raise ExperimentError("query_seed_not_searchable")
def lifecycle_probe(transport: object, query: str, timeout: float, tag: str) -> dict[str, bool]:
    timeout_id = f"timeout-{tag}"
    timeout_status, timeout_body, _ = send_query(transport, query, timeout_id, 1)
    timeout_exact = (
        timeout_status == 200
        and timeout_body.get("schema_version") == "resume-ir.search-response.v3"
        and timeout_body.get("request_id") == timeout_id
        and "deadline_exceeded" in timeout_body.get("partial_reasons", [])
    )
    search_id, cancel_id, token = f"cancel-search-{tag}", f"cancel-{tag}", f"token-{tag}"
    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(send_query, transport, query, search_id, 10_000, token)
        time.sleep(0.005)
        cancel_status, cancel_body = transport.post_json("/search/cancel", {  # type: ignore[attr-defined]
            "schema_version": "resume-ir.search-cancel-request.v1",
            "request_id": cancel_id,
            "cancel_token": token,
        })
        search_status, search_body, _ = future.result(timeout=timeout)
    cancellation_exact = (
        cancel_status == 200
        and cancel_body.get("schema_version") == "resume-ir.search-cancel-response.v1"
        and cancel_body.get("request_id") == cancel_id
        and cancel_body.get("status") in {"cancelled", "cancel_requested", "complete"}
        and search_status == 200
        and search_body.get("schema_version") == "resume-ir.search-response.v3"
        and search_body.get("request_id") == search_id
        and search_body.get("status") in {"ok", "cancelled"}
    )
    return {
        "cancellation_exact": cancellation_exact,
        "timeout_exact": timeout_exact,
        "ready_exact": True,
    }
def process_table() -> dict[int, tuple[int, str]]:
    try:
        output = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,comm="], check=True, text=True,
            capture_output=True, timeout=10,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        raise ExperimentError("process_tree_unavailable") from None
    table = {}
    for line in output.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) == 3 and fields[0].isdigit() and fields[1].isdigit():
            table[int(fields[0])] = (int(fields[1]), fields[2])
    return table
def descendants(root: int) -> set[int]:
    table, result, frontier = process_table(), {root}, [root]
    while frontier:
        parent = frontier.pop()
        children = [pid for pid, (ppid, _) in table.items() if ppid == parent]
        result.update(children)
        frontier.extend(children)
    return result
def process_tree_peak_mib(daemon_pid: int) -> float:
    try:
        peak = sum(physical_footprint(pid)[1] for pid in descendants(daemon_pid))
    except RuntimeError:
        raise ExperimentError("memory_measurement_failed") from None
    return peak / (1024 * 1024)
def read_observer(path: Path, timeout: float = 5.0) -> dict[str, int]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            value = read_small_json(path)
        except (FileNotFoundError, json.JSONDecodeError):
            time.sleep(0.02)
            continue
        keys = {"schema_version", "completed_calls", "completed_inputs",
                "active_token_count", "nonconforming_calls"}
        if set(value) != keys or value.get("schema_version") != OBSERVER_SCHEMA:
            raise ExperimentError("observer_invalid")
        counters = {key: value[key] for key in keys - {"schema_version"}}
        if any(not isinstance(item, int) or item < 0 for item in counters.values()):
            raise ExperimentError("observer_invalid")
        return counters  # type: ignore[return-value]
    raise ExperimentError("observer_unavailable")
def observer_delta(before: dict[str, int], after: dict[str, int]) -> dict[str, int]:
    delta = {key: after[key] - before[key] for key in before}
    if any(value < 0 for value in delta.values()):
        raise ExperimentError("observer_counter_regressed")
    return delta
def wait_saturated(path: Path, baseline: dict[str, int], process: object,
                   timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.process.poll() is not None:  # type: ignore[attr-defined]
            raise ExperimentError("daemon_exited_during_import")
        if not path.is_file():
            time.sleep(0.05)
            continue
        delta = observer_delta(baseline, read_observer(path))
        if delta["nonconforming_calls"] > 2:
            raise ExperimentError("bulk_saturation_shape_failed")
        if delta["completed_inputs"] >= 104 and delta["nonconforming_calls"] == 2:
            return
        time.sleep(0.05)
    raise ExperimentError("bulk_saturation_timeout")
def wait_seed_publication(path: Path, process: object, timeout: float) -> None:
    expected = {
        "completed_calls": 2, "completed_inputs": 4,
        "active_token_count": 4 * 512, "nonconforming_calls": 2,
    }
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.process.poll() is not None:  # type: ignore[attr-defined]
            raise ExperimentError("daemon_exited_during_seed_publication")
        if path.is_file():
            observed = read_observer(path)
            if observed == expected:
                return
            if any(observed[key] > expected[key] for key in expected):
                raise ExperimentError("seed_publication_shape_failed")
        time.sleep(0.05)
    raise ExperimentError("seed_publication_timeout")
def measured_queries(transport: object, query: str, baseline: tuple[str, ...],
                     samples: int, duration: float, tag: str, observer: Path
                     ) -> tuple[list[float], int, bool, dict[str, int]]:
    start, futures = time.monotonic(), []
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        for index in range(samples):
            target = start + index * 0.5
            time.sleep(max(0.0, target - time.monotonic()))
            request_id = f"measure-{tag}-{index}"
            futures.append((request_id, pool.submit(send_query, transport, query, request_id)))
        time.sleep(max(0.0, start + duration - time.monotonic()))
        window_end = read_observer(observer)
        outcomes = []
        for request_id, future in futures:
            try:
                status, body, elapsed = future.result(timeout=15)
                outcomes.append((elapsed, response_signature(status, body, request_id) == baseline))
            except (RuntimeError, concurrent.futures.TimeoutError):
                outcomes.append((15_000.0, False))
    return ([item[0] for item in outcomes], sum(item[1] for item in outcomes),
            all(item[1] for item in outcomes), window_end)
def normalized_load() -> float:
    return os.getloadavg()[0] / max(os.cpu_count() or 1, 1)
def daemon_command(inputs: Inputs, data: Path, arm: str) -> list[str]:
    return [
        str(inputs.daemon), "--data-dir", str(data), "run", "--foreground",
        "--work-imports", "--work-index",
        "--expected-ipc-protocol", IPC_PROTOCOL,
        "--ipc-listen", "127.0.0.1:0", "--embedding-command", str(inputs.embedding),
        "--embedding-model-id", MODEL_ID, "--embedding-dimension", str(DIMENSION),
        "--embedding-timeout-ms", "60000",
        "--resume-classifier-model", str(inputs.classifier),
        "--resident-role-isolation-arm", arm,
    ]
def run_session(inputs: Inputs, block: int, arm: str, query: str, seed_root: Path,
                bulk_root: Path, preflight: dict[str, bool], warmup: float,
                measurement: float) -> SessionResult:
    with tempfile.TemporaryDirectory(prefix="resume-ir-role-isolation-session-") as temporary:
        base, data = Path(temporary), Path(temporary) / "data"
        data.mkdir()
        observer = base / "observer.json"
        environment = os.environ.copy()
        environment.update({
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(inputs.runtime_dir),
            "RESUME_IR_RESIDENT_ROLE_ISOLATION_OBSERVER": str(observer),
        })
        process = ManagedProcess.start(daemon_command(inputs, data, arm), environment)
        cleanup_exact = False
        try:
            transport = wait_ready(process, data, inputs.timeout)
            tag = f"b{block}-{ARMS.index(arm)}"
            start_import(transport, seed_root)
            wait_seed_publication(observer, process, inputs.timeout)
            baseline = wait_baseline(transport, query, inputs.timeout, tag)
            process.cleanup()
            restart_cleanup = not group_exists(process.group)
            observer.unlink(missing_ok=True)
            process = ManagedProcess.start(daemon_command(inputs, data, arm), environment)
            transport = wait_ready(process, data, inputs.timeout)
            restarted = wait_baseline(transport, query, inputs.timeout, f"{tag}-restart")
            correctness = {**preflight, **lifecycle_probe(
                transport, query, inputs.timeout, tag,
            )}
            correctness["restart_exact"] = restart_cleanup and restarted == baseline
            seed_observer = dict.fromkeys(
                ("completed_calls", "completed_inputs", "active_token_count", "nonconforming_calls"), 0
            )
            before_load = normalized_load()
            start_import(transport, bulk_root)
            wait_saturated(observer, seed_observer, process, inputs.timeout)
            time.sleep(warmup)
            measured_before = read_observer(observer)
            samples = round(measurement * 2)
            latencies, successes, query_exact, measured_after = measured_queries(
                transport, query, baseline, samples, measurement, tag, observer
            )
            delta = observer_delta(measured_before, measured_after)
            if (delta["completed_calls"] <= 0
                    or delta["completed_inputs"] != delta["completed_calls"] * 4
                    or delta["active_token_count"] != delta["completed_inputs"] * 512
                    or delta["nonconforming_calls"] != 0):
                raise ExperimentError("measured_bulk_shape_failed")
            if observer_delta(seed_observer, measured_after)["completed_inputs"] >= sum(
                path.is_file() for path in bulk_root.iterdir()
            ):
                raise ExperimentError("bulk_not_saturated_for_full_window")
            memory = process_tree_peak_mib(process.process.pid)
            after_load = normalized_load()
            correctness["query_results_exact"] = query_exact
        finally:
            process.cleanup()
            cleanup_exact = not group_exists(process.group)
        correctness["cleanup_exact"] = cleanup_exact
        return SessionResult(
            block, arm, delta["completed_calls"], delta["completed_inputs"],
            delta["completed_inputs"] / measurement, latencies, successes, memory,
            correctness, abs(after_load - before_load) <= MAX_LOAD_DRIFT,
        )
def build_workload(base: Path, query: str, documents: int) -> tuple[Path, Path]:
    seed_root, bulk_root = base / "seed", base / "bulk"
    seed_root.mkdir(); bulk_root.mkdir()
    (seed_root / "seed-00000.txt").write_text(
        query + "\n" + passage(99_999), encoding="utf-8"
    )
    for index in range(1, 4):
        (seed_root / f"seed-{index:05d}.txt").write_text(
            passage(99_999 + index), encoding="utf-8"
        )
    for index in range(documents):
        (bulk_root / f"resume-{index:05d}.txt").write_text(passage(index), encoding="utf-8")
    return seed_root.resolve(), bulk_root.resolve()
def bootstrap_ci(values: list[float], seed: int) -> tuple[float, float]:
    if not values:
        raise ExperimentError("empty_paired_samples")
    if len(values) == 1:
        return values[0], values[0]
    generator, means = random.Random(seed), []
    for _ in range(10_000):
        means.append(sum(generator.choice(values) for _ in values) / len(values))
    return percentile(means, 0.025), percentile(means, 0.975)
def aggregate_arm(results: list[SessionResult], arm: str) -> dict[str, object]:
    sessions = [result for result in results if result.arm == arm]
    latencies = [value for result in sessions for value in result.latencies]
    topology, threads, residents = ARM_IDENTITY[arm]
    correctness = {
        key: all(result.correctness.get(key, False) for result in sessions)
        for key in (
            "vectors_elementwise_exact", "counts_exact", "order_exact", "cancellation_exact",
            "timeout_exact", "restart_exact", "ready_exact", "cleanup_exact",
            "query_results_exact",
        )
    }
    return {
        "topology": topology, "interactive_threads": 3, "bulk_threads": threads,
        "resident_count": residents, "sessions": len(sessions),
        "bulk": {
            "completed_batches": sum(result.batches for result in sessions),
            "completed_inputs": sum(result.inputs for result in sessions),
            "mean_throughput_inputs_per_second": sum(result.throughput for result in sessions) / len(sessions),
        },
        "interactive": {
            "samples": len(latencies), "successes": sum(result.successes for result in sessions),
            "failures": len(latencies) - sum(result.successes for result in sessions),
            "p50_ms": percentile(latencies, 0.50), "p95_ms": percentile(latencies, 0.95),
            "p99_ms": percentile(latencies, 0.99),
            "max_queue_wait_upper_bound_ms": max(latencies),
        },
        "resources": {"process_tree_private_or_anonymous_peak_mib": max(
            result.memory_mib for result in sessions
        )},
        "correctness": correctness,
    }
def comparison(results: list[SessionResult], arms: dict[str, object], candidate: str,
               kind: str, run_valid: bool) -> dict[str, object]:
    by_key = {(result.block, result.arm): result for result in results}
    blocks = sorted({result.block for result in results})
    paired = [
        (by_key[(block, candidate)].throughput / by_key[(block, ARMS[0])].throughput - 1) * 100
        for block in blocks if (block, candidate) in by_key and (block, ARMS[0]) in by_key
    ]
    improvement = sum(paired) / len(paired)
    low, high = bootstrap_ci(paired, SEED + ARMS.index(candidate))
    control_interactive = arms[ARMS[0]]["interactive"]  # type: ignore[index]
    candidate_arm = arms[candidate]  # type: ignore[index]
    interactive = candidate_arm["interactive"]  # type: ignore[index]
    p95_regression = (interactive["p95_ms"] / control_interactive["p95_ms"] - 1) * 100
    p99_regression = (interactive["p99_ms"] / control_interactive["p99_ms"] - 1) * 100
    correctness = all(arms[name]["interactive"]["failures"] == 0  # type: ignore[index]
                      and all(arms[name]["correctness"].values())  # type: ignore[index]
                      for name in (ARMS[0], candidate))
    gates = {
        "bulk_at_least_8_percent": improvement >= 8,
        "bulk_ci_positive": low > 0,
        "query_p95_within_5_percent": p95_regression <= 5,
        "query_p99_within_10_percent": p99_regression <= 10,
        "queue_wait_within_200_ms": interactive["max_queue_wait_upper_bound_ms"] <= 200,
        "resource_within_1536_mib": candidate_arm["resources"]["process_tree_private_or_anonymous_peak_mib"] <= 1536,  # type: ignore[index]
        "correctness_exact": correctness,
    }
    gates["accepted"] = kind == "formal_public_matrix" and run_valid and all(gates.values())
    return {
        "control": ARMS[0], "candidate": candidate, "paired_blocks": len(paired),
        "bulk_improvement_percent": improvement,
        "bulk_paired_ci95_low_percent": low, "bulk_paired_ci95_high_percent": high,
        "query_p95_regression_percent": p95_regression,
        "query_p99_regression_percent": p99_regression,
        "max_queue_wait_upper_bound_ms": interactive["max_queue_wait_upper_bound_ms"],
        "process_tree_private_or_anonymous_peak_mib": candidate_arm["resources"]["process_tree_private_or_anonymous_peak_mib"],  # type: ignore[index]
        "correctness_pass": correctness, "gates": gates,
    }
def build_report(inputs: Inputs, results: list[SessionResult], kind: str, blocks: int,
                 warmup: float, measurement: float, thermal: list[str]) -> dict[str, object]:
    arms = {arm: aggregate_arm(results, arm) for arm in ARMS}
    all_complete = len(results) == blocks * 3
    thermal_guard = bool(thermal) and not any(
        state in {"serious", "critical", "unknown"} for state in thermal
    )
    load_guard = all(result.load_guard for result in results)
    comparisons = [comparison(
        results, arms, candidate, kind, all_complete and thermal_guard and load_guard
    ) for candidate in ARMS[1:]]
    accepted = [item["candidate"] for item in comparisons if item["gates"]["accepted"]]
    all_correct = all(
        arm["interactive"]["failures"] == 0 and all(arm["correctness"].values())
        for arm in arms.values()
    )
    if kind == "smoke":
        decision = {"status": "smoke_pass" if all_complete and thermal_guard and load_guard and all_correct else "smoke_failed",
                    "winner": None, "private_matrix_eligible": False}
        claims = ["capability_only", "no_product_speedup", "no_private_claim", "no_release_claim"]
    else:
        status = "won" if all_complete and thermal_guard and load_guard and len(accepted) == 1 else (
            "lost" if all_complete and thermal_guard and load_guard and not accepted else "inconclusive"
        )
        decision = {"status": status, "winner": accepted[0] if status == "won" else None,
                    "private_matrix_eligible": status == "won"}
        claims = ["candidate_selection_only", "no_product_migration", "no_private_product_claim", "no_release_claim"]
    return {
        "schema_version": SCHEMA, "artifact_id": "resident-embedding-role-isolation-issue-319",
        "issue": "#319", "source": "public_synthetic_daemon_sessions",
        "revision": inputs.revision,
        "platform": {"os": "macos", "architecture": "arm64", "machine": "M4",
                     "governor": "H2_Aggressive",
                     "memory_measurement": "process_tree_private_or_anonymous_peak_mib"},
        "run": {"kind": kind, "seed": SEED, "blocks": blocks, "sessions": blocks * 3,
                "sessions_per_arm": blocks, "independent_release_daemon_sessions": True,
                "williams_balanced": True, "warmup_seconds": warmup,
                "measurement_seconds": measurement, "all_sessions_completed": all_complete,
                "thermal_guard_passed": thermal_guard, "host_load_guard_passed": load_guard},
        "fixed_workload": FIXED_WORKLOAD, "arms": arms, "comparisons": comparisons,
        "decision": decision, "privacy": PRIVACY, "claims": claims,
    }
def secure_write(path: Path, report: dict[str, object]) -> None:
    encoded = (json.dumps(report, allow_nan=False, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if len(encoded) > MAX_REPORT_BYTES or any(signal in encoded for signal in (b"/Users/", b"file://")):
        raise ExperimentError("public_report_boundary_failed")
    path.parent.mkdir(parents=True, exist_ok=True)
    staging = path.with_name(f".{path.name}.next")
    descriptor = os.open(staging, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(encoded)
        os.replace(staging, path)
        os.chmod(path, 0o600)
    finally:
        staging.unlink(missing_ok=True)
def attest(inputs: Inputs) -> None:
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise ExperimentError("platform_not_macos_arm64")
    chip = subprocess.run(
        ["sysctl", "-n", "machdep.cpu.brand_string"], text=True,
        capture_output=True, timeout=5,
    ).stdout
    if "M4" not in chip:
        raise ExperimentError("platform_not_m4")
    for path in (inputs.daemon, inputs.embedding, inputs.classifier):
        if not path.is_file():
            raise ExperimentError("required_input_missing")
    if not inputs.runtime_dir.is_dir() or re.fullmatch(r"[0-9a-f]{40}", inputs.revision) is None:
        raise ExperimentError("required_input_missing")
    head = subprocess.run(
        ["git", "-C", str(ROOT), "rev-parse", "HEAD"], check=True, text=True,
        capture_output=True, timeout=5,
    ).stdout.strip()
    dirty = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=no"],
        check=True, text=True, capture_output=True, timeout=5,
    ).stdout
    if head != inputs.revision or dirty:
        raise ExperimentError("revision_not_exact_clean_head")
def run_matrix(arguments: argparse.Namespace) -> int:
    inputs = Inputs(arguments.daemon_bin.resolve(), arguments.embedding_bin.resolve(),
                    arguments.embedding_runtime_dir.resolve(), arguments.classifier_model.resolve(),
                    arguments.revision, arguments.out.resolve(), arguments.timeout_seconds)
    attest(inputs)
    blocks, warmup, measurement, documents = (
        (1, 1.0, 1.0, 512) if arguments.command == "smoke"
        else (10, 30.0, 60.0, 4096)
    )
    schedule = (FORMAL_SCHEDULE[0],) if blocks == 1 else FORMAL_SCHEDULE
    monitor = SystemPressureMonitor()
    monitor.start()
    try:
        _, query, preflight = vector_preflight(inputs)
        with tempfile.TemporaryDirectory(prefix="resume-ir-role-isolation-workload-") as temporary:
            seed_root, bulk_root = build_workload(Path(temporary), query, documents)
            results = []
            for block, row in enumerate(schedule):
                for arm in row:
                    results.append(run_session(
                        inputs, block, arm, query, seed_root, bulk_root, preflight[arm],
                        warmup, measurement,
                    ))
    finally:
        monitor.stop()
    report = build_report(
        inputs, results, "smoke" if blocks == 1 else "formal_public_matrix",
        blocks, warmup, measurement, monitor.thermal_states,
    )
    secure_write(inputs.output, report)
    subprocess.run(
        [sys.executable, str(ROOT / "scripts/ci/check-resident-embedding-role-isolation.py"),
         str(inputs.output)], check=True,
    )
    print(json.dumps({"status": report["decision"]["status"], "report_bytes": inputs.output.stat().st_size}))  # type: ignore[index]
    return 0
def self_test() -> int:
    assert len(FORMAL_SCHEDULE) == 10
    assert all(set(row) == set(ARMS) for row in FORMAL_SCHEDULE)
    positions = [sum(row[index] == arm for row in FORMAL_SCHEDULE)
                 for arm in ARMS for index in range(3)]
    carry = [sum((left, right) in tuple(zip(row, row[1:])) for row in FORMAL_SCHEDULE)
             for left in ARMS for right in ARMS if left != right]
    assert max(positions) - min(positions) <= 1 and max(carry) - min(carry) <= 2
    assert percentile([3, 1, 2, 4], 0.50) == 2
    assert bootstrap_ci([8.5], SEED) == (8.5, 8.5)
    try:
        observer_delta({"counter": 2}, {"counter": 1})
    except ExperimentError:
        pass
    else:
        raise AssertionError("observer regression was accepted")
    print(json.dumps({"status": "self_test_pass", "checks": 4}))
    return 0
def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)
    subparsers.add_parser("self-test")
    for command in ("smoke", "formal"):
        sub = subparsers.add_parser(command)
        sub.add_argument("--daemon-bin", type=Path, required=True)
        sub.add_argument("--embedding-bin", type=Path, required=True)
        sub.add_argument("--embedding-runtime-dir", type=Path, required=True)
        sub.add_argument("--classifier-model", type=Path, required=True)
        sub.add_argument("--revision", required=True)
        sub.add_argument("--out", type=Path, required=True)
        sub.add_argument("--timeout-seconds", type=float, default=240.0)
    return result
def main() -> int:
    arguments = parser().parse_args()
    if arguments.command == "self-test":
        return self_test()
    if arguments.timeout_seconds <= 0:
        raise ExperimentError("timeout_invalid")
    return run_matrix(arguments)

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ExperimentError, OSError, subprocess.SubprocessError) as error:
        code = str(error) if isinstance(error, ExperimentError) else "experiment_subprocess_failed"
        print(json.dumps({"status": "blocked", "reason": code}), file=sys.stderr)
        raise SystemExit(2)
