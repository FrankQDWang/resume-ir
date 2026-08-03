#!/usr/bin/env python3
"""Classify first and warm queries across synthetic OCR publications."""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import runpy
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OCR = runpy.run_path(str(ROOT / "scripts/local/ocr-on-import-attribution.py"))
CONTRACT = runpy.run_path(str(ROOT / "scripts/ci/check-ocr-query-publication.py"))
HttpTransport = OCR["HttpTransport"]
ManagedProcess = OCR["ManagedProcess"]
HarnessError = OCR["HarnessError"]
group_exists = OCR["group_exists"]
start_managed_scan = OCR["start_managed_scan"]
validate_contract = CONTRACT["validate"]

SCHEMA = "resume-ir.ocr-query-publication.v1"
IPC_PROTOCOL = "resume-ir.daemon-ipc.v5"
MODEL_ID, DIMENSION = "intfloat-multilingual-e5-small-qint8-r1", 384
SEED, DEADLINE_MS, WORKER_INTERVAL_MS = 20260803, 10_000, 30_000
MODES = ("fulltext", "semantic", "hybrid")
SCHEDULE = (
    ("hybrid", "fulltext", "semantic"),
    ("fulltext", "semantic", "hybrid"),
    ("semantic", "hybrid", "fulltext"),
)
STAGES = ("parse", "prefilter", "bm25", "ann", "fusion", "bulk_hydrate", "snippet")
OUTCOMES = (
    "exact_expected", "valid_epoch_result_change", "deadline_partial",
    "semantic_partial", "overload", "http_error", "protocol_error",
    "transport_error", "cancelled",
)
VALID_COMPLETIONS = {"exact_expected", "valid_epoch_result_change"}
PRIVACY = {
    "contains_private_paths": False,
    "contains_filenames": False,
    "contains_resume_text": False,
    "contains_query_text": False,
    "contains_candidate_results": False,
    "contains_document_or_version_ids": False,
    "contains_token_ids": False,
    "contains_vectors": False,
    "contains_pids": False,
    "contains_logs_or_traces": False,
    "contains_direct_raw_hashes": False,
    "contains_databases_or_indexes": False,
    "contains_model_or_runtime_bytes": False,
}
FIXED_WORKLOAD = dict(CONTRACT["FIXED_WORKLOAD"])
OBSERVABILITY = dict(CONTRACT["OBSERVABILITY"])
CLAIMS = list(CONTRACT["CLAIMS"])


class ExperimentError(RuntimeError):
    """A fixed public-safe experiment failure code."""


@dataclass(frozen=True)
class Inputs:
    daemon: Path
    embedding: Path
    embedding_dir: Path
    ocr: Path
    tessdata: Path
    renderer: Path
    pdfium_dir: Path
    classifier: Path
    output: Path
    timeout: float
    revision: str


def bounded_int(value: object, reason: str = "invalid_status_aggregate") -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value < 2**63:
        raise ExperimentError(reason)
    return value


def finite(value: object) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    result = float(value)
    return result if math.isfinite(result) and result >= 0.0 else None


def exact_revision() -> str:
    try:
        value = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, check=True, text=True,
            capture_output=True, timeout=10,
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        raise ExperimentError("revision_unavailable") from None
    if len(value) != 40 or any(character not in "0123456789abcdef" for character in value):
        raise ExperimentError("revision_invalid")
    return value


def build_pdf(objects: list[bytes]) -> bytes:
    output = bytearray(b"%PDF-1.4\n%\xff\xff\xff\xff\n")
    offsets = []
    for index, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{index} 0 obj\n".encode())
        output.extend(value)
        output.extend(b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f\r\n".encode())
    for offset in offsets:
        output.extend(f"{offset:010} 00000 n\r\n".encode())
    output.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
        f"startxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(output)


def text_pdf(index: int) -> bytes:
    lines = (
        "SUMMARY", f"Synthetic Publication Candidate {index}", "EXPERIENCE",
        f"Built deterministic local service cohort {index}", "EDUCATION",
        "Synthetic Institute", "SKILLS", "Python SQL local systems",
    )
    commands = ["BT", "/F1 20 Tf", "72 730 Td"]
    for position, line in enumerate(lines):
        escaped = line.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
        if position:
            commands.append("0 -42 Td")
        commands.append(f"({escaped}) Tj")
    commands.append("ET")
    content = ("\n".join(commands) + "\n").encode()
    stream = f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"endstream"
    return build_pdf([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        stream,
    ])


def image_pdf(ppm: bytes) -> bytes:
    lines = []
    start = 0
    for _ in range(3):
        end = ppm.find(b"\n", start)
        if end < 0:
            raise ExperimentError("raster_protocol_invalid")
        lines.append(ppm[start:end])
        start = end + 1
    if lines[0] != b"P6" or lines[2] != b"255":
        raise ExperimentError("raster_protocol_invalid")
    try:
        width, height = (int(value) for value in lines[1].split())
    except (TypeError, ValueError):
        raise ExperimentError("raster_protocol_invalid") from None
    pixels = ppm[start:]
    if width <= 0 or height <= 0 or len(pixels) != width * height * 3:
        raise ExperimentError("raster_protocol_invalid")
    image = (
        f"<< /Type /XObject /Subtype /Image /Width {width} /Height {height} "
        f"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {len(pixels)} >>\nstream\n"
    ).encode() + pixels + b"\nendstream"
    content = b"q\n612 0 0 792 0 0 cm\n/Im1 Do\nQ\n"
    stream = f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"endstream"
    return build_pdf([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>",
        image,
        stream,
    ])


def rasterize(inputs: Inputs, staging: Path, count: int) -> list[bytes]:
    environment = os.environ.copy()
    environment["RESUME_IR_PDFIUM_RUNTIME_DIR"] = str(inputs.pdfium_dir)
    output = []
    for index in range(1, count + 1):
        source = staging / f"source-{index}.pdf"
        source.write_bytes(text_pdf(index))
        render_environment = dict(environment)
        render_environment.update({
            "RESUME_IR_PDF_RENDER_INPUT_PATH": str(source),
            "RESUME_IR_PDF_RENDER_PAGE_NO": "1",
            "RESUME_IR_PDF_RENDER_DPI": "150",
        })
        try:
            rendered = subprocess.run(
                [str(inputs.renderer)], env=render_environment, check=False,
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=inputs.timeout,
            )
        except (OSError, subprocess.TimeoutExpired):
            raise ExperimentError("rasterization_failed") from None
        if rendered.returncode != 0 or len(rendered.stdout) > 32 * 1024 * 1024:
            raise ExperimentError("rasterization_failed")
        output.append(image_pdf(rendered.stdout))
    return output


def daemon_command(inputs: Inputs, data: Path, work_ocr: bool) -> list[str]:
    command = [
        str(inputs.daemon), "--data-dir", str(data), "run", "--foreground",
        "--work-imports", "--work-index", "--rescan-completed-imports",
        "--watch-import-roots", "--import-rescan-min-age-seconds", "300",
        "--expected-ipc-protocol", IPC_PROTOCOL, "--ipc-listen", "127.0.0.1:0",
        "--embedding-command", str(inputs.embedding), "--embedding-model-id", MODEL_ID,
        "--embedding-dimension", str(DIMENSION), "--ocr-tesseract-command", str(inputs.ocr),
        "--ocr-lang", "eng+chi_sim", "--pdf-render-command", str(inputs.renderer),
        "--resume-classifier-model", str(inputs.classifier),
        "--worker-interval-ms", str(WORKER_INTERVAL_MS),
    ]
    if work_ocr:
        command.extend(("--work-ocr", "--ocr-jobs-per-tick", "1"))
    return command


def daemon_environment(inputs: Inputs) -> dict[str, str]:
    environment = os.environ.copy()
    environment.update({
        "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(inputs.embedding_dir),
        "RESUME_IR_EMBEDDING_INTRA_THREADS": "3",
        "RESUME_IR_PDFIUM_RUNTIME_DIR": str(inputs.pdfium_dir),
        "TESSDATA_PREFIX": str(inputs.tessdata),
    })
    return environment


def wait_seed(transport: object, root_id: str, task_id: str, publications: int,
              timeout: float) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        roots = transport.get_json("/source-roots")  # type: ignore[attr-defined]
        status = transport.get_json("/status")  # type: ignore[attr-defined]
        values = roots.get("roots")
        matches = [item.get("last_scan") for item in values if isinstance(item, dict)
                   and item.get("root_id") == root_id] if isinstance(values, list) else []
        scan = matches[0] if len(matches) == 1 and isinstance(matches[0], dict) else None
        counts = scan.get("counts") if isinstance(scan, dict) else None
        complete = (
            isinstance(scan, dict) and scan.get("scan_id") == task_id
            and scan.get("phase") == "complete" and scan.get("completeness") == "complete"
            and isinstance(counts, dict) and counts.get("discovered") == publications + 1
            and counts.get("searchable") == 1 and counts.get("ocr") == publications
            and counts.get("failed") == 0 and counts.get("processed") == publications + 1
        )
        ready = (
            status.get("index_health") == "ready"
            and bounded_int(status.get("visible_epoch")) > 0
            and status.get("searchable_documents") == 1
            and status.get("indexed_documents") == 1
            and status.get("ocr_queue_depth") == publications
        )
        if complete and ready:
            return status
        time.sleep(0.05)
    raise ExperimentError("seed_publication_timeout")


def query_payload(query: str, mode: str, request_id: str) -> dict[str, object]:
    return {
        "schema_version": "resume-ir.ipc-request.v3",
        "request_id": request_id,
        "client_capability": "benchmark",
        "deadline_ms": DEADLINE_MS,
        "payload": {"query": query, "mode": mode, "top_k": 1, "filters": {}},
    }


def error_sample(outcome: str, wall: float) -> dict[str, object]:
    return {
        "outcome": outcome, "client_wall_ms": round(max(wall, 0.001), 6),
        "server_latency_ms": None, "stage_latency_ms": None,
        "unclassified_wall_ms": None,
    }


def completed_sample(outcome: str, wall: float, server: float,
                     stages: dict[str, float]) -> dict[str, object]:
    residual = max(server - sum(stages.values()), 0.0)
    return {
        "outcome": outcome, "client_wall_ms": round(max(wall, 0.001), 6),
        "server_latency_ms": server, "stage_latency_ms": stages,
        "unclassified_wall_ms": round(residual, 6),
    }


def classify_response(http_status: int, body: dict[str, object], wall: float,
                      request_id: str, mode: str, epoch: int,
                      expected: tuple[str, str]) -> tuple[dict[str, object], tuple[str, str] | None]:
    error = body.get("error")
    if http_status == 503 and isinstance(error, dict) and error.get("code") == "OVERLOADED":
        return error_sample("overload", wall), None
    if http_status != 200:
        return error_sample("http_error", wall), None
    root_fields = {
        "schema_version", "request_id", "status", "visible_epoch", "query_mode",
        "partial", "partial_reasons", "latency_ms", "stage_latency_ms",
        "search_index", "result_count", "results",
    }
    response_mode = "keyword" if mode == "fulltext" else mode
    reasons, results = body.get("partial_reasons"), body.get("results")
    server = finite(body.get("latency_ms"))
    raw_stages = body.get("stage_latency_ms")
    if (
        set(body) != root_fields or body.get("schema_version") != "resume-ir.search-response.v3"
        or body.get("request_id") != request_id or body.get("query_mode") != response_mode
        or body.get("visible_epoch") != epoch or not isinstance(reasons, list)
        or any(reason not in {"deadline_exceeded", "embedding_runtime_unavailable"} for reason in reasons)
        or not isinstance(results, list) or body.get("result_count") != len(results)
        or server is None or not isinstance(raw_stages, dict) or set(raw_stages) != set(STAGES)
    ):
        return error_sample("protocol_error", wall), None
    stages = {key: finite(raw_stages[key]) for key in STAGES}
    if any(value is None for value in stages.values()):
        return error_sample("protocol_error", wall), None
    retained_stages = {key: round(float(stages[key]), 6) for key in STAGES}
    retained_server = round(server, 6)
    status = body.get("status")
    if status == "cancelled":
        return completed_sample("cancelled", wall, retained_server, retained_stages), None
    if status != "ok" or body.get("search_index") != "available":
        return error_sample("protocol_error", wall), None
    if body.get("partial") is not bool(reasons):
        return error_sample("protocol_error", wall), None
    if "deadline_exceeded" in reasons:
        return completed_sample("deadline_partial", wall, retained_server, retained_stages), None
    if "embedding_runtime_unavailable" in reasons:
        return completed_sample("semantic_partial", wall, retained_server, retained_stages), None
    if reasons or len(results) != 1:
        return error_sample("protocol_error", wall), None
    result = results[0]
    selection = result.get("selection") if isinstance(result, dict) else None
    signature = None
    if (
        isinstance(result, dict) and result.get("rank") == 1
        and isinstance(selection, dict) and selection.get("visible_epoch") == epoch
        and isinstance(selection.get("doc_id"), str)
        and isinstance(selection.get("version_id"), str)
    ):
        signature = (selection["doc_id"], selection["version_id"])
    if signature is None:
        return error_sample("protocol_error", wall), None
    outcome = "exact_expected" if signature == expected else "valid_epoch_result_change"
    return completed_sample(outcome, wall, retained_server, retained_stages), signature


def observe_query(transport: object, query: str, mode: str, request_id: str, epoch: int,
                  expected: tuple[str, str]) -> tuple[dict[str, object], tuple[str, str] | None]:
    started = time.perf_counter()
    try:
        status, body = transport.post_json(  # type: ignore[attr-defined]
            "/search", query_payload(query, mode, request_id)
        )
    except HarnessError as error:
        wall = (time.perf_counter() - started) * 1_000.0
        outcome = "transport_error" if str(error) == "ipc_request_failed" else "protocol_error"
        return error_sample(outcome, wall), None
    wall = (time.perf_counter() - started) * 1_000.0
    return classify_response(status, body, wall, request_id, mode, epoch, expected)


def valid_probe_body(request_id: str, mode: str = "hybrid", epoch: int = 7) -> dict[str, object]:
    return {
        "schema_version": "resume-ir.search-response.v3", "request_id": request_id,
        "status": "ok", "visible_epoch": epoch, "query_mode": mode,
        "partial": False, "partial_reasons": [], "latency_ms": 2.0,
        "stage_latency_ms": {key: 0.1 for key in STAGES}, "search_index": "available",
        "result_count": 1, "results": [{"rank": 1, "file_name": "synthetic",
        "snippet": "synthetic", "selection": {"doc_id": "d", "version_id": "v",
        "visible_epoch": epoch}}],
    }


def classification_probes() -> tuple[bool, bool, bool]:
    oracle, _ = classify_response(
        200, valid_probe_body("oracle"), 2.1, "oracle", "hybrid", 7, ("other", "other")
    )
    protocol_body = valid_probe_body("wrong")
    protocol, _ = classify_response(
        200, protocol_body, 2.1, "expected", "hybrid", 7, ("d", "v")
    )

    class BrokenTransport:
        def post_json(self, _path: str, _payload: dict[str, object]) -> object:
            raise HarnessError("ipc_request_failed")

    transport, _ = observe_query(
        BrokenTransport(), "synthetic", "hybrid", "transport", 7, ("d", "v")
    )
    return (
        oracle["outcome"] == "valid_epoch_result_change",
        transport["outcome"] == "transport_error",
        protocol["outcome"] == "protocol_error",
    )


def status_epoch(status: dict[str, object]) -> int:
    return bounded_int(status.get("visible_epoch"))


def resources(process: object, before: dict[str, object], after: dict[str, object]) -> dict[str, object]:
    root = process.process.pid  # type: ignore[attr-defined]
    try:
        output = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,rss="], check=True, text=True,
            capture_output=True, timeout=10,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        raise ExperimentError("process_tree_rss_unavailable") from None
    table: dict[int, tuple[int, int]] = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) == 3 and all(value.isdigit() for value in fields):
            table[int(fields[0])] = (int(fields[1]), int(fields[2]))
    members, frontier = {root}, [root]
    while frontier:
        parent = frontier.pop()
        children = [pid for pid, (ppid, _) in table.items() if ppid == parent]
        members.update(children)
        frontier.extend(children)
    if root not in table:
        raise ExperimentError("process_tree_rss_unavailable")
    rss = sum(table[pid][1] for pid in members if pid in table) / 1024.0
    return {
        "process_tree_rss_mib": round(rss, 3),
        "normalized_one_minute_load": round(os.getloadavg()[0] / max(os.cpu_count() or 1, 1), 6),
        "status_embedding_queue_depth_before": bounded_int(before.get("embedding_queue_depth")),
        "status_embedding_queue_depth_after": bounded_int(after.get("embedding_queue_depth")),
        "ocr_queue_depth_after": bounded_int(after.get("ocr_queue_depth")),
    }


def observe_group(transport: object, process: object, query: str, expected: tuple[str, str],
                  epoch: int, order: tuple[str, ...], tag: str) -> dict[str, object]:
    before = transport.get_json("/status")  # type: ignore[attr-defined]
    if status_epoch(before) != epoch:
        raise ExperimentError("publication_epoch_raced")
    samples: dict[str, dict[str, object]] = {}
    for mode in order:
        pair = {}
        for phase in ("first", "warm"):
            sample, _ = observe_query(
                transport, query, mode, f"{tag}-{mode}-{phase}", epoch, expected
            )
            pair[phase] = sample
        samples[mode] = pair
    after = transport.get_json("/status")  # type: ignore[attr-defined]
    if status_epoch(after) != epoch:
        raise ExperimentError("publication_epoch_raced")
    return {"mode_order": list(order), "samples": samples,
            "resources": resources(process, before, after)}


def warm_anchor(transport: object, query: str, epoch: int, timeout: float) -> tuple[str, str]:
    deadline, expected, attempt = time.monotonic() + timeout, None, 0
    while time.monotonic() < deadline:
        attempt += 1
        sample, signature = observe_query(
            transport, query, "fulltext", f"anchor-{attempt}", epoch, expected or ("", "")
        )
        if signature is not None and sample["outcome"] in VALID_COMPLETIONS:
            expected = signature
            break
        time.sleep(0.05)
    if expected is None:
        raise ExperimentError("anchor_not_searchable")
    for mode in MODES:
        sample, _ = observe_query(transport, query, mode, f"warm-{mode}", epoch, expected)
        if sample["outcome"] != "exact_expected":
            raise ExperimentError("anchor_not_dominant")
    return expected


def wait_epoch(transport: object, process: object, previous: int,
               timeout: float) -> dict[str, object]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.process.poll() is not None:  # type: ignore[attr-defined]
            raise ExperimentError("daemon_exited_during_publication")
        status = transport.get_json("/status")  # type: ignore[attr-defined]
        epoch = status_epoch(status)
        if epoch > previous + 1:
            raise ExperimentError("publication_epoch_skipped")
        if epoch == previous + 1:
            return status
        time.sleep(0.02)
    raise ExperimentError("publication_timeout")


def latency_summary(values: list[float]) -> dict[str, float]:
    ordered = sorted(values)
    def percentile(fraction: float) -> float:
        return ordered[max(math.ceil(len(ordered) * fraction) - 1, 0)]
    return {"p50": round(percentile(0.50), 3), "p95": round(percentile(0.95), 3),
            "p99": round(percentile(0.99), 3), "max": round(max(ordered), 3)}


def aggregate(mode: str, control: dict[str, object], epochs: list[dict[str, object]]) -> tuple[dict[str, object], int]:
    control_pair = control["samples"][mode]  # type: ignore[index]
    control_delta = control_pair["first"]["client_wall_ms"] - control_pair["warm"]["client_wall_ms"]
    control_residual = control_pair["first"]["unclassified_wall_ms"] - control_pair["warm"]["unclassified_wall_ms"]
    firsts, warms, deltas, excesses, residuals = [], [], [], [], []
    complete, signals = 0, 0
    for epoch in epochs:
        pair = epoch["samples"][mode]  # type: ignore[index]
        first, warm = pair["first"], pair["warm"]
        first_wall, warm_wall = first["client_wall_ms"], warm["client_wall_ms"]
        delta = first_wall - warm_wall
        excess = delta - control_delta
        complete_pair = first["outcome"] in VALID_COMPLETIONS and warm["outcome"] in VALID_COMPLETIONS
        if complete_pair:
            complete += 1
            residual = first["unclassified_wall_ms"] - warm["unclassified_wall_ms"] - control_residual
            signalled = excess >= 20 and first_wall / warm_wall >= 1.5 and residual >= 15
        else:
            residual, signalled = 0.0, True
        firsts.append(first_wall); warms.append(warm_wall); deltas.append(delta)
        excesses.append(excess); residuals.append(residual); signals += int(signalled)
    return {
        "epoch_pairs": len(epochs), "complete_epoch_pairs": complete,
        "first_client_ms": latency_summary(firsts), "warm_client_ms": latency_summary(warms),
        "first_minus_warm_ms": latency_summary(deltas),
        "excess_over_control_ms": latency_summary(excesses),
        "unclassified_excess_over_control_ms": latency_summary(residuals),
        "signal_epochs": signals,
    }, signals


def build_report(inputs: Inputs, kind: str, control: dict[str, object],
                 epochs: list[dict[str, object]], probes: tuple[bool, bool, bool],
                 cleanup: bool) -> dict[str, object]:
    samples = [pair[phase] for group in [control, *epochs]
               for pair in group["samples"].values() for phase in ("first", "warm")]  # type: ignore[union-attr]
    counts = Counter(sample["outcome"] for sample in samples)
    aggregates, signals = {}, {}
    for mode in MODES:
        aggregates[mode], signals[mode] = aggregate(mode, control, epochs)
    fulltext_signal = signals["fulltext"] >= 2
    semantic_signal = signals["semantic"] >= 2 or signals["hybrid"] >= 2
    if kind == "smoke":
        status = "smoke_pass" if counts["exact_expected"] == len(samples) else "smoke_failed"
        action, diagnosed_fulltext, diagnosed_semantic = None, False, False
    else:
        action = (
            "combined_bounded_fix_issue" if fulltext_signal and semantic_signal
            else "generation_publication_fix_issue" if fulltext_signal
            else "resident_isolation_rerun" if semantic_signal
            else "no_reproduced_product_defect"
        )
        status = "diagnosed" if fulltext_signal or semantic_signal else "not_reproduced"
        diagnosed_fulltext, diagnosed_semantic = fulltext_signal, semantic_signal
    loads = [group["resources"]["normalized_one_minute_load"] for group in [control, *epochs]]  # type: ignore[index]
    report = {
        "schema_version": SCHEMA, "artifact_id": "ocr-query-publication-issue-342",
        "issue": "#342", "source": "public_synthetic_repeated_ocr_publication",
        "revision": inputs.revision,
        "platform": {"os": "macos", "architecture": "arm64", "machine": "M4",
                     "governor": "H2_Aggressive", "memory_measurement": "process_tree_rss_mib"},
        "run": {"kind": kind, "seed": SEED, "publication_count": len(epochs),
                "query_attempts": len(samples), "all_publications_observed": True,
                "epoch_steps_exact": True, "oracle_corruption_probe_passed": probes[0],
                "transport_probe_passed": probes[1], "protocol_probe_passed": probes[2],
                "process_cleanup_passed": cleanup,
                "host_load_guard_passed": max(loads) - min(loads) <= 0.25},
        "fixed_workload": FIXED_WORKLOAD, "observability": OBSERVABILITY,
        "stable_control": control, "epochs": epochs,
        "outcomes": {"attempted": len(samples), "counts": {key: counts[key] for key in OUTCOMES},
                     "completed_valid": counts["exact_expected"] + counts["valid_epoch_result_change"],
                     "degraded_or_failed": len(samples) - counts["exact_expected"]
                     - counts["valid_epoch_result_change"], "count_conserved": True},
        "mode_aggregates": aggregates,
        "diagnosis": {"status": status, "selected_next_action": action,
                      "fulltext_signal": diagnosed_fulltext,
                      "semantic_or_hybrid_signal": diagnosed_semantic,
                      "signal_epoch_counts": signals,
                      "outcome_integrity_passed": counts["protocol_error"] == 0,
                      "no_speedup_claim": True, "no_query_hot_path_acceptance_claim": True},
        "privacy": PRIVACY, "claims": CLAIMS,
    }
    validate_contract(report)
    return report


def run(inputs: Inputs, kind: str) -> dict[str, object]:
    publications = 1 if kind == "smoke" else 3
    probes = classification_probes()
    if not all(probes):
        raise ExperimentError("classification_probe_failed")
    with tempfile.TemporaryDirectory(prefix="resume-ir-ocr-query-publication-") as temporary:
        base = Path(temporary).resolve()
        data, root, staging, runtime = (
            base / name for name in ("data", "cohort", "staging", "runtime")
        )
        for directory in (data, root, staging, runtime):
            directory.mkdir()
        staged = []
        for source, name in (
            (inputs.daemon, "resume-daemon"),
            (inputs.embedding, "resume-embedding-runtime"),
            (inputs.renderer, "resume-pdf-render-runtime"),
        ):
            destination = runtime / name
            shutil.copy2(source, destination)
            destination.chmod(0o700)
            staged.append(destination)
        inputs = Inputs(staged[0], staged[1], inputs.embedding_dir, inputs.ocr, inputs.tessdata,
                        staged[2], inputs.pdfium_dir, inputs.classifier, inputs.output,
                        inputs.timeout, inputs.revision)
        query = "quasar observatory telemetry"
        anchor = (
            "SUMMARY\nSynthetic dominant retrieval anchor\nEXPERIENCE\n"
            + "Built deterministic retrieval systems with " + (query + " ") * 120
            + "\nEDUCATION\nSynthetic Institute\nSKILLS\nquasar observatory telemetry\n"
        )
        (root / "anchor.txt").write_text(anchor, encoding="utf-8")
        for index, scanned in enumerate(rasterize(inputs, staging, publications), 1):
            (root / f"scan-{index}.pdf").write_bytes(scanned)
        environment = daemon_environment(inputs)
        seed_process = ManagedProcess.start(daemon_command(inputs, data, False), environment)
        try:
            seed_transport, _ = OCR["wait_ready"](seed_process, data, inputs.timeout)
            root_id, ack = start_managed_scan(seed_transport, root, time.monotonic)
            seed_status = wait_seed(seed_transport, root_id, ack.task, publications, inputs.timeout)
            seed_epoch = status_epoch(seed_status)
        finally:
            seed_process.cleanup()
        if group_exists(seed_process.group):
            raise ExperimentError("seed_process_cleanup_failed")
        process = ManagedProcess.start(daemon_command(inputs, data, True), environment)
        cleanup = False
        try:
            transport, ready = OCR["wait_ready"](process, data, inputs.timeout)
            if status_epoch(ready) != seed_epoch or ready.get("ocr_queue_depth") != publications:
                raise ExperimentError("stable_control_fence_failed")
            expected = warm_anchor(transport, query, seed_epoch, min(inputs.timeout, 20.0))
            control = observe_group(
                transport, process, query, expected, seed_epoch, MODES, "control"
            )
            if any(sample["outcome"] != "exact_expected"
                   for pair in control["samples"].values() for sample in pair.values()):  # type: ignore[union-attr]
                raise ExperimentError("stable_control_not_exact")
            after_control = transport.get_json("/status")
            if status_epoch(after_control) != seed_epoch or after_control.get("ocr_queue_depth") != publications:
                raise ExperimentError("ocr_started_before_control")
            epochs, previous = [], seed_epoch
            for ordinal in range(1, publications + 1):
                observed = wait_epoch(transport, process, previous, inputs.timeout)
                previous = status_epoch(observed)
                group = observe_group(
                    transport, process, query, expected, previous,
                    SCHEDULE[ordinal - 1], f"epoch-{ordinal}",
                )
                group["ordinal"] = ordinal
                epochs.append(group)
        finally:
            process.cleanup()
            cleanup = not group_exists(process.group)
    return build_report(inputs, kind, control, epochs, probes, cleanup)


class SelfTests(unittest.TestCase):
    def test_classification_probes(self) -> None:
        self.assertEqual(classification_probes(), (True, True, True))

    def test_completed_residual_is_recomputed(self) -> None:
        sample, signature = classify_response(
            200, valid_probe_body("exact"), 2.2, "exact", "hybrid", 7, ("d", "v")
        )
        self.assertEqual(sample["outcome"], "exact_expected")
        self.assertEqual(signature, ("d", "v"))
        self.assertAlmostEqual(sample["unclassified_wall_ms"], 1.3)

    def test_http_and_overload_are_distinct(self) -> None:
        overloaded, _ = classify_response(
            503, {"error": {"code": "OVERLOADED"}}, 1.0, "r", "hybrid", 1, ("d", "v")
        )
        rejected, _ = classify_response(500, {}, 1.0, "r", "hybrid", 1, ("d", "v"))
        self.assertEqual((overloaded["outcome"], rejected["outcome"]), ("overload", "http_error"))

    def test_pdf_round_trip_shape(self) -> None:
        ppm = b"P6\n2 1\n255\n" + b"\x00" * 6
        self.assertTrue(image_pdf(ppm).startswith(b"%PDF-1.4"))
        self.assertTrue(text_pdf(1).endswith(b"%%EOF\n"))


def inputs_from(arguments: argparse.Namespace) -> Inputs:
    paths = (
        arguments.daemon_bin, arguments.embedding_bin, arguments.embedding_runtime_dir,
        arguments.ocr_bin, arguments.tessdata_dir, arguments.pdf_render_bin,
        arguments.pdfium_runtime_dir, arguments.classifier_model,
    )
    if any(not path.exists() for path in paths):
        raise ExperimentError("required_input_missing")
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise ExperimentError("unsupported_platform")
    return Inputs(
        *(path.resolve() for path in paths), arguments.output.resolve(),
        arguments.timeout_seconds, exact_revision(),
    )


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    commands.add_parser("self-test")
    for name in ("smoke", "formal"):
        command = commands.add_parser(name)
        command.add_argument("--daemon-bin", type=Path, required=True)
        command.add_argument("--embedding-bin", type=Path, required=True)
        command.add_argument("--embedding-runtime-dir", type=Path, required=True)
        command.add_argument("--ocr-bin", type=Path, required=True)
        command.add_argument("--tessdata-dir", type=Path, required=True)
        command.add_argument("--pdf-render-bin", type=Path, required=True)
        command.add_argument("--pdfium-runtime-dir", type=Path, required=True)
        command.add_argument("--classifier-model", type=Path, required=True)
        command.add_argument("--output", type=Path, required=True)
        command.add_argument("--timeout-seconds", type=float, default=300.0)
    return root


def main() -> int:
    arguments = parser().parse_args()
    if arguments.command == "self-test":
        result = unittest.TextTestRunner(verbosity=2).run(
            unittest.defaultTestLoader.loadTestsFromTestCase(SelfTests)
        )
        return 0 if result.wasSuccessful() else 1
    try:
        inputs = inputs_from(arguments)
        report = run(inputs, "smoke" if arguments.command == "smoke" else "formal_public_witness")
        encoded = json.dumps(report, allow_nan=False, sort_keys=True, separators=(",", ":"))
        inputs.output.parent.mkdir(parents=True, exist_ok=True)
        inputs.output.write_text(encoded + "\n", encoding="utf-8")
        inputs.output.chmod(0o600)
        print(json.dumps({"status": "ok", "diagnosis": report["diagnosis"],
                          "outcomes": report["outcomes"]}, separators=(",", ":")))
        return 0
    except (ExperimentError, HarnessError, ValueError) as error:
        reason = str(error)
        if not reason.replace("_", "").isalnum() or len(reason) > 80:
            reason = "contract_validation_failed"
        print(json.dumps({"status": "failed", "reason": reason}, separators=(",", ":")))
        return 2
    except KeyboardInterrupt:
        print('{"status":"failed","reason":"interrupted"}')
        return 130
    except Exception:
        print('{"status":"failed","reason":"internal_failure"}')
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
