#!/usr/bin/env python3
"""Run the frozen public #341 fixed-B4 resident-pool matrix."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import runpy
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from resident_embedding_pool_support import (
    ARM_IDENTITY,
    ARMS,
    CONTRACT,
    SEED,
    ExperimentError,
    HarnessError,
    Inputs,
    ManagedProcess,
    attest,
    bootstrap_ci,
    build_workload,
    child_restarted,
    counters,
    crash_bulk_child,
    delta,
    group_exists,
    lifecycle_probe,
    percentile,
    process_descendants,
    process_tree_peak_mib,
    queue_after,
    read_observer,
    secure_write,
    send_query,
    start_import,
    vector_preflight,
    wait_baseline,
    wait_observer_flush,
    wait_ready,
    wait_saturated,
    wait_seed,
)

ROOT = Path(__file__).resolve().parents[2]
PRESSURE = runpy.run_path(str(ROOT / "scripts/local/run-mixed-import-variance.py"))
SystemPressureMonitor = PRESSURE["SystemPressureMonitor"]
validate_report = CONTRACT["validate"]
IPC_PROTOCOL = "resume-ir.daemon-ipc.v5"
SCHEMA = "resume-ir.resident-embedding-pool.v1"
MAX_EXTERNAL_CPU_FRACTION = 0.25
OUTCOMES = tuple(CONTRACT["OUTCOMES"])
DEGRADED = set(CONTRACT["DEGRADED_OUTCOMES"])
FIXED_WORKLOAD = dict(CONTRACT["FIXED_WORKLOAD"])
PRIVACY = {key: False for key in CONTRACT["PRIVACY"]}
WILLIAMS = (
    ARMS,
    (ARMS[0], ARMS[2], ARMS[1]),
    (ARMS[1], ARMS[0], ARMS[2]),
    (ARMS[1], ARMS[2], ARMS[0]),
    (ARMS[2], ARMS[0], ARMS[1]),
    (ARMS[2], ARMS[1], ARMS[0]),
)
FORMAL_SCHEDULE = WILLIAMS + tuple(WILLIAMS[index] for index in (0, 1, 2, 5))


@dataclass
class SessionResult:
    block: int
    arm: str
    batches: int
    inputs: int
    throughput: float
    latencies: list[float]
    outcomes: Counter[str]
    queue_waits: list[float]
    memory_mib: float
    correctness: dict[str, bool]
    cleanup: bool


class HostLoadMonitor:
    """Reject three consecutive high-CPU samples outside the runner tree."""

    def __init__(self) -> None:
        self._invalid = False
        self._high_samples = 0
        self._stop = threading.Event()
        self._lock = threading.Lock()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self._sample()
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._thread.join(timeout=2)

    def passed(self) -> bool:
        with self._lock:
            return not self._invalid

    def _run(self) -> None:
        while not self._stop.wait(1):
            self._sample()

    def _sample(self) -> None:
        try:
            table = cpu_process_table()
            excluded = process_descendants(table, {os.getpid()})
            external = sum(cpu for pid, (_, cpu) in table.items() if pid not in excluded)
            high = (
                external / (100.0 * max(os.cpu_count() or 1, 1))
                > MAX_EXTERNAL_CPU_FRACTION
            )
        except ExperimentError:
            high = True
        with self._lock:
            self._high_samples = self._high_samples + 1 if high else 0
            self._invalid |= self._high_samples >= 3


def cpu_process_table() -> dict[int, tuple[int, float]]:
    try:
        output = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,%cpu="], check=True, text=True,
            capture_output=True, timeout=10,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        raise ExperimentError("host_load_measurement_failed") from None
    table = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) == 3:
            try:
                table[int(fields[0])] = (int(fields[1]), float(fields[2]))
            except ValueError:
                continue
    return table


def measured_queries(transport: object, query: str, expected: tuple[str, str],
                     samples: int, duration: float, tag: str, observer: Path,
                     before: dict[str, object], timeout: float
                     ) -> tuple[
                         list[float], Counter[str], list[float],
                         dict[str, object], dict[str, object],
                     ]:
    start, futures = time.monotonic(), []
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        for index in range(samples):
            target = start + index * 0.5
            time.sleep(max(0.0, target - time.monotonic()))
            futures.append(pool.submit(
                send_query, transport, query, f"measure-{tag}-{index}", expected
            ))
        time.sleep(max(0.0, start + duration - time.monotonic()))
        window_end = read_observer(observer)
        observations = []
        for future in futures:
            try:
                observations.append(future.result(timeout=15))
            except concurrent.futures.TimeoutError:
                observations.append(("transport_error", 15_000.0, None))
    flushed = wait_observer_flush(
        observer, counters(window_end, "bulk")["completed_calls"], timeout
    )
    waits = queue_after(counters(before, "interactive")["completed_calls"], flushed)
    return (
        [item[1] for item in observations],
        Counter(item[0] for item in observations),
        waits, window_end, flushed,
    )


def daemon_command(inputs: Inputs, data: Path, arm: str) -> list[str]:
    return [
        str(inputs.daemon), "--data-dir", str(data), "run", "--foreground",
        "--work-imports", "--work-index", "--expected-ipc-protocol", IPC_PROTOCOL,
        "--ipc-listen", "127.0.0.1:0", "--embedding-command", str(inputs.embedding),
        "--embedding-model-id", FIXED_WORKLOAD["model_id"],
        "--embedding-dimension", str(FIXED_WORKLOAD["dimension"]),
        "--embedding-timeout-ms", "60000",
        "--resume-classifier-model", str(inputs.classifier),
        "--resident-embedding-pool-arm", arm,
    ]


def run_session(inputs: Inputs, block: int, arm: str, query: str,
                seed_root: Path, bulk_root: Path, preflight: dict[str, bool],
                warmup: float, measurement: float) -> SessionResult:
    with tempfile.TemporaryDirectory(prefix="resume-ir-pool-session-") as temporary:
        base, data = Path(temporary), Path(temporary) / "data"
        data.mkdir()
        observer = base / "observer.json"
        environment = os.environ.copy()
        environment.update({
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(inputs.runtime_dir),
            "RESUME_IR_RESIDENT_EMBEDDING_POOL_OBSERVER": str(observer),
        })
        process = ManagedProcess.start(daemon_command(inputs, data, arm), environment)
        cleanup, correctness = False, dict(preflight)
        try:
            transport = wait_ready(process, data, inputs.timeout)
            tag = f"b{block}-{ARMS.index(arm)}"
            start_import(transport, seed_root)
            wait_seed(observer, process, inputs.timeout)
            baseline = wait_baseline(transport, query, inputs.timeout, tag)
            process.cleanup()
            restart_cleanup = not group_exists(process.group)
            observer.unlink(missing_ok=True)
            process = ManagedProcess.start(daemon_command(inputs, data, arm), environment)
            transport = wait_ready(process, data, inputs.timeout)
            restarted = wait_baseline(
                transport, query, inputs.timeout, f"{tag}-restart"
            )
            correctness.update(lifecycle_probe(
                transport, query, baseline, inputs.timeout, tag
            ))
            correctness["crash_restart_exact"] = (
                restart_cleanup and restarted == baseline
            )
            victim = crash_bulk_child(
                process, inputs.embedding, ARM_IDENTITY[arm][2]
            )
            post_crash, _, _ = send_query(
                transport, query, f"crash-query-{tag}", baseline
            )
            restarted = child_restarted(
                process, inputs.embedding, victim,
                ARM_IDENTITY[arm][2], inputs.timeout,
            )
            correctness["crash_restart_exact"] &= (
                post_crash == "exact_expected" and restarted
            )
            start_import(transport, bulk_root)
            wait_saturated(observer, process, inputs.timeout)
            time.sleep(warmup)
            before = read_observer(observer)
            samples = round(measurement * 2)
            latencies, outcomes, waits, window_end, flushed = measured_queries(
                transport, query, baseline, samples, measurement, tag,
                observer, before, inputs.timeout,
            )
            bulk = delta(counters(before, "bulk"), counters(window_end, "bulk"))
            interactive = delta(
                counters(before, "interactive"), counters(flushed, "interactive")
            )
            bulk_shape = (
                bulk["completed_calls"] > 0
                and bulk["completed_inputs"] == bulk["completed_calls"] * 4
                and bulk["active_token_count"] == bulk["completed_inputs"] * 512
                and bulk["nonconforming_calls"] == 0
            )
            interactive_shape = (
                interactive["completed_inputs"] == interactive["completed_calls"]
                and interactive["active_token_count"] == interactive["completed_inputs"] * 32
                and interactive["nonconforming_calls"] == 0
                and interactive["completed_calls"] == samples
                and len(waits) == samples
            )
            if not bulk_shape:
                raise ExperimentError("measured_bulk_shape_failed")
            if counters(window_end, "bulk")["completed_inputs"] >= sum(
                path.is_file() for path in bulk_root.iterdir()
            ):
                raise ExperimentError("bulk_not_saturated_for_full_window")
            correctness["complete_batch_grouping_exact"] &= bulk_shape
            correctness["publication_atomicity_exact"] = not any(
                outcomes[key] for key in DEGRADED
            )
            correctness["query_outcomes_exact"] = (
                not any(outcomes[key] for key in DEGRADED) and interactive_shape
            )
            memory = process_tree_peak_mib(process.process.pid)
        finally:
            process.cleanup()
            cleanup = not group_exists(process.group)
        correctness["cleanup_exact"] = cleanup
        return SessionResult(
            block, arm, bulk["completed_calls"], bulk["completed_inputs"],
            bulk["completed_inputs"] / measurement, latencies, outcomes,
            waits, memory, correctness, cleanup,
        )


def aggregate_arm(results: list[SessionResult], arm: str) -> dict[str, Any]:
    sessions = [result for result in results if result.arm == arm]
    latencies = [value for result in sessions for value in result.latencies]
    waits = [value for result in sessions for value in result.queue_waits]
    outcomes: Counter[str] = Counter()
    for result in sessions:
        outcomes.update(result.outcomes)
    threads, bulk_residents, residents = ARM_IDENTITY[arm]
    correctness = {
        key: all(result.correctness.get(key, False) for result in sessions)
        for key in CONTRACT["CORRECTNESS"]
    }
    return {
        "topology": "interactive_plus_bulk_pool",
        "interactive_threads": 3, "bulk_threads": threads,
        "bulk_resident_count": bulk_residents, "resident_count": residents,
        "sessions": len(sessions),
        "bulk": {
            "completed_batches": sum(result.batches for result in sessions),
            "completed_inputs": sum(result.inputs for result in sessions),
            "mean_throughput_inputs_per_second": (
                sum(result.throughput for result in sessions) / len(sessions)
            ),
        },
        "interactive": {
            "samples": len(latencies),
            "outcomes": {key: outcomes[key] for key in OUTCOMES},
            "p50_ms": percentile(latencies, 0.50),
            "p95_ms": percentile(latencies, 0.95),
            "p99_ms": percentile(latencies, 0.99),
            "max_resident_queue_wait_ms": max(waits, default=0.0),
        },
        "resources": {
            "process_tree_private_or_anonymous_peak_mib": max(
                result.memory_mib for result in sessions
            )
        },
        "correctness": correctness,
    }


def comparison(results: list[SessionResult], arms: dict[str, dict[str, Any]],
               candidate: str, kind: str, run_valid: bool) -> dict[str, Any]:
    by_key = {(result.block, result.arm): result for result in results}
    blocks = sorted({result.block for result in results})
    paired = [
        (
            by_key[(block, candidate)].throughput
            / by_key[(block, ARMS[0])].throughput - 1
        ) * 100
        for block in blocks
    ]
    improvement = sum(paired) / len(paired)
    low, high = bootstrap_ci(paired, SEED + ARMS.index(candidate))
    control = arms[ARMS[0]]
    selected = arms[candidate]
    control_query, selected_query = control["interactive"], selected["interactive"]
    p95 = (selected_query["p95_ms"] / control_query["p95_ms"] - 1) * 100
    p99 = (selected_query["p99_ms"] / control_query["p99_ms"] - 1) * 100
    queue = max(
        control_query["max_resident_queue_wait_ms"],
        selected_query["max_resident_queue_wait_ms"],
    )
    memory = max(
        control["resources"]["process_tree_private_or_anonymous_peak_mib"],
        selected["resources"]["process_tree_private_or_anonymous_peak_mib"],
    )
    outcome_guard = all(
        arms[name]["interactive"]["outcomes"][key] == 0  # type: ignore[index]
        for name in (ARMS[0], candidate) for key in DEGRADED
    )
    correctness = all(
        all(arms[name]["correctness"].values())  # type: ignore[index]
        for name in (ARMS[0], candidate)
    )
    gates = {
        "bulk_at_least_15_percent": improvement >= 15,
        "bulk_ci_positive": low > 0,
        "query_p95_within_5_percent": p95 <= 5,
        "query_p99_within_10_percent": p99 <= 10,
        "direct_queue_wait_within_200_ms": queue <= 200,
        "resource_within_1536_mib": memory <= 1536,
        "outcomes_exact": outcome_guard,
        "correctness_exact": correctness,
    }
    gates["accepted"] = (
        kind == "formal_public_matrix" and run_valid and all(gates.values())
    )
    return {
        "control": ARMS[0], "candidate": candidate,
        "paired_blocks": len(paired), "bulk_improvement_percent": improvement,
        "bulk_paired_ci95_low_percent": low,
        "bulk_paired_ci95_high_percent": high,
        "query_p95_regression_percent": p95,
        "query_p99_regression_percent": p99,
        "max_interactive_resident_queue_wait_ms": queue,
        "max_process_tree_private_or_anonymous_peak_mib": memory,
        "outcome_guard_pass": outcome_guard,
        "correctness_pass": correctness, "gates": gates,
    }


def build_report(inputs: Inputs, results: list[SessionResult], kind: str,
                 blocks: int, quiet: float, warmup: float, measurement: float,
                 thermal: list[str], host_passed: bool) -> dict[str, Any]:
    arms = {arm: aggregate_arm(results, arm) for arm in ARMS}
    complete = len(results) == blocks * len(ARMS)
    thermal_passed = bool(thermal) and not any(
        state in {"serious", "critical", "unknown"} for state in thermal
    )
    cleanup = all(result.cleanup for result in results)
    run_valid = complete and thermal_passed and host_passed and cleanup
    comparisons = [
        comparison(results, arms, candidate, kind, run_valid)
        for candidate in ARMS[1:]
    ]
    accepted = [
        item["candidate"] for item in comparisons if item["gates"]["accepted"]
    ]
    if kind == "smoke":
        passed = run_valid and all(
            all(arm["correctness"].values())
            and all(arm["interactive"]["outcomes"][key] == 0 for key in DEGRADED)
            for arm in arms.values()
        )
        decision = {
            "status": "smoke_pass" if passed else "smoke_failed",
            "winner": None, "private_matrix_eligible": False,
        }
        claims = [
            "capability_only", "no_product_speedup",
            "no_private_claim", "no_release_claim",
        ]
    else:
        status = (
            "inconclusive" if not run_valid or len(accepted) > 1
            else "lost" if not accepted else "won"
        )
        decision = {
            "status": status,
            "winner": accepted[0] if status == "won" else None,
            "private_matrix_eligible": status == "won",
        }
        claims = [
            "candidate_selection_only", "no_product_migration",
            "no_private_product_claim", "no_release_claim",
        ]
    return {
        "schema_version": SCHEMA,
        "artifact_id": "resident-embedding-pool-issue-341",
        "issue": "#341", "source": "public_synthetic_daemon_sessions",
        "revision": inputs.revision,
        "platform": {
            "os": "macos", "architecture": "arm64", "machine": "M4",
            "governor": "H2_Aggressive",
            "memory_measurement": "process_tree_private_or_anonymous_peak_mib",
        },
        "run": {
            "kind": kind, "seed": SEED, "quiet_preflight_seconds": quiet,
            "blocks": blocks, "sessions": blocks * len(ARMS),
            "sessions_per_arm": blocks,
            "independent_release_daemon_sessions": True,
            "williams_balanced": True, "warmup_seconds": warmup,
            "measurement_seconds": measurement,
            "all_sessions_completed": complete,
            "thermal_guard_passed": thermal_passed,
            "host_load_guard_passed": host_passed,
            "process_cleanup_passed": cleanup,
        },
        "fixed_workload": FIXED_WORKLOAD, "arms": arms,
        "comparisons": comparisons, "decision": decision,
        "privacy": PRIVACY, "claims": claims,
    }


def guards_passed(pressure: object, host: HostLoadMonitor) -> bool:
    thermal = pressure.thermal_states  # type: ignore[attr-defined]
    return (
        bool(thermal)
        and not any(state in {"serious", "critical", "unknown"} for state in thermal)
        and host.passed()
    )


def run_matrix(arguments: argparse.Namespace) -> int:
    inputs = Inputs(
        arguments.daemon_bin.resolve(), arguments.embedding_bin.resolve(),
        arguments.embedding_runtime_dir.resolve(), arguments.classifier_model.resolve(),
        arguments.revision, arguments.out.resolve(), arguments.timeout_seconds,
    )
    attest(inputs)
    blocks, quiet, warmup, measurement, documents = (
        (1, 1.0, 1.0, 1.0, 512)
        if arguments.command == "smoke"
        else (10, 120.0, 30.0, 60.0, 4096)
    )
    schedule = (FORMAL_SCHEDULE[0],) if blocks == 1 else FORMAL_SCHEDULE
    pressure, host = SystemPressureMonitor(), HostLoadMonitor()
    pressure.start()
    host.start()
    try:
        time.sleep(quiet)
        if not guards_passed(pressure, host):
            raise ExperimentError("quiet_host_preflight_failed")
        query, preflight = vector_preflight(inputs)
        with tempfile.TemporaryDirectory(prefix="resume-ir-pool-workload-") as temporary:
            seed, bulk = build_workload(Path(temporary), query, documents)
            results = []
            for block, row in enumerate(schedule):
                for arm in row:
                    results.append(run_session(
                        inputs, block, arm, query, seed, bulk, preflight[arm],
                        warmup, measurement,
                    ))
                    if not guards_passed(pressure, host):
                        raise ExperimentError("matrix_host_or_thermal_guard_failed")
    finally:
        host.stop()
        pressure.stop()
    report = build_report(
        inputs, results,
        "smoke" if blocks == 1 else "formal_public_matrix",
        blocks, quiet, warmup, measurement, pressure.thermal_states, host.passed(),
    )
    validate_report(report)
    secure_write(inputs.output, report)
    print(json.dumps({
        "status": report["decision"]["status"],
        "report_bytes": inputs.output.stat().st_size,
    }, separators=(",", ":")))
    return 0


def self_test() -> int:
    support = runpy.run_path(str(ROOT / "scripts/local/resident_embedding_pool_support.py"))
    assert support["self_test"]() == 0
    assert len(FORMAL_SCHEDULE) == 10
    assert all(set(row) == set(ARMS) for row in FORMAL_SCHEDULE)
    positions = [
        sum(row[position] == arm for row in FORMAL_SCHEDULE)
        for arm in ARMS for position in range(3)
    ]
    assert max(positions) - min(positions) <= 1
    validate_report(json.loads(
        (ROOT / "perf/fixtures/resident-embedding-pool/valid-public-report.json").read_text()
    ))
    print(json.dumps({"status": "self_test_pass", "checks": 4}, separators=(",", ":")))
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("self-test")
    for command in ("smoke", "formal"):
        sub = commands.add_parser(command)
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
    except (
        ExperimentError, HarnessError, OSError, ValueError,
        json.JSONDecodeError, subprocess.SubprocessError,
    ) as error:
        reason = (
            str(error)
            if isinstance(error, (ExperimentError, HarnessError))
            else "experiment_failed"
        )
        print(json.dumps({"status": "blocked", "reason": reason}), file=sys.stderr)
        raise SystemExit(2)
