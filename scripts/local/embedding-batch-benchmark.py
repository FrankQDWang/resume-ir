#!/usr/bin/env python3
"""Run a short, synthetic resident embedding base-vs-head comparison."""

from __future__ import annotations

import argparse
import json
import math
import os
import select
import subprocess
import sys
import time
from pathlib import Path


SCHEMA_VERSION = "resume-ir.embedding-stream.v1"
REPORT_SCHEMA = "resume-ir.embedding-batch-microbenchmark.v1"
DEFAULT_MODEL_ID = "intfloat-multilingual-e5-small-qint8-r1"
DEFAULT_DIMENSION = 384
MAX_FRAME_BYTES = 4 * 1024 * 1024
BATCH_SIZES = (1, 2, 4)
SYNTHETIC_TEXTS = (
    "synthetic benchmark passage alpha beta gamma delta epsilon zeta eta theta",
    "synthetic benchmark passage iota kappa lambda mu nu xi omicron pi",
    "synthetic benchmark passage rho sigma tau upsilon phi chi psi omega",
    "synthetic benchmark passage one two three four five six seven eight",
)


def benchmark_texts(workload: str) -> tuple[str, ...]:
    if workload == "max-length":
        return tuple(" ".join([text] * 160) for text in SYNTHETIC_TEXTS)
    return SYNTHETIC_TEXTS


def read_exact(stream: object, size: int, timeout_seconds: float) -> bytes:
    file_descriptor = stream.fileno()  # type: ignore[attr-defined]
    deadline = time.monotonic() + timeout_seconds
    chunks: list[bytes] = []
    remaining = size
    while remaining:
        wait_seconds = deadline - time.monotonic()
        if wait_seconds <= 0 or not select.select([file_descriptor], [], [], wait_seconds)[0]:
            raise RuntimeError("resident response timed out")
        chunk = os.read(file_descriptor, remaining)
        if not chunk:
            raise RuntimeError("resident response ended early")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def read_frame(stream: object, timeout_seconds: float) -> dict[str, object]:
    size = int.from_bytes(read_exact(stream, 4, timeout_seconds), "big")
    if size <= 0 or size > MAX_FRAME_BYTES:
        raise RuntimeError("resident response frame was invalid")
    try:
        payload = json.loads(read_exact(stream, size, timeout_seconds))
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise RuntimeError("resident response payload was invalid") from error
    if not isinstance(payload, dict):
        raise RuntimeError("resident response shape was invalid")
    return payload


def write_frame(stream: object, payload: dict[str, object]) -> None:
    encoded = json.dumps(payload, separators=(",", ":"), allow_nan=False).encode()
    if not encoded or len(encoded) > MAX_FRAME_BYTES:
        raise RuntimeError("resident request frame was invalid")
    stream.write(len(encoded).to_bytes(4, "big"))  # type: ignore[attr-defined]
    stream.write(encoded)  # type: ignore[attr-defined]
    stream.flush()  # type: ignore[attr-defined]


def percentile(samples: list[float], fraction: float) -> float:
    ordered = sorted(samples)
    index = min(len(ordered) - 1, math.ceil(fraction * len(ordered)) - 1)
    return ordered[index]


def run_variant(
    label: str,
    binary: Path,
    runtime_dir: Path,
    model_id: str,
    dimension: int,
    repetitions: int,
    timeout_seconds: float,
    resident_mode: str,
    intra_threads: int,
    workload: str,
) -> tuple[dict[str, object], dict[int, list[list[float]]]]:
    environment = os.environ.copy()
    environment.update(
        {
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(runtime_dir),
            "RESUME_IR_EMBEDDING_MODEL_ID": model_id,
            "RESUME_IR_EMBEDDING_DIMENSION": str(dimension),
            "RESUME_IR_EMBEDDING_INTRA_THREADS": str(intra_threads),
        }
    )
    resident_args = ["--resident"]
    if resident_mode == "bulk":
        resident_args = [
            "--resident-embedding-pool-experiment",
            "--resident-embedding-pool-role=bulk",
        ]
    process = subprocess.Popen(
        [str(binary), *resident_args],
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        if process.stdin is None or process.stdout is None:
            raise RuntimeError("resident pipes were unavailable")
        ready = read_frame(process.stdout, timeout_seconds)
        if (
            ready.get("type") != "ready"
            or ready.get("schema_version") != SCHEMA_VERSION
            or ready.get("model_id") != model_id
            or ready.get("dimension") != dimension
        ):
            raise RuntimeError("resident ready response was invalid")

        measurements: list[dict[str, object]] = []
        vectors_by_size: dict[int, list[list[float]]] = {}
        request_id = 1
        texts = benchmark_texts(workload)
        for batch_size in BATCH_SIZES:
            inputs = [
                {"role": "passage", "text": text}
                for text in texts[:batch_size]
            ]
            request = {
                "schema_version": SCHEMA_VERSION,
                "request_id": request_id,
                "model_id": model_id,
                "dimension": dimension,
                "inputs": inputs,
            }
            write_frame(process.stdin, request)
            warmup = read_frame(process.stdout, timeout_seconds)
            if warmup.get("type") != "result" or warmup.get("request_id") != request_id:
                raise RuntimeError("resident warmup response was invalid")
            request_id += 1

            samples: list[float] = []
            for _ in range(repetitions):
                request["request_id"] = request_id
                started = time.perf_counter()
                write_frame(process.stdin, request)
                response = read_frame(process.stdout, timeout_seconds)
                elapsed_ms = (time.perf_counter() - started) * 1000.0
                vectors = response.get("vectors")
                if (
                    response.get("type") != "result"
                    or response.get("request_id") != request_id
                    or not isinstance(vectors, list)
                    or len(vectors) != batch_size
                    or any(
                        not isinstance(vector, list)
                        or len(vector) != dimension
                        or any(not isinstance(value, (int, float)) or not math.isfinite(value) for value in vector)
                        for vector in vectors
                    )
                ):
                    raise RuntimeError("resident result response was invalid")
                vectors_by_size[batch_size] = vectors
                samples.append(elapsed_ms)
                request_id += 1
            measurements.append(
                {
                    "batch_size": batch_size,
                    "samples": len(samples),
                    "mean_ms": sum(samples) / len(samples),
                    "p50_ms": percentile(samples, 0.50),
                    "p95_ms": percentile(samples, 0.95),
                }
            )

        process.stdin.close()
        if process.wait(timeout=5) != 0:
            raise RuntimeError("resident process did not exit cleanly")
        return {"label": label, "measurements": measurements}, vectors_by_size
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        process.kill()
        process.wait()
        raise RuntimeError(f"{label} resident benchmark failed") from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-bin", type=Path, required=True)
    parser.add_argument("--candidate-bin", type=Path, required=True)
    parser.add_argument("--runtime-dir", type=Path, required=True)
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--dimension", type=int, default=DEFAULT_DIMENSION)
    parser.add_argument("--repetitions", type=int, default=8)
    parser.add_argument("--timeout-seconds", type=float, default=30.0)
    parser.add_argument(
        "--resident-mode", choices=("default", "bulk"), default="default"
    )
    parser.add_argument("--intra-threads", type=int, default=1)
    parser.add_argument(
        "--workload", choices=("standard", "max-length"), default="standard"
    )
    args = parser.parse_args()
    if (
        args.dimension <= 0
        or args.repetitions <= 0
        or args.timeout_seconds <= 0
        or args.intra_threads <= 0
    ):
        parser.error(
            "dimension, repetitions, timeout-seconds, and intra-threads must be positive"
        )

    baseline, baseline_vectors = run_variant(
        "base",
        args.baseline_bin,
        args.runtime_dir,
        args.model_id,
        args.dimension,
        args.repetitions,
        args.timeout_seconds,
        args.resident_mode,
        args.intra_threads,
        args.workload,
    )
    candidate, candidate_vectors = run_variant(
        "head",
        args.candidate_bin,
        args.runtime_dir,
        args.model_id,
        args.dimension,
        args.repetitions,
        args.timeout_seconds,
        args.resident_mode,
        args.intra_threads,
        args.workload,
    )
    baseline_by_size = {
        item["batch_size"]: item for item in baseline["measurements"]  # type: ignore[index]
    }
    candidate_by_size = {
        item["batch_size"]: item for item in candidate["measurements"]  # type: ignore[index]
    }
    speedups = []
    for batch_size in BATCH_SIZES:
        base_mean = baseline_by_size[batch_size]["mean_ms"]
        head_mean = candidate_by_size[batch_size]["mean_ms"]
        max_abs_delta = max(
            abs(base_value - head_value)
            for base_vector, head_vector in zip(
                baseline_vectors[batch_size], candidate_vectors[batch_size]
            )
            for base_value, head_value in zip(base_vector, head_vector)
        )
        dot_product = sum(
            base_value * head_value
            for base_vector, head_vector in zip(
                baseline_vectors[batch_size], candidate_vectors[batch_size]
            )
            for base_value, head_value in zip(base_vector, head_vector)
        )
        base_norm = math.sqrt(
            sum(
                base_value * base_value
                for vector in baseline_vectors[batch_size]
                for base_value in vector
            )
        )
        head_norm = math.sqrt(
            sum(
                head_value * head_value
                for vector in candidate_vectors[batch_size]
                for head_value in vector
            )
        )
        speedups.append(
            {
                "batch_size": batch_size,
                "mean_speedup": base_mean / head_mean,
                "max_abs_vector_delta": max_abs_delta,
                "cosine_similarity": dot_product / (base_norm * head_norm),
            }
        )
    report = {
        "schema_version": REPORT_SCHEMA,
        "source": "synthetic_public_fixture",
        "claim": "local_synthetic_microbenchmark_only",
        "repetitions": args.repetitions,
        "resident_mode": args.resident_mode,
        "intra_threads": args.intra_threads,
        "workload": args.workload,
        "variants": [baseline, candidate],
        "speedups": speedups,
    }
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
