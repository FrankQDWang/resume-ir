#!/usr/bin/env python3
"""Run a private mixed-import variance experiment with bounded macOS telemetry."""

from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import pathlib
import re
import secrets
import statistics
import subprocess
import sys
import threading
import time
from typing import Any


MIN_WARMUP_SECONDS = 30
MIN_FORMAL_REPEATS = 5
EXTERNAL_CPU_CAPACITY_FRACTION = 0.20
SUSTAINED_SAMPLE_FRACTION = 0.5
MAX_PUBLIC_RUNS = 16
EXPERIMENT_ID = re.compile(r"[a-z0-9][a-z0-9._-]{0,63}\Z")
TIME_FIELDS = {
    "maximum resident set size": "peak_rss_bytes",
    "page faults": "page_faults",
    "block input operations": "block_input_operations",
    "block output operations": "block_output_operations",
    "voluntary context switches": "voluntary_context_switches",
    "involuntary context switches": "involuntary_context_switches",
    "instructions retired": "instructions_retired",
    "cycles elapsed": "cycles_elapsed",
}
STAGE_FIELDS = {
    "full import ready ms": "full_import_ready_ms",
    "scan complete ms": "scan_complete_ms",
    "first searchable ms": "first_searchable_ms",
    "ttf100 searchable ms": "ttf100_searchable_ms",
    "ttf1000 searchable ms": "ttf1000_searchable_ms",
    "stage scan ms": "stage_scan_ms",
    "stage parse ms": "stage_parse_ms",
    "stage db ms": "stage_db_ms",
    "stage index ms": "stage_index_ms",
}
THERMAL_STATES = {0: "nominal", 1: "fair", 2: "serious", 3: "critical"}
MEMORY_PRESSURE_STATES = {0: "normal", 1: "warning", 2: "critical", 4: "critical"}


class ProtocolInvalid(Exception):
    """The requested run weakens the frozen benchmark protocol."""


class TelemetryFailed(Exception):
    """A required bounded telemetry surface was unavailable."""


class SystemPressureMonitor:
    """Poll public macOS thermal and VM-pressure APIs without elevated privileges."""

    def __init__(self) -> None:
        self.thermal_states: list[str] = []
        self.memory_pressure_events: list[str] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._objc = ctypes.CDLL("/usr/lib/libobjc.A.dylib")
        self._objc.objc_getClass.argtypes = [ctypes.c_char_p]
        self._objc.objc_getClass.restype = ctypes.c_void_p
        self._objc.sel_registerName.argtypes = [ctypes.c_char_p]
        self._objc.sel_registerName.restype = ctypes.c_void_p
        self._message = self._objc.objc_msgSend
        self._message.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        self._message.restype = ctypes.c_void_p
        process_info_class = self._objc.objc_getClass(b"NSProcessInfo")
        self._process_info = self._message(
            process_info_class, self._objc.sel_registerName(b"processInfo")
        )
        self._thermal_selector = self._objc.sel_registerName(b"thermalState")
        self._libc = ctypes.CDLL(None)
        self._libc.sysctlbyname.argtypes = [
            ctypes.c_char_p,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_size_t),
            ctypes.c_void_p,
            ctypes.c_size_t,
        ]

    def start(self) -> None:
        self._sample()
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._thread.join(timeout=2)

    def _run(self) -> None:
        while not self._stop.wait(1):
            self._sample()

    def _sample(self) -> None:
        self._message.restype = ctypes.c_long
        thermal = int(self._message(self._process_info, self._thermal_selector))
        self.thermal_states.append(THERMAL_STATES.get(thermal, "unknown"))
        pressure = ctypes.c_int()
        size = ctypes.c_size_t(ctypes.sizeof(pressure))
        result = self._libc.sysctlbyname(
            b"vm.memory_pressure", ctypes.byref(pressure), ctypes.byref(size), None, 0
        )
        self.memory_pressure_events.append(
            MEMORY_PRESSURE_STATES.get(pressure.value, "unknown") if result == 0 else "unknown"
        )


def canonical_json(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def secure_write_json(path: pathlib.Path, value: object) -> None:
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(6)}.tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(canonical_json(value) + "\n")
        os.replace(temporary, path)
        os.chmod(path, 0o600)
    finally:
        temporary.unlink(missing_ok=True)


def validate_protocol(warmup_seconds: int, formal_repeats: int) -> None:
    if warmup_seconds < MIN_WARMUP_SECONDS:
        raise ProtocolInvalid("warm-up must be at least 30 seconds")
    if formal_repeats < MIN_FORMAL_REPEATS:
        raise ProtocolInvalid("formal repeats must be at least five")
    if formal_repeats > MAX_PUBLIC_RUNS:
        raise ProtocolInvalid("formal repeat count exceeds the bounded report limit")


def run_names(formal_repeats: int) -> list[str]:
    return ["warmup", *(f"formal-{index:02d}" for index in range(1, formal_repeats + 1))]


def parse_time_output(text: str) -> dict[str, int | float]:
    parsed: dict[str, int | float] = {}
    for line in text.splitlines():
        stripped = line.strip()
        match = re.fullmatch(r"(real|user|sys)\s+([0-9]+(?:\.[0-9]+)?)", stripped)
        if match:
            parsed[f"{match.group(1)}_seconds"] = float(match.group(2))
            continue
        match = re.fullmatch(r"([0-9]+)\s+(.+)", stripped)
        if match and match.group(2) in TIME_FIELDS:
            parsed[TIME_FIELDS[match.group(2)]] = int(match.group(1))
    required = {"real_seconds", "user_seconds", "sys_seconds", *TIME_FIELDS.values()}
    if set(parsed) != required:
        raise TelemetryFailed("/usr/bin/time output is incomplete")
    return parsed


def parse_stage_output(text: str) -> dict[str, float | int | bool]:
    metrics: dict[str, float | int | bool] = {}
    for line in text.splitlines():
        if ": " not in line:
            continue
        label, value = line.split(": ", 1)
        if label in STAGE_FIELDS:
            metrics[STAGE_FIELDS[label]] = float(value)
        elif label == "files discovered":
            metrics["files_discovered"] = int(value)
        elif label == "searchable documents":
            metrics["searchable_documents"] = int(value)
        elif label == "ocr required documents":
            metrics["ocr_backlog_documents"] = int(value)
        elif label == "failed documents":
            metrics["failed_documents"] = int(value)
        elif label == "resume classifier promotion":
            metrics["classifier_artifact_enabled"] = value == "enabled"
    return metrics


def parse_byte_count(value: str) -> int:
    match = re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)([KMGTP]?)", value)
    if not match:
        raise TelemetryFailed("top disk byte counter is invalid")
    scale = {"": 1, "K": 1024, "M": 1024**2, "G": 1024**3, "T": 1024**4, "P": 1024**5}
    return int(float(match.group(1)) * scale[match.group(2)])


def parse_top_output(text: str) -> list[dict[str, float]]:
    samples: list[dict[str, float]] = []
    current_idle: float | None = None
    current_disk: tuple[int, int] | None = None
    for line in text.splitlines():
        cpu = re.match(r"CPU usage: .*?([0-9.]+)% idle", line)
        if cpu:
            current_idle = float(cpu.group(1))
            continue
        disk = re.match(r"Disks: .*?/([0-9.]+[KMGTP]?) read, .*?/([0-9.]+[KMGTP]?) written", line)
        if disk:
            current_disk = (parse_byte_count(disk.group(1)), parse_byte_count(disk.group(2)))
            continue
        process = re.match(r"\s*\d+\s+([0-9.]+)\s+", line)
        if current_idle is not None and current_disk is not None and process:
            samples.append(
                {
                    "system_idle_percent": current_idle,
                    "target_cpu_percent": float(process.group(1)),
                    "system_disk_read_bytes": current_disk[0],
                    "system_disk_written_bytes": current_disk[1],
                }
            )
            current_idle = None
            current_disk = None
    return samples


def summarize_top_samples(samples: list[dict[str, float]], logical_cpus: int) -> dict[str, Any]:
    usable = samples[1:]
    if not usable or logical_cpus < 1:
        raise TelemetryFailed("top did not capture usable target samples")
    external = [
        round(max(0.0, (100.0 - sample["system_idle_percent"]) * logical_cpus - sample["target_cpu_percent"]), 3)
        for sample in usable
    ]
    threshold = logical_cpus * 100.0 * EXTERNAL_CPU_CAPACITY_FRACTION
    sustained = sum(value >= threshold for value in external) / len(external)
    return {
        "sample_count": len(usable),
        "system_idle_percent_min": min(sample["system_idle_percent"] for sample in usable),
        "system_idle_percent_median": statistics.median(sample["system_idle_percent"] for sample in usable),
        "target_cpu_percent_median": statistics.median(sample["target_cpu_percent"] for sample in usable),
        "external_cpu_percent_samples": external,
        "external_cpu_percent_median": statistics.median(external),
        "external_cpu_percent_max": max(external),
        "sustained_external_overlap_fraction": sustained,
        "system_disk_read_bytes_delta": max(
            0, int(usable[-1]["system_disk_read_bytes"] - usable[0]["system_disk_read_bytes"])
        ),
        "system_disk_written_bytes_delta": max(
            0, int(usable[-1]["system_disk_written_bytes"] - usable[0]["system_disk_written_bytes"])
        ),
    }


def parse_vm_stat(text: str) -> dict[str, int]:
    values: dict[str, int] = {}
    for line in text.splitlines():
        match = re.match(r"(Pageouts|Swapouts):\s+([0-9]+)\.", line)
        if match:
            values[match.group(1).lower()] = int(match.group(2))
    return {"pageouts": values.get("pageouts", 0), "swapouts": values.get("swapouts", 0)}


def memory_free_percent(text: str) -> int:
    match = re.search(r"System-wide memory free percentage:\s+([0-9]+)%", text)
    if not match:
        raise TelemetryFailed("memory_pressure output is incomplete")
    return int(match.group(1))


def classify_validity(observation: dict[str, Any]) -> dict[str, Any]:
    reasons: list[str] = []
    if observation["command_exit_code"] != 0:
        reasons.append("command_failed")
    if not observation["telemetry_ok"]:
        reasons.append("telemetry_failed")
    if not observation.get("semantic_ok", True):
        reasons.append("semantic_drift")
    if any(value in {"serious", "critical", "unknown"} for value in observation["thermal_states"]):
        reasons.append("thermal_pressure")
    if any(value in {"warning", "critical", "unknown"} for value in observation["memory_pressure_events"]):
        reasons.append("memory_pressure")
    if observation["pageouts_delta"] > 0:
        reasons.append("pageout_growth")
    if observation["swapouts_delta"] > 0:
        reasons.append("swapout_growth")
    if observation["sustained_external_overlap_fraction"] >= SUSTAINED_SAMPLE_FRACTION:
        reasons.append("sustained_external_cpu_overlap")
    return {"valid": not reasons, "reasons": reasons}


def variance(values: list[float]) -> dict[str, float | None]:
    if not values:
        return {"mean": None, "cv_percent": None, "range_percent": None}
    mean = statistics.fmean(values)
    return {
        "mean": mean,
        "cv_percent": 0.0 if len(values) == 1 or mean == 0 else statistics.pstdev(values) / mean * 100,
        "range_percent": 0.0 if min(values) == 0 else (max(values) / min(values) - 1) * 100,
    }


def summarize_formal_runs(runs: list[dict[str, Any]]) -> dict[str, Any]:
    if not runs:
        return {"formal_run_count": 0, "valid_run_count": 0}
    valid = [run for run in runs if run["validity"]["valid"]]
    ordered = sorted(valid, key=lambda run: run["metrics"]["full_import_ready_ms"])
    fields = ["full_import_ready_ms", "stage_parse_ms", "stage_db_ms", "stage_index_ms", "peak_rss_bytes"]
    return {
        "formal_run_count": len(runs),
        "valid_run_count": len(valid),
        "median_valid_run_id": ordered[(len(ordered) - 1) // 2]["run_id"] if ordered else None,
        "worst_valid_run_id": ordered[-1]["run_id"] if ordered else None,
        "variance": {
            "all_formal_runs": {field: variance([float(run["metrics"][field]) for run in runs]) for field in fields},
            "valid_formal_runs": {field: variance([float(run["metrics"][field]) for run in valid]) for field in fields},
        },
    }


def public_summary(
    experiment_id: str, runs: list[dict[str, Any]], aggregate: dict[str, Any], terminal: str
) -> dict[str, Any]:
    bounded_runs = [
        {
            "run_id": run["run_id"],
            "valid": run["validity"]["valid"],
            "invalid_reasons": run["validity"]["reasons"],
            "metrics": run["metrics"],
            "telemetry": run.get("telemetry", {}),
        }
        for run in runs[:MAX_PUBLIC_RUNS]
    ]
    return {
        "schema_version": "resume-ir.mixed-import-variance.v1",
        "experiment_id": experiment_id,
        "terminal": terminal,
        "blind_holdout_evaluated": False,
        "protocol": {
            "min_warmup_seconds": MIN_WARMUP_SECONDS,
            "min_formal_repeats": MIN_FORMAL_REPEATS,
            "profiler_run_separate": True,
            "single_process_spike_veto": False,
            "sustained_external_cpu_capacity_fraction": EXTERNAL_CPU_CAPACITY_FRACTION,
            "sustained_sample_fraction": SUSTAINED_SAMPLE_FRACTION,
        },
        "runs": bounded_runs,
        "aggregate": aggregate,
        "privacy": {
            "aggregate_only": True,
            "contains_raw_resume_text": False,
            "contains_raw_query_text": False,
            "contains_candidate_results": False,
            "contains_local_paths": False,
            "contains_filenames": False,
            "contains_process_names": False,
            "contains_commands": False,
            "contains_raw_samples": False,
            "contains_tokens": False,
        },
    }


def command_output(args: list[str]) -> str:
    return subprocess.run(args, check=True, text=True, capture_output=True).stdout


def find_child(parent_pid: int, timeout_seconds: float = 3.0) -> int:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        result = subprocess.run(["pgrep", "-P", str(parent_pid)], text=True, capture_output=True)
        if result.returncode == 0 and result.stdout.strip():
            return int(result.stdout.splitlines()[0])
        time.sleep(0.05)
    raise TelemetryFailed("timed command child process was not observable")


def terminate(process: subprocess.Popen[Any] | None) -> None:
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def run_once(
    run_id: str,
    run_dir: pathlib.Path,
    command: list[str],
    logical_cpus: int,
) -> dict[str, Any]:
    run_dir.mkdir(mode=0o700)
    stdout_path = run_dir / "target.stdout.local"
    stderr_path = run_dir / "target.stderr.local"
    time_path = run_dir / "time.local"
    top_path = run_dir / "top.local"
    vm_before = parse_vm_stat(command_output(["vm_stat"]))
    memory_before = memory_free_percent(command_output(["memory_pressure", "-Q"]))
    monitor = SystemPressureMonitor()
    monitor.start()
    timed: subprocess.Popen[Any] | None = None
    top: subprocess.Popen[Any] | None = None
    telemetry_ok = True
    with stdout_path.open("w", encoding="utf-8") as stdout, stderr_path.open("w", encoding="utf-8") as stderr:
        try:
            timed = subprocess.Popen(
                ["/usr/bin/time", "-l", "-p", "-o", str(time_path), *command],
                stdout=stdout,
                stderr=stderr,
            )
            child_pid = find_child(timed.pid)
            with top_path.open("w", encoding="utf-8") as top_output:
                top = subprocess.Popen(
                    [
                        "top", "-l", "0", "-s", "1", "-pid", str(child_pid),
                        "-stats", "pid,cpu,csw,time,threads,state,mem",
                    ],
                    stdout=top_output,
                    stderr=subprocess.DEVNULL,
                )
                exit_code = timed.wait()
        except (OSError, subprocess.SubprocessError, TelemetryFailed):
            telemetry_ok = False
            exit_code = 125 if timed is None else timed.wait()
        finally:
            terminate(top)
            monitor.stop()
    vm_after = parse_vm_stat(command_output(["vm_stat"]))
    memory_after = memory_free_percent(command_output(["memory_pressure", "-Q"]))
    try:
        time_metrics = parse_time_output(time_path.read_text(encoding="utf-8"))
        top_summary = summarize_top_samples(parse_top_output(top_path.read_text(encoding="utf-8")), logical_cpus)
        thermal_states = monitor.thermal_states
        memory_events = monitor.memory_pressure_events
        if not thermal_states or not memory_events:
            raise TelemetryFailed("system pressure monitor captured no state")
    except (OSError, TelemetryFailed):
        telemetry_ok = False
        time_metrics = {}
        top_summary = {"sustained_external_overlap_fraction": 1.0}
        thermal_states, memory_events = ["unknown"], ["warning"]
    stage_metrics = parse_stage_output(stdout_path.read_text(encoding="utf-8"))
    required_stage_metrics = {*STAGE_FIELDS.values(), "classifier_artifact_enabled"}
    semantic_ok = required_stage_metrics.issubset(stage_metrics) and stage_metrics.get(
        "classifier_artifact_enabled"
    ) is True
    observation = {
        "telemetry_ok": telemetry_ok,
        "command_exit_code": exit_code,
        "semantic_ok": semantic_ok,
        "thermal_states": thermal_states,
        "memory_pressure_events": memory_events,
        "pageouts_delta": max(0, vm_after["pageouts"] - vm_before["pageouts"]),
        "swapouts_delta": max(0, vm_after["swapouts"] - vm_before["swapouts"]),
        "sustained_external_overlap_fraction": top_summary["sustained_external_overlap_fraction"],
    }
    metrics = {**stage_metrics, **time_metrics}
    return {
        "run_id": run_id,
        "metrics": metrics,
        "validity": classify_validity(observation),
        "telemetry": {
            "sample_count": top_summary.get("sample_count", 0),
            "system_idle_percent_min": top_summary.get("system_idle_percent_min"),
            "system_idle_percent_median": top_summary.get("system_idle_percent_median"),
            "target_cpu_percent_median": top_summary.get("target_cpu_percent_median"),
            "external_cpu_percent_median": top_summary.get("external_cpu_percent_median"),
            "external_cpu_percent_max": top_summary.get("external_cpu_percent_max"),
            "sustained_external_overlap_fraction": top_summary["sustained_external_overlap_fraction"],
            "system_disk_read_bytes_delta": top_summary.get("system_disk_read_bytes_delta"),
            "system_disk_written_bytes_delta": top_summary.get("system_disk_written_bytes_delta"),
            "thermal_states": sorted(set(thermal_states)),
            "memory_pressure_events": sorted(set(memory_events)),
            "memory_free_percent_min": min(memory_before, memory_after),
            "pageouts_delta": observation["pageouts_delta"],
            "swapouts_delta": observation["swapouts_delta"],
        },
    }


def actual_command(arguments: argparse.Namespace, data_dir: pathlib.Path) -> list[str]:
    return [
        str(arguments.binary), "--data-dir", str(data_dir), "import", "--root", str(arguments.root),
        "--profile", "discovery", "--parse-workers", str(arguments.parse_workers),
        "--resume-classifier-model", str(arguments.model),
    ]


def smoke_command() -> list[str]:
    output = "\n".join(
        [
            "resume classifier promotion: enabled", "files discovered: 8", "searchable documents: 4",
            "ocr required documents: 1", "failed documents: 1", "scan complete ms: 10",
            "first searchable ms: 20", "ttf100 searchable ms: 30", "ttf1000 searchable ms: 40",
            "full import ready ms: 100", "stage scan ms: 10", "stage parse ms: 80",
            "stage db ms: 50", "stage index ms: 20",
        ]
    )
    return [sys.executable, "-c", f"import time; print({output!r}); time.sleep(2.2)"]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=pathlib.Path, default=os.environ.get("RESUME_IR_LOCAL_EVIDENCE_DIR"))
    parser.add_argument("--experiment-id", required=True)
    parser.add_argument("--binary", type=pathlib.Path)
    parser.add_argument("--root", type=pathlib.Path)
    parser.add_argument("--model", type=pathlib.Path)
    parser.add_argument("--parse-workers", type=int, default=3)
    parser.add_argument("--warmup-seconds", type=int, default=MIN_WARMUP_SECONDS)
    parser.add_argument("--formal-repeats", type=int, default=MIN_FORMAL_REPEATS)
    parser.add_argument("--synthetic-smoke", action="store_true")
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    validate_protocol(arguments.warmup_seconds, arguments.formal_repeats)
    if not EXPERIMENT_ID.fullmatch(arguments.experiment_id):
        raise ProtocolInvalid("experiment ID must be opaque and public-safe")
    if arguments.evidence_dir is None:
        raise ProtocolInvalid("local evidence directory is required")
    configured_evidence_dir = pathlib.Path(arguments.evidence_dir).expanduser()
    if configured_evidence_dir.is_symlink():
        raise ProtocolInvalid("local evidence directory cannot be a symlink")
    evidence_dir = configured_evidence_dir.resolve()
    evidence_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    experiment_dir = evidence_dir / arguments.experiment_id
    experiment_dir.mkdir(mode=0o700)
    if not arguments.synthetic_smoke:
        for value in [arguments.binary, arguments.root, arguments.model]:
            if value is None or not value.exists():
                raise ProtocolInvalid("binary, root, and owner-only model are required")
        if arguments.model.stat().st_mode & 0o077:
            raise ProtocolInvalid("classifier model must be owner-only")
    logical_cpus = int(command_output(["sysctl", "-n", "hw.logicalcpu"]).strip())
    spec = {
        "schema_version": "resume-ir.mixed-import-variance-spec.v1",
        "experiment_id": arguments.experiment_id,
        "warmup_seconds": arguments.warmup_seconds,
        "formal_repeats": arguments.formal_repeats,
        "synthetic_smoke": arguments.synthetic_smoke,
        "blind_holdout_evaluated": False,
    }
    secure_write_json(experiment_dir / "spec.local.json", spec)
    warmup_elapsed = 0.0
    warmup_index = 0
    while warmup_elapsed < arguments.warmup_seconds:
        warmup_index += 1
        run_id = f"warmup-{warmup_index:02d}"
        data_dir = experiment_dir / f"{run_id}-data"
        command = smoke_command() if arguments.synthetic_smoke else actual_command(arguments, data_dir)
        started = time.monotonic()
        warmup = run_once(run_id, experiment_dir / run_id, command, logical_cpus)
        warmup_elapsed += time.monotonic() - started
        secure_write_json(experiment_dir / f"{run_id}.local.json", warmup)
        if warmup["validity"]["reasons"] and "command_failed" in warmup["validity"]["reasons"]:
            raise ProtocolInvalid("warm-up command failed")
        if arguments.synthetic_smoke:
            warmup_elapsed = arguments.warmup_seconds
    formal_runs: list[dict[str, Any]] = []
    for run_id in run_names(arguments.formal_repeats)[1:]:
        data_dir = experiment_dir / f"{run_id}-data"
        command = smoke_command() if arguments.synthetic_smoke else actual_command(arguments, data_dir)
        run = run_once(run_id, experiment_dir / run_id, command, logical_cpus)
        secure_write_json(experiment_dir / f"{run_id}.local.json", run)
        formal_runs.append(run)
    aggregate = summarize_formal_runs(formal_runs)
    terminal = "synthetic_smoke_complete" if arguments.synthetic_smoke else (
        "variance_observed" if aggregate["valid_run_count"] >= MIN_FORMAL_REPEATS else "methodology_failed"
    )
    summary = public_summary(arguments.experiment_id, formal_runs, aggregate, terminal)
    secure_write_json(experiment_dir / "public-redacted-aggregate.json", summary)
    print(canonical_json(summary))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ProtocolInvalid, subprocess.CalledProcessError) as error:
        print(f"mixed import variance blocked: {error}", file=sys.stderr)
        raise SystemExit(2)
