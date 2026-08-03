#!/usr/bin/env python3
"""Support the frozen public #341 fixed-B4 resident-pool matrix."""

from __future__ import annotations

import concurrent.futures
import json
import os
import platform
import random
import re
import runpy
import signal
import stat
import subprocess
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OCR = runpy.run_path(str(ROOT / "scripts/local/ocr-on-import-attribution.py"))
STREAM = runpy.run_path(str(ROOT / "scripts/local/embedding-batch-benchmark.py"))
RESOURCE = runpy.run_path(str(ROOT / "scripts/local/embedding-prepacking-benchmark.py"))
CONTRACT = runpy.run_path(str(ROOT / "scripts/ci/check-resident-embedding-pool.py"))
HttpTransport = OCR["HttpTransport"]
ManagedProcess = OCR["ManagedProcess"]
HarnessError = OCR["HarnessError"]
group_exists = OCR["group_exists"]
read_frame = STREAM["read_frame"]
write_frame = STREAM["write_frame"]
percentile = STREAM["percentile"]
physical_footprint = RESOURCE["physical_footprint"]

OBSERVER_SCHEMA = "resume-ir.resident-embedding-pool-observer.v1"
STREAM_SCHEMA = "resume-ir.embedding-stream.v1"
MODEL_ID = "intfloat-multilingual-e5-small-qint8-r1"
DIMENSION, SEED, MAX_REPORT_BYTES = 384, 20260803, 64 * 1024
ARMS = tuple(CONTRACT["ARMS"])
ARM_IDENTITY = {
    "i3_bulk1x4_b4": (4, 1, 2),
    "i3_bulk2x2_b4": (2, 2, 3),
    "i3_bulk2x3_b4": (3, 2, 3),
}


class ExperimentError(RuntimeError):
    """A fixed public-safe experiment failure code."""


@dataclass(frozen=True)
class Inputs:
    daemon: Path
    embedding: Path
    runtime_dir: Path
    classifier: Path
    revision: str
    output: Path
    timeout: float


class Resident:
    def __init__(self, inputs: Inputs, threads: int):
        environment = os.environ.copy()
        environment.update({
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(inputs.runtime_dir),
            "RESUME_IR_EMBEDDING_MODEL_ID": MODEL_ID,
            "RESUME_IR_EMBEDDING_DIMENSION": str(DIMENSION),
            "RESUME_IR_EMBEDDING_INTRA_THREADS": str(threads),
        })
        try:
            self.process = subprocess.Popen(
                [str(inputs.embedding), "--resident-embedding-pool-experiment"],
                env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, start_new_session=True,
            )
        except OSError:
            raise ExperimentError("resident_start_failed") from None
        self.timeout, self.request_id = inputs.timeout, 0
        if self.process.stdin is None or self.process.stdout is None:
            self.close()
            raise ExperimentError("resident_pipes_missing")
        ready = read_frame(self.process.stdout, self.timeout)
        if ready != {
            "type": "ready", "schema_version": STREAM_SCHEMA,
            "model_id": MODEL_ID, "dimension": DIMENSION,
        }:
            self.close()
            raise ExperimentError("resident_ready_invalid")

    def embed(self, texts: list[str], role: str) -> tuple[list[list[float]], int]:
        self.request_id += 1
        write_frame(self.process.stdin, {
            "schema_version": STREAM_SCHEMA, "request_id": self.request_id,
            "model_id": MODEL_ID, "dimension": DIMENSION,
            "inputs": [{"role": role, "text": text} for text in texts],
        })
        response = read_frame(self.process.stdout, self.timeout)
        vectors, telemetry = response.get("vectors"), response.get("telemetry")
        if (
            response.get("type") != "result"
            or response.get("request_id") != self.request_id
            or not isinstance(vectors, list)
            or len(vectors) != len(texts)
            or any(not isinstance(vector, list) or len(vector) != DIMENSION for vector in vectors)
            or not isinstance(telemetry, dict)
            or not isinstance(telemetry.get("active_token_count"), int)
        ):
            raise ExperimentError("resident_result_invalid")
        return vectors, telemetry["active_token_count"]

    def close(self) -> None:
        if self.process.poll() is None:
            if self.process.stdin is not None:
                self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()

    def __enter__(self) -> Resident:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


def passage(index: int) -> str:
    head = (
        f"Synthetic Candidate {index:05d}\nProfessional Summary\n"
        "Software engineer building deterministic local systems.\nWork Experience\n"
        "Developed reliable Rust and Python services with measured performance.\n"
        "Education\nSynthetic Institute\nSkills\nRust Python SQL systems performance\n"
    )
    return head + ("throughputdocument scheduling isolation benchmark evidence " * 180)


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


def vector_preflight(inputs: Inputs) -> tuple[str, dict[str, dict[str, bool]]]:
    texts = [passage(index) for index in range(8)]
    references: list[list[float]] | None = None
    query: str | None = None
    results = {}
    for arm in ARMS:
        threads = ARM_IDENTITY[arm][0]
        with Resident(inputs, threads) as resident:
            first, first_active = resident.embed(texts[:4], "passage")
            second, second_active = resident.embed(texts[4:], "passage")
            if query is None:
                query = calibrate_query(resident)
            _, query_active = resident.embed([query], "query")
        vectors = first + second
        if first_active != 4 * 512 or second_active != 4 * 512 or query_active != 32:
            raise ExperimentError("frozen_token_shape_failed")
        if references is None:
            references = vectors
        results[arm] = {
            "vectors_elementwise_exact": vectors == references,
            "complete_batch_grouping_exact": len(first) == len(second) == 4,
            "counts_exact": len(vectors) == 8,
            "order_exact": all(vector == references[index] for index, vector in enumerate(vectors)),
        }
    if query is None or not all(all(values.values()) for values in results.values()):
        raise ExperimentError("vector_preflight_failed")
    return query, results


def read_small_json(path: Path) -> dict[str, object]:
    body = path.read_bytes()
    if len(body) > MAX_REPORT_BYTES:
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
        except (FileNotFoundError, json.JSONDecodeError, HarnessError):
            time.sleep(0.05)
            continue
        runtimes, capabilities = status.get("optional_runtimes"), status.get("capabilities")
        runtime_ready = isinstance(runtimes, dict) and all(
            isinstance(runtimes.get(key), dict) and runtimes[key].get("state") == "available"
            for key in ("embedding", "classifier")
        )
        capability_ready = isinstance(capabilities, dict) and all(
            isinstance(capabilities.get(key), dict)
            and capabilities[key].get("state") == "available"
            for key in (
                "text_import", "index_publication", "keyword_search",
                "semantic_search", "hybrid_search",
            )
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
    tasks = body.get("task_ids")
    if (
        status != 202
        or body.get("schema_version") != "daemon.import.v1"
        or body.get("status") != "accepted"
        or body.get("accepted_roots") != 1
        or not isinstance(tasks, list)
        or len(tasks) != 1
        or not isinstance(tasks[0], str)
    ):
        raise ExperimentError("import_not_accepted")


def query_request(query: str, request_id: str, deadline_ms: int = 10_000,
                  cancel_token: str | None = None) -> dict[str, object]:
    request = {
        "schema_version": "resume-ir.ipc-request.v3",
        "request_id": request_id, "client_capability": "benchmark",
        "deadline_ms": deadline_ms,
        "payload": {"query": query, "mode": "hybrid", "top_k": 1, "filters": {}},
    }
    if cancel_token is not None:
        request["cancel_token"] = cancel_token
    return request


def classify_query(status: int, body: dict[str, object], request_id: str,
                   expected: tuple[str, str] | None) -> tuple[str, tuple[str, str] | None]:
    error = body.get("error")
    if status == 503 and isinstance(error, dict) and error.get("code") == "OVERLOADED":
        return "overload", None
    if status != 200:
        return "http_error", None
    required = {
        "schema_version", "request_id", "status", "visible_epoch", "query_mode",
        "partial", "partial_reasons", "latency_ms", "stage_latency_ms",
        "search_index", "result_count", "results",
    }
    reasons, results, epoch = (
        body.get("partial_reasons"), body.get("results"), body.get("visible_epoch")
    )
    if (
        set(body) != required
        or body.get("schema_version") != "resume-ir.search-response.v3"
        or body.get("request_id") != request_id
        or body.get("query_mode") != "hybrid"
        or not isinstance(epoch, int)
        or isinstance(epoch, bool)
        or epoch <= 0
        or not isinstance(reasons, list)
        or any(reason not in {"deadline_exceeded", "embedding_runtime_unavailable"} for reason in reasons)
        or not isinstance(results, list)
        or body.get("result_count") != len(results)
    ):
        return "protocol_error", None
    if body.get("status") == "cancelled":
        return "cancelled", None
    if (
        body.get("status") != "ok"
        or body.get("search_index") != "available"
        or body.get("partial") is not bool(reasons)
    ):
        return "protocol_error", None
    if "deadline_exceeded" in reasons:
        return "deadline_partial", None
    if "embedding_runtime_unavailable" in reasons:
        return "semantic_partial", None
    if reasons or len(results) != 1:
        return "protocol_error", None
    result = results[0]
    selection = result.get("selection") if isinstance(result, dict) else None
    if (
        not isinstance(result, dict)
        or result.get("rank") != 1
        or not isinstance(selection, dict)
        or selection.get("visible_epoch") != epoch
        or not isinstance(selection.get("doc_id"), str)
        or not isinstance(selection.get("version_id"), str)
    ):
        return "protocol_error", None
    signature = (selection["doc_id"], selection["version_id"])
    if expected is not None and signature != expected:
        # The frozen bulk corpus never contains the calibrated anchor term.
        return "protocol_error", signature
    return "exact_expected", signature


def send_query(transport: object, query: str, request_id: str,
               expected: tuple[str, str] | None, deadline_ms: int = 10_000,
               cancel_token: str | None = None) -> tuple[str, float, tuple[str, str] | None]:
    started = time.perf_counter()
    try:
        status, body = transport.post_json(  # type: ignore[attr-defined]
            "/search", query_request(query, request_id, deadline_ms, cancel_token)
        )
    except HarnessError as error:
        elapsed = max((time.perf_counter() - started) * 1_000.0, 0.001)
        outcome = "transport_error" if str(error) == "ipc_request_failed" else "protocol_error"
        return outcome, elapsed, None
    elapsed = max((time.perf_counter() - started) * 1_000.0, 0.001)
    outcome, signature = classify_query(status, body, request_id, expected)
    return outcome, elapsed, signature


def wait_baseline(transport: object, query: str, timeout: float,
                  tag: str) -> tuple[str, str]:
    deadline, attempt = time.monotonic() + timeout, 0
    while time.monotonic() < deadline:
        status = transport.get_json("/status")  # type: ignore[attr-defined]
        if (
            status.get("searchable_documents", 0) < 1
            or status.get("indexed_documents", 0) < 1
            or status.get("index_health") != "ready"
        ):
            time.sleep(0.1)
            continue
        attempt += 1
        outcome, _, signature = send_query(
            transport, query, f"baseline-{tag}-{attempt}", None
        )
        if outcome == "exact_expected" and signature is not None:
            return signature
        time.sleep(0.2)
    raise ExperimentError("query_seed_not_searchable")


def lifecycle_probe(transport: object, query: str, expected: tuple[str, str],
                    timeout: float, tag: str) -> dict[str, bool]:
    timeout_id = f"timeout-{tag}"
    timeout_outcome, _, _ = send_query(
        transport, query, timeout_id, expected, deadline_ms=1
    )
    search_id, cancel_id, token = (
        f"cancel-search-{tag}", f"cancel-{tag}", f"token-{tag}"
    )
    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(
            send_query, transport, query, search_id, expected, 10_000, token
        )
        time.sleep(0.005)
        cancel_status, cancel_body = transport.post_json("/search/cancel", {  # type: ignore[attr-defined]
            "schema_version": "resume-ir.search-cancel-request.v1",
            "request_id": cancel_id, "cancel_token": token,
        })
        search_outcome, _, _ = future.result(timeout=timeout)
    cancellation = (
        cancel_status == 200
        and cancel_body.get("schema_version") == "resume-ir.search-cancel-response.v1"
        and cancel_body.get("request_id") == cancel_id
        and cancel_body.get("status") in {"cancelled", "cancel_requested", "complete"}
        and search_outcome in {"exact_expected", "cancelled"}
    )
    return {
        "cancellation_exact": cancellation,
        "timeout_exact": timeout_outcome == "deadline_partial",
        "ready_exact": True,
    }


def process_descendants(
    table: Mapping[int, tuple[int, object]], roots: set[int]
) -> set[int]:
    result, frontier = set(roots), list(roots)
    while frontier:
        parent = frontier.pop()
        children = [pid for pid, (ppid, _) in table.items() if ppid == parent]
        result.update(children)
        frontier.extend(children)
    return result


def process_commands() -> dict[int, tuple[int, str]]:
    try:
        output = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,command="], check=True, text=True,
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


def embedding_children(root: int, binary: Path) -> list[int]:
    table = process_commands()
    members = process_descendants(table, {root})
    expected = binary.name
    return sorted(
        pid for pid in members - {root}
        if Path(table[pid][1].split()[0]).name == expected
    )


def process_tree_peak_mib(root: int) -> float:
    table = process_commands()
    members = process_descendants(table, {root})
    try:
        peak = sum(physical_footprint(pid)[1] for pid in members)
    except RuntimeError:
        raise ExperimentError("memory_measurement_failed") from None
    return peak / (1024 * 1024)


def read_observer(path: Path, timeout: float = 5.0) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            value = read_small_json(path)
        except (FileNotFoundError, json.JSONDecodeError):
            time.sleep(0.02)
            continue
        if (
            set(value) != {"schema_version", "bulk", "interactive"}
            or value.get("schema_version") != OBSERVER_SCHEMA
            or stat.S_IMODE(path.stat().st_mode) != 0o600
        ):
            raise ExperimentError("observer_invalid")
        bulk, interactive = value["bulk"], value["interactive"]
        if (
            not isinstance(bulk, dict)
            or set(bulk) != {
                "completed_calls", "completed_inputs",
                "active_token_count", "nonconforming_calls",
            }
            or not isinstance(interactive, dict)
            or set(interactive) != {
                "completed_calls", "completed_inputs", "active_token_count",
                "nonconforming_calls", "first_retained_sequence", "queue_wait_us",
            }
        ):
            raise ExperimentError("observer_invalid")
        numbers = [*bulk.values(), *(
            interactive[key] for key in interactive if key != "queue_wait_us"
        )]
        queue = interactive["queue_wait_us"]
        if (
            any(not isinstance(item, int) or isinstance(item, bool) or item < 0 for item in numbers)
            or not isinstance(queue, list)
            or len(queue) > 512
            or any(not isinstance(item, int) or isinstance(item, bool) or item < 0 for item in queue)
        ):
            raise ExperimentError("observer_invalid")
        calls, first = interactive["completed_calls"], interactive["first_retained_sequence"]
        expected_first = calls - len(queue) + 1 if queue else 0
        if first != expected_first:
            raise ExperimentError("observer_invalid")
        return value
    raise ExperimentError("observer_unavailable")


def counters(value: dict[str, object], role: str) -> dict[str, int]:
    return value[role]  # type: ignore[return-value]


def delta(before: dict[str, int], after: dict[str, int]) -> dict[str, int]:
    result = {key: after[key] - before[key] for key in before}
    if any(value < 0 for value in result.values()):
        raise ExperimentError("observer_counter_regressed")
    return result


def queue_after(sequence: int, observer: Mapping[str, object]) -> list[float]:
    interactive = observer["interactive"]  # type: ignore[assignment]
    first = interactive["first_retained_sequence"]  # type: ignore[index]
    values = interactive["queue_wait_us"]  # type: ignore[index]
    return [
        values[index] / 1_000.0
        for index in range(len(values))
        if first + index > sequence
    ]


def wait_seed(path: Path, process: object, timeout: float) -> None:
    expected = {
        "completed_calls": 2, "completed_inputs": 4,
        "active_token_count": 4 * 512, "nonconforming_calls": 2,
    }
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.process.poll() is not None:  # type: ignore[attr-defined]
            raise ExperimentError("daemon_exited_during_seed")
        if path.is_file():
            observed = counters(read_observer(path), "bulk")
            if observed == expected:
                return
            if any(observed[key] > expected[key] for key in expected):
                raise ExperimentError("seed_publication_shape_failed")
        time.sleep(0.05)
    raise ExperimentError("seed_publication_timeout")


def wait_saturated(path: Path, process: object, timeout: float) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.process.poll() is not None:  # type: ignore[attr-defined]
            raise ExperimentError("daemon_exited_during_import")
        if path.is_file():
            observed = read_observer(path)
            bulk = counters(observed, "bulk")
            if bulk["nonconforming_calls"] > 2:
                raise ExperimentError("bulk_saturation_shape_failed")
            if bulk["completed_inputs"] >= 104 and bulk["nonconforming_calls"] == 2:
                return observed
        time.sleep(0.05)
    raise ExperimentError("bulk_saturation_timeout")


def wait_observer_flush(path: Path, completed_bulk: int, timeout: float) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        observed = read_observer(path)
        if counters(observed, "bulk")["completed_calls"] > completed_bulk:
            return observed
        time.sleep(0.02)
    raise ExperimentError("observer_flush_timeout")


def crash_bulk_child(process: object, embedding: Path, expected_count: int) -> int:
    children = embedding_children(process.process.pid, embedding)  # type: ignore[attr-defined]
    if len(children) != expected_count:
        raise ExperimentError("resident_process_count_invalid")
    victim = children[-1]
    os.kill(victim, signal.SIGKILL)
    return victim


def child_restarted(process: object, embedding: Path, victim: int,
                    expected_count: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        children = embedding_children(process.process.pid, embedding)  # type: ignore[attr-defined]
        if victim not in children and len(children) == expected_count:
            return True
        time.sleep(0.05)
    return False




def build_workload(base: Path, query: str, documents: int) -> tuple[Path, Path]:
    seed, bulk = base / "seed", base / "bulk"
    seed.mkdir()
    bulk.mkdir()
    (seed / "seed-00000.txt").write_text(
        query + "\n" + passage(99_999), encoding="utf-8"
    )
    for index in range(1, 4):
        (seed / f"seed-{index:05d}.txt").write_text(
            passage(99_999 + index), encoding="utf-8"
        )
    for index in range(documents):
        (bulk / f"resume-{index:05d}.txt").write_text(
            passage(index), encoding="utf-8"
        )
    return seed.resolve(), bulk.resolve()


def bootstrap_ci(values: list[float], seed: int) -> tuple[float, float]:
    if not values:
        raise ExperimentError("empty_paired_samples")
    if len(values) == 1:
        return values[0], values[0]
    generator, means = random.Random(seed), []
    for _ in range(10_000):
        means.append(sum(generator.choice(values) for _ in values) / len(values))
    return percentile(means, 0.025), percentile(means, 0.975)




def secure_write(path: Path, report: dict[str, object]) -> None:
    encoded = (
        json.dumps(report, allow_nan=False, sort_keys=True, separators=(",", ":")) + "\n"
    ).encode()
    if (
        len(encoded) > MAX_REPORT_BYTES
        or any(signal_value in encoded for signal_value in (b"/Users/", b"file://", b"Bearer "))
    ):
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
    if (
        not inputs.runtime_dir.is_dir()
        or re.fullmatch(r"[0-9a-f]{40}", inputs.revision) is None
        or "release" not in inputs.daemon.parts
        or "release" not in inputs.embedding.parts
    ):
        raise ExperimentError("required_input_missing")
    head = subprocess.run(
        ["git", "-C", str(ROOT), "rev-parse", "HEAD"], check=True,
        text=True, capture_output=True, timeout=5,
    ).stdout.strip()
    dirty = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=no"],
        check=True, text=True, capture_output=True, timeout=5,
    ).stdout
    if head != inputs.revision or dirty:
        raise ExperimentError("revision_not_exact_clean_head")




def valid_probe(request_id: str, signature: tuple[str, str]) -> dict[str, object]:
    return {
        "schema_version": "resume-ir.search-response.v3", "request_id": request_id,
        "status": "ok", "visible_epoch": 7, "query_mode": "hybrid",
        "partial": False, "partial_reasons": [], "latency_ms": 2.0,
        "stage_latency_ms": {
            key: 0.1 for key in (
                "parse", "prefilter", "bm25", "ann",
                "fusion", "bulk_hydrate", "snippet",
            )
        },
        "search_index": "available", "result_count": 1,
        "results": [{
            "rank": 1, "file_name": "synthetic", "snippet": "synthetic",
            "selection": {
                "doc_id": signature[0], "version_id": signature[1],
                "visible_epoch": 7,
            },
        }],
    }


def self_test() -> int:
    expected = ("d", "v")
    assert classify_query(200, valid_probe("r", expected), "r", expected)[0] == "exact_expected"
    assert classify_query(200, valid_probe("r", ("x", "y")), "r", expected)[0] == "protocol_error"
    assert bootstrap_ci([15.5], SEED) == (15.5, 15.5)
    synthetic: dict[str, object] = {
        "interactive": {
            "completed_calls": 4, "first_retained_sequence": 2,
            "queue_wait_us": [1000, 2000, 3000],
        }
    }
    assert queue_after(2, synthetic) == [2.0, 3.0]
    print(json.dumps({"status": "self_test_pass", "checks": 4}, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(self_test())
