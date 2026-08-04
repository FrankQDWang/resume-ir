#!/usr/bin/env python3
"""Run the bounded Issue #380 ORT-vs-Core-ML fixed-B4x512 screen."""

from __future__ import annotations

import argparse
import json
import math
import os
import random
import resource
import select
import statistics
import subprocess
import time
from pathlib import Path

import numpy as np


STREAM_SCHEMA = "resume-ir.embedding-stream.v1"
MODEL_ID = "intfloat-multilingual-e5-small-qint8-r1"
DIMENSION, BATCH, TOKENS = 384, 4, 512
MAX_FRAME_BYTES = 4 * 1024 * 1024


def read_exact(stream: object, size: int, timeout: float) -> bytes:
    descriptor = stream.fileno()  # type: ignore[attr-defined]
    deadline = time.monotonic() + timeout
    result = bytearray()
    while len(result) < size:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([descriptor], [], [], remaining)[0]:
            raise RuntimeError("resident response timed out")
        chunk = os.read(descriptor, size - len(result))
        if not chunk:
            raise RuntimeError("resident response ended early")
        result.extend(chunk)
    return bytes(result)


def read_frame(stream: object, timeout: float) -> dict[str, object]:
    size = int.from_bytes(read_exact(stream, 4, timeout), "big")
    if size <= 0 or size > MAX_FRAME_BYTES:
        raise RuntimeError("resident response frame was invalid")
    payload = json.loads(read_exact(stream, size, timeout))
    if not isinstance(payload, dict):
        raise RuntimeError("resident response payload was invalid")
    return payload


def write_frame(stream: object, payload: dict[str, object]) -> None:
    encoded = json.dumps(payload, separators=(",", ":")).encode()
    stream.write(len(encoded).to_bytes(4, "big"))  # type: ignore[attr-defined]
    stream.write(encoded)  # type: ignore[attr-defined]
    stream.flush()  # type: ignore[attr-defined]


def ort_block(
    binary: Path,
    runtime_dir: Path,
    texts: list[str],
    warmups: int,
    repetitions: int,
    timeout: float,
) -> list[float]:
    environment = os.environ.copy()
    environment.update(
        RESUME_IR_EMBEDDING_RUNTIME_DIR=str(runtime_dir),
        RESUME_IR_EMBEDDING_MODEL_ID=MODEL_ID,
        RESUME_IR_EMBEDDING_DIMENSION=str(DIMENSION),
        RESUME_IR_EMBEDDING_INTRA_THREADS="3",
    )
    process = subprocess.Popen(
        [
            str(binary),
            "--resident-embedding-pool-experiment",
            "--resident-embedding-pool-role=bulk",
        ],
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        if process.stdin is None or process.stdout is None:
            raise RuntimeError("resident pipes were unavailable")
        ready = read_frame(process.stdout, timeout)
        if ready.get("type") != "ready":
            raise RuntimeError("resident did not become ready")
        inputs = [
            {"role": "passage", "text": text.removeprefix("passage: ")}
            for text in texts
        ]
        samples: list[float] = []
        for request_id in range(1, warmups + repetitions + 1):
            request = {
                "schema_version": STREAM_SCHEMA,
                "request_id": request_id,
                "model_id": MODEL_ID,
                "dimension": DIMENSION,
                "inputs": inputs,
            }
            write_frame(process.stdin, request)
            response = read_frame(process.stdout, timeout)
            telemetry = response.get("telemetry")
            if response.get("type") != "result" or not isinstance(telemetry, dict):
                raise RuntimeError("resident result was invalid")
            if telemetry.get("padded_token_count") != BATCH * TOKENS:
                raise RuntimeError("ORT control did not route fixed B4x512")
            if request_id > warmups:
                samples.append(
                    sum(
                        float(telemetry[key])
                        for key in ("onnx_us", "pool_us", "normalize_us")
                    )
                    / 1000.0
                )
        process.stdin.close()
        if process.wait(timeout=5) != 0:
            raise RuntimeError("resident did not exit cleanly")
        return samples
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def coreml_block(
    runner: Path,
    model: Path,
    candidate_dir: Path,
    output: Path,
    warmups: int,
    repetitions: int,
    include_plan: bool,
    timeout: float,
) -> dict[str, object]:
    completed = subprocess.run(
        [
            str(runner),
            str(model),
            str(candidate_dir / "input_ids.i32le"),
            str(candidate_dir / "attention_mask.i32le"),
            str(output),
            str(warmups),
            str(repetitions),
            "1" if include_plan else "0",
        ],
        check=True,
        capture_output=True,
        timeout=timeout,
    )
    payload = json.loads(completed.stdout)
    if payload.get("vector_count") != BATCH or payload.get("dimension") != DIMENSION:
        raise RuntimeError("Core ML output was invalid")
    return payload


def bootstrap_interval(values: list[float], rounds: int = 20_000) -> tuple[float, float]:
    generator = random.Random(380)
    means = sorted(
        statistics.fmean(generator.choice(values) for _ in values) for _ in range(rounds)
    )
    return means[int(0.025 * rounds)], means[int(0.975 * rounds)]


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, math.ceil(len(ordered) * fraction) - 1)]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--embedding-bin", type=Path, required=True)
    parser.add_argument("--runtime-dir", type=Path, required=True)
    parser.add_argument("--swift-runner", type=Path, required=True)
    parser.add_argument("--compiled-model", type=Path, required=True)
    parser.add_argument("--candidate-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--blocks", type=int, default=10)
    parser.add_argument("--repetitions", type=int, default=8)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--timeout-seconds", type=float, default=45.0)
    args = parser.parse_args()
    if not 4 <= args.blocks <= 20 or not 3 <= args.repetitions <= 32:
        parser.error("blocks or repetitions are outside the bounded contract")

    texts = json.loads((args.candidate_dir / "synthetic_texts.json").read_text())
    if not isinstance(texts, list) or len(texts) != BATCH:
        raise RuntimeError("synthetic text fixture was invalid")
    candidate_samples: list[float] = []
    control_samples: list[float] = []
    paired_block_improvements: list[float] = []
    placement: dict[str, float] | None = None
    vector_output = args.output.with_suffix(".vectors.f32le")

    for block in range(args.blocks):
        order = ("control", "candidate") if block % 2 == 0 else ("candidate", "control")
        block_values: dict[str, list[float]] = {}
        for arm in order:
            if arm == "control":
                values = ort_block(
                    args.embedding_bin,
                    args.runtime_dir,
                    texts,
                    args.warmups,
                    args.repetitions,
                    args.timeout_seconds,
                )
            else:
                result = coreml_block(
                    args.swift_runner,
                    args.compiled_model,
                    args.candidate_dir,
                    vector_output,
                    args.warmups,
                    args.repetitions,
                    placement is None,
                    args.timeout_seconds,
                )
                values = [float(value) for value in result["samples_ms"]]
                if placement is None:
                    placement = {
                        key: float(value)
                        for key, value in result[
                            "estimated_cost_by_preferred_device"
                        ].items()
                    }
            block_values[arm] = values
        control_samples.extend(block_values["control"])
        candidate_samples.extend(block_values["candidate"])
        control_mean = statistics.fmean(block_values["control"])
        candidate_mean = statistics.fmean(block_values["candidate"])
        paired_block_improvements.append(
            (control_mean - candidate_mean) / control_mean * 100.0
        )

    reference = np.fromfile(
        args.candidate_dir / "pytorch_reference.f32le", dtype="<f4"
    ).reshape(BATCH, DIMENSION)
    candidate = np.fromfile(vector_output, dtype="<f4").reshape(BATCH, DIMENSION)
    cosine = np.sum(reference * candidate, axis=1) / (
        np.linalg.norm(reference, axis=1) * np.linalg.norm(candidate, axis=1)
    )
    low, high = bootstrap_interval(paired_block_improvements)
    placement = placement or {}
    total_cost = sum(placement.values())
    cpu_cost_fraction = placement.get("cpu", 0.0) / total_cost if total_cost else 1.0
    child_peak_mib = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss / 1_048_576
    mean_improvement = statistics.fmean(paired_block_improvements)
    gates = {
        "latency_improvement_at_least_25_percent": mean_improvement >= 25.0,
        "paired_95_ci_positive": low > 0.0,
        "minimum_source_cosine_at_least_0_995": float(cosine.min()) >= 0.995,
        "mean_source_cosine_at_least_0_999": float(cosine.mean()) >= 0.999,
        "compute_not_cpu_dominant": cpu_cost_fraction < 0.5,
        "child_peak_rss_at_most_3072_mib": child_peak_mib <= 3072.0,
        "lifecycle_clean": True,
    }
    report = {
        "schema_version": "resume-ir.coreml-b4x512-experiment.v1",
        "claim": "pass" if all(gates.values()) else "fail",
        "host_tier": "macos_m4_h2_local_only",
        "blocks": args.blocks,
        "samples_per_arm": len(control_samples),
        "control": {
            "mean_ms": statistics.fmean(control_samples),
            "p50_ms": percentile(control_samples, 0.5),
            "p95_ms": percentile(control_samples, 0.95),
        },
        "candidate": {
            "mean_ms": statistics.fmean(candidate_samples),
            "p50_ms": percentile(candidate_samples, 0.5),
            "p95_ms": percentile(candidate_samples, 0.95),
        },
        "paired_improvement_percent": {
            "mean": mean_improvement,
            "ci95_low": low,
            "ci95_high": high,
        },
        "source_parity": {
            "minimum_cosine": float(cosine.min()),
            "mean_cosine": float(cosine.mean()),
            "finite": bool(np.isfinite(candidate).all()),
        },
        "compute_preferred_cost_fraction": {
            key: value / total_cost for key, value in placement.items()
        },
        "child_peak_rss_mib": child_peak_mib,
        "gates": gates,
        "privacy": {
            "contains_private_resume_text": False,
            "contains_raw_query_text": False,
            "contains_candidate_results": False,
            "contains_local_paths": False,
            "contains_tokens": False,
            "contains_vectors": False,
        },
    }
    encoded = json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n"
    if len(encoded.encode()) > 64 * 1024:
        raise RuntimeError("report exceeded the public boundary")
    args.output.write_text(encoded)
    print(json.dumps(report, separators=(",", ":"), allow_nan=False))
    return 0 if report["claim"] == "pass" else 2


if __name__ == "__main__":
    raise SystemExit(main())
