#!/usr/bin/env python3
"""Run a bounded OCR-on import attribution smoke or AB/BA witness."""
from __future__ import annotations
import argparse, http.client, json, math
import os, shutil, signal, subprocess, sys, tempfile, time, unittest
from dataclasses import dataclass
from pathlib import Path
from typing import Callable
from unittest import mock
from urllib.parse import urlsplit
CONTROL_REVISION, CANDIDATE_REVISION = (
    "974533dd7fce3994ca147d89dd227ff520d2cae8",
    "96a40d977fddd5f5465f9efe607263984601b114",
)
IPC_PROTOCOL, REPORT_SCHEMA = "resume-ir.daemon-ipc.v5", "resume-ir.ocr-import-attribution-witness.v1"
MODEL_ID, MODEL_DIMENSION, BATCH_BOUND = "intfloat-multilingual-e5-small-qint8-r1", 384, 4
OVERHEAD_LIMIT_PCT, MAX_PAIR_SPREAD_PP, MAX_LOAD_DRIFT = 3.0, 3.0, 0.25
MIN_SIGNAL_MS, MAX_BODY_BYTES, MAX_REPORT_BYTES = 500.0, 2 * 1024 * 1024, 64 * 1024
POLL_PATHS = ("/imports/progress", "/source-roots", "/status")
PRIVACY = {
    "raw_resume_text": False, "raw_query_text": False, "candidate_results": False,
    "local_paths": False, "tokens": False, "diagnostics_package": False, "model_cache": False,
}
SAFE_SENSITIVE_AGGREGATES = {
    "active_token_count", "padded_token_count", "tokenize_us", "vector_publication_us",
    "fulltext_setup_us", "fulltext_documents_us", "fulltext_commit_us",
    "fulltext_plaintext_validation_us", "fulltext_encrypted_publication_us",
    "fulltext_encrypted_validation_us", "fulltext_atomic_publication_us",
}
class HarnessError(RuntimeError):
    """A fixed, public-safe failure code."""
@dataclass(frozen=True)
class Ack:
    task: str; start: float; accepted: float
    @property
    def milliseconds(self) -> float:
        return (self.accepted - self.start) * 1_000.0
@dataclass(frozen=True)
class Variant:
    label: str; daemon: Path; embedding: Path; cli: Path; attribution: bool
@dataclass(frozen=True)
class Inputs:
    embedding_dir: Path; ocr: Path; tessdata: Path; renderer: Path
    pdfium_dir: Path; classifier: Path; scanned: Path
    direct_count: int; timeout: float; poll_interval: float
class OcrOverlap:
    def __init__(self, initial: int) -> None:
        self.previous = initial
        self.rise = False
        self.decline_after_rise = False
    def observe_before_complete(self, depth: int) -> None:
        self.rise = self.rise or depth > self.previous
        self.decline_after_rise = self.decline_after_rise or (
            self.rise and depth < self.previous
        )
        self.previous = depth
    def classify(self, endpoint_depth: int) -> str:
        if self.decline_after_rise:
            return "observed"
        if endpoint_depth > 0:
            return "not_observed"
        return "unobservable"
class HttpTransport:
    def __init__(self, endpoint: str, token: str, timeout: float) -> None:
        parsed = urlsplit(endpoint)
        if (
            parsed.scheme != "http"
            or parsed.hostname not in {"127.0.0.1", "::1", "localhost"}
            or parsed.port is None
        ):
            raise HarnessError("invalid_ipc_endpoint")
        self.host, self.port, self.token = parsed.hostname, parsed.port, token
        self.timeout = min(timeout, 10.0)
    def request(
        self, method: str, path: str, payload: dict[str, object] | None = None
    ) -> tuple[int, dict[str, object]]:
        encoded = None if payload is None else json.dumps(payload).encode()
        headers = {"Authorization": f"Bearer {self.token}"}
        if encoded is not None:
            headers["Content-Type"] = "application/json"
        connection = http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)
        try:
            connection.request(method, path, body=encoded, headers=headers)
            response = connection.getresponse()
            body = response.read(MAX_BODY_BYTES + 1)
            status = response.status
        except (OSError, http.client.HTTPException):
            raise HarnessError("ipc_request_failed") from None
        finally:
            connection.close()
        if len(body) > MAX_BODY_BYTES:
            raise HarnessError("ipc_response_too_large")
        try:
            lines = [line for line in body.splitlines() if line.strip()]
            value = json.loads(lines[-1] if path == POLL_PATHS[0] else body)
        except (IndexError, UnicodeDecodeError, json.JSONDecodeError):
            raise HarnessError("invalid_ipc_response") from None
        if not isinstance(value, dict):
            raise HarnessError("invalid_ipc_response")
        return status, value
    def get_json(self, path: str) -> dict[str, object]:
        status, value = self.request("GET", path)
        if status != 200:
            raise HarnessError("ipc_get_rejected")
        return value
    def post_json(self, path: str, payload: dict[str, object]) -> tuple[int, dict[str, object]]:
        return self.request("POST", path, payload)
class ManagedProcess:
    def __init__(self, process: subprocess.Popen[bytes], stderr: object) -> None:
        self.process, self.group, self.stderr = process, process.pid, stderr
    @classmethod
    def start(cls, command: list[str], environment: dict[str, str]) -> "ManagedProcess":
        stderr = tempfile.TemporaryFile()
        try:
            process = subprocess.Popen(
                command,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=stderr,
                start_new_session=True,
            )
        except (OSError, ValueError):
            stderr.close()
            raise HarnessError("daemon_start_failed") from None
        return cls(process, stderr)
    def cleanup(self) -> None:
        try:
            for signal_value in (signal.SIGTERM, signal.SIGKILL):
                if group_exists(self.group):
                    os.killpg(self.group, signal_value)
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    continue
                if not group_exists(self.group):
                    break
            deadline = time.monotonic() + 3
            while group_exists(self.group) and time.monotonic() < deadline:
                time.sleep(0.02)
            if group_exists(self.group):
                raise HarnessError("process_cleanup_failed")
        finally:
            self.stderr.close()  # type: ignore[attr-defined]
def group_exists(group: int) -> bool:
    try:
        os.killpg(group, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return False
    return True
def bounded_int(value: object) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or not 0 <= value < 2**63:
        raise HarnessError("invalid_numeric_aggregate")
    return value
def start_managed_scan(transport: object, root: Path, clock: Callable[[], float]) -> tuple[str, Ack]:
    payload = {
        "schema_version": "resume-ir.source-root-register-request.v1",
        "requested_path": str(root), "display_label": "Import attribution cohort",
    }
    deadline, root_id = time.monotonic() + 5.0, None
    while root_id is None:
        status, body = transport.post_json("/source-roots/register", payload)  # type: ignore[attr-defined]
        managed = body.get("root") if isinstance(body, dict) else None
        candidate = managed.get("root_id") if isinstance(managed, dict) else None
        if status == 200 and isinstance(candidate, str) and 0 < len(candidate) <= 128:
            root_id = candidate
            break
        if status != 503:
            raise HarnessError(f"source_root_registration_http_{status}")
        list_status, listed = transport.request("GET", "/source-roots")  # type: ignore[attr-defined]
        values = listed.get("roots") if list_status == 200 else None
        if not isinstance(values, list):
            raise HarnessError("source_root_registration_reconciliation_failed")
        matches = [item.get("root_id") for item in values if isinstance(item, dict) and item.get("display_label") == payload["display_label"]]
        if len(matches) == 1 and isinstance(matches[0], str):
            root_id = matches[0]
            break
        if values or time.monotonic() >= deadline:
            raise HarnessError("source_root_registration_reconciliation_failed")
        time.sleep(0.1)
    start = clock()
    status, body = transport.post_json("/source-roots/scan", {  # type: ignore[attr-defined]
        "schema_version": "resume-ir.source-root-scan-request.v1", "root_id": root_id,
    })
    accepted, managed = clock(), body.get("root") if isinstance(body, dict) else None
    scan = managed.get("last_scan") if isinstance(managed, dict) else None
    task = scan.get("scan_id") if isinstance(scan, dict) else None
    if status != 200 or not isinstance(managed, dict) or managed.get("root_id") != root_id or not isinstance(task, str) or not 0 < len(task) <= 128:
        raise HarnessError("source_root_scan_not_accepted")
    return root_id, Ack(task, start, accepted)
def poll_round(
    transport: object, clock: Callable[[], float] = time.monotonic
) -> tuple[dict[str, object], dict[str, object], dict[str, object], float]:
    progress = transport.get_json(POLL_PATHS[0])  # type: ignore[attr-defined]
    roots = transport.get_json(POLL_PATHS[1])  # type: ignore[attr-defined]
    observed = clock()
    status = transport.get_json(POLL_PATHS[2])  # type: ignore[attr-defined]
    return progress, roots, status, observed
def stage_metrics(attribution: dict[str, object]) -> dict[str, int | bool | None]:
    embedding, vector, publication = (
        attribution.get("embedding"),
        attribution.get("vector"),
        attribution.get("publication"),
    )
    if not all(isinstance(value, dict) for value in (embedding, vector, publication)):
        raise HarnessError("invalid_attribution")
    fulltext = publication.get("fulltext")  # type: ignore[union-attr]
    if not isinstance(fulltext, dict):
        raise HarnessError("invalid_attribution")
    batches = bounded_int(embedding.get("batch_count"))  # type: ignore[union-attr]
    inputs = bounded_int(embedding.get("input_count"))  # type: ignore[union-attr]
    return {
        "available": True,
        "batch_count": batches,
        "input_count": inputs,
        "batch_bound": BATCH_BOUND,
        "batch_bound_respected": inputs <= batches * BATCH_BOUND,
        "active_token_count": bounded_int(embedding.get("active_token_count")),  # type: ignore[union-attr]
        "padded_token_count": bounded_int(embedding.get("padded_token_count")),  # type: ignore[union-attr]
        "queue_wait_us": bounded_int(embedding.get("queue_wait_us")),  # type: ignore[union-attr]
        "ipc_wall_us": bounded_int(embedding.get("ipc_wall_us")),  # type: ignore[union-attr]
        "resident_request_us": bounded_int(embedding.get("request_wall_us")),  # type: ignore[union-attr]
        "child_total_us": bounded_int(embedding.get("child_total_us")),  # type: ignore[union-attr]
        "tokenize_us": bounded_int(embedding.get("tokenize_us")),  # type: ignore[union-attr]
        "tensor_us": bounded_int(embedding.get("tensor_us")),  # type: ignore[union-attr]
        "onnx_us": bounded_int(embedding.get("onnx_us")),  # type: ignore[union-attr]
        "pool_us": bounded_int(embedding.get("pool_us")),  # type: ignore[union-attr]
        "normalize_us": bounded_int(embedding.get("normalize_us")),  # type: ignore[union-attr]
        "vector_publication_us": bounded_int(vector.get("publication_wall_us")),  # type: ignore[union-attr]
        "non_request_publication_us": bounded_int(vector.get("non_embedding_wall_us")),  # type: ignore[union-attr]
        "owner_wait_us": bounded_int(publication.get("owner_wait_us")),  # type: ignore[union-attr]
        "metadata_commit_us": bounded_int(publication.get("metadata_decision_commit_us")),  # type: ignore[union-attr]
        **{f"fulltext_{key}": bounded_int(fulltext.get(key)) for key in (
            "setup_us", "documents_us", "commit_us", "plaintext_validation_us",
            "encrypted_publication_us", "encrypted_validation_us", "atomic_publication_us",
        )},
    }
def empty_stage() -> dict[str, int | bool | None]:
    keys = (
        "batch_count", "input_count", "active_token_count", "padded_token_count",
        "queue_wait_us", "ipc_wall_us", "resident_request_us", "child_total_us",
        "tokenize_us", "tensor_us", "onnx_us", "pool_us", "normalize_us",
        "vector_publication_us", "non_request_publication_us", "owner_wait_us", "metadata_commit_us",
        "fulltext_setup_us", "fulltext_documents_us", "fulltext_commit_us",
        "fulltext_plaintext_validation_us", "fulltext_encrypted_publication_us",
        "fulltext_encrypted_validation_us", "fulltext_atomic_publication_us",
    )
    return {
        "available": False,
        **{key: None for key in keys},
        "batch_bound": BATCH_BOUND,
        "batch_bound_respected": True,
    }
def evaluate_round(
    root_id: str, task: str,
    progress: dict[str, object],
    roots: dict[str, object],
    status: dict[str, object],
    require_attribution: bool,
) -> tuple[dict[str, int], dict[str, int | bool | None], dict[str, bool]] | None:
    values = roots.get("roots")
    if not isinstance(values, list):
        raise HarnessError("invalid_source_root_response")
    matches = [
        item["last_scan"]
        for item in values
        if isinstance(item, dict)
        and item.get("root_id") == root_id
        and isinstance(item.get("last_scan"), dict)
        and item["last_scan"].get("scan_id") == task
    ]
    if len(matches) > 1:
        raise HarnessError("ambiguous_source_root_fence")
    if not matches:
        return None
    scan = matches[0]
    if scan.get("phase") in {"failed", "partial"}:
        raise HarnessError("source_root_failed")
    if scan.get("phase") != "complete" or scan.get("completeness") != "complete":
        return None
    raw_counts = scan.get("counts")
    if not isinstance(raw_counts, dict):
        raise HarnessError("invalid_source_root_counts")
    fields = {
        "discovered": "discovered", "searchable": "searchable", "ocr_required": "ocr",
        "failed": "failed", "ignored": "ignored", "processed": "processed",
    }
    counts = {public: bounded_int(raw_counts.get(source)) for public, source in fields.items()}
    scope = progress.get("latest_import_scan")
    if not isinstance(scope, dict):
        raise HarnessError("invalid_import_scope")
    scope_fields = {"discovered": "files_discovered", "searchable": "searchable_documents", "ocr_required": "ocr_required_documents", "failed": "failed_documents", "ignored": "ignored_entries"}
    scope_counts = {key: bounded_int(scope.get(value)) for key, value in scope_fields.items()}
    if any(scope_counts[key] != counts[key] for key in scope_fields):
        raise HarnessError("import_scope_mismatch")
    scope_queued = bounded_int(scope.get("ocr_jobs_queued"))
    attribution = progress.get("latest_import_attribution")
    if require_attribution:
        if not isinstance(attribution, dict) or attribution.get("task_id") != task:
            raise HarnessError("attribution_task_mismatch")
        ocr = attribution.get("ocr")
        if not isinstance(ocr, dict):
            raise HarnessError("invalid_attribution")
        required, queued = bounded_int(ocr.get("required_documents")), bounded_int(ocr.get("jobs_queued"))
        if queued != scope_queued:
            raise HarnessError("attribution_scope_mismatch")
        if bounded_int(attribution.get("searchable_documents")) != counts["searchable"]:
            raise HarnessError("attribution_count_mismatch")
        stage, binding = stage_metrics(attribution), True
    else:
        if attribution is not None:
            raise HarnessError("unexpected_control_attribution")
        required, queued = counts["ocr_required"], scope_queued
        stage, binding = empty_stage(), False
    indexed = bounded_int(status.get("indexed_documents"))
    searchable = bounded_int(status.get("searchable_documents"))
    visible = bounded_int(status.get("visible_epoch"))
    counts.update({"ocr_queued": queued, "indexed": indexed})
    invariants = {
        "source_fence": True,
        "task_binding": binding,
        "ocr_durable": required == counts["ocr_required"] > 0 and queued >= required,
        "visible_nonzero": visible > 0,
        "coverage_consistent": indexed == searchable and indexed >= counts["searchable"],
        "atomic_consistent": counts["processed"] == counts["discovered"],
        "batch_bound_respected": bool(stage["batch_bound_respected"]),
        "attribution_unavailable_by_contract": not require_attribution,
    }
    required_invariants = (
        "source_fence", "ocr_durable", "visible_nonzero", "coverage_consistent",
        "atomic_consistent", "batch_bound_respected",
    )
    if not all(invariants[key] for key in required_invariants):
        raise HarnessError("publication_invariant_failed")
    return counts, stage, invariants
def build_cohort(root: Path, direct_count: int, scanned: Path) -> dict[str, int]:
    if not 0 < direct_count <= 256 or not scanned.is_file():
        raise HarnessError("invalid_synthetic_inputs")
    root.mkdir()
    for index in range(direct_count):
        (root / f"direct-{index:04d}.txt").write_text(
            f"Synthetic Candidate {index:04d}\nProfessional Summary\n"
            f"Synthetic systems engineer identity {index:04d}.\nWork Experience\n"
            f"Built deterministic service family {index:04d} with Rust and Python.\n"
            f"Education\nSynthetic Institute cohort {index:04d}.\n"
            f"Skills\nRust Python SQL synthetic_skill_{index:04d}\n",
            encoding="utf-8",
        )
    shutil.copyfile(scanned, root / "scanned.pdf")
    (root / "empty.txt").write_bytes(b"")
    (root / ".DS_Store").write_bytes(b"synthetic ignored entry")
    return {
        "discovered": direct_count + 2,
        "searchable": direct_count,
        "ocr_required": 1,
        "failed": 1,
        "ignored": 1,
        "processed": direct_count + 2,
    }
def load_private_root() -> Path:
    value = os.environ.get("RESUME_IR_PRIVATE_RESUME_ROOT")
    root = Path(value) if value else None
    if root is None or not root.is_dir() or not os.access(root, os.R_OK):
        raise HarnessError("private_root_unavailable")
    return root.resolve()
def daemon_command(variant: Variant, inputs: Inputs, data: Path, classifier: Path) -> list[str]:
    return [
        str(variant.daemon), "--data-dir", str(data), "run", "--foreground",
        "--work-imports", "--work-index", "--rescan-completed-imports", "--watch-import-roots",
        "--import-rescan-min-age-seconds", "300", "--expected-ipc-protocol", IPC_PROTOCOL,
        "--ipc-listen", "127.0.0.1:0",
        "--embedding-command", str(variant.embedding), "--embedding-model-id", MODEL_ID,
        "--embedding-dimension", str(MODEL_DIMENSION), "--work-ocr",
        "--ocr-tesseract-command", str(inputs.ocr), "--ocr-lang", "eng+chi_sim",
        "--ocr-jobs-per-tick", "1", "--pdf-render-command", str(inputs.renderer),
        "--resume-classifier-model", str(classifier),
    ]
def read_small_json(path: Path) -> dict[str, object]:
    body = path.read_bytes()
    if len(body) > 64 * 1024:
        raise HarnessError("ipc_owner_file_too_large")
    value = json.loads(body)
    if not isinstance(value, dict):
        raise HarnessError("invalid_ipc_owner_file")
    return value
def wait_ready(process: ManagedProcess, data: Path, timeout: float) -> tuple[HttpTransport, dict[str, object]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.process.poll() is not None:
            raise HarnessError("daemon_exited_before_ready")
        try:
            endpoints = read_small_json(data / "ipc.endpoints.json")
            auth = read_small_json(data / "ipc.auth")
        except (FileNotFoundError, json.JSONDecodeError):
            time.sleep(0.02)
            continue
        if (
            endpoints.get("schema_version") != IPC_PROTOCOL
            or endpoints.get("launch_id") != auth.get("launch_id")
            or endpoints.get("instance_id") != auth.get("instance_id")
            or not isinstance(auth.get("token"), str)
            or not 16 <= len(auth["token"]) <= 512
        ):
            raise HarnessError("invalid_ipc_owner_file")
        expected = {"status": "/status", "source_root_register": "/source-roots/register", "source_root_scan": "/source-roots/scan", "import_progress": POLL_PATHS[0], "source_roots": POLL_PATHS[1]}
        parsed = {key: urlsplit(str(endpoints.get(key, ""))) for key in expected}
        status_url = parsed["status"]
        if any(
            value.scheme != "http"
            or value.hostname not in {"127.0.0.1", "::1", "localhost"}
            or value.hostname != status_url.hostname
            or value.port != status_url.port
            or value.path != expected[key]
            for key, value in parsed.items()
        ):
            raise HarnessError("invalid_ipc_endpoint")
        transport = HttpTransport(str(endpoints["status"]), auth["token"], timeout)  # type: ignore[arg-type]
        try:
            status = transport.get_json("/status")
        except HarnessError as error:
            if str(error) != "ipc_request_failed":
                raise
            time.sleep(0.02)
            continue
        runtimes, capabilities = status.get("optional_runtimes"), status.get("capabilities")
        runtime_ready = isinstance(runtimes, dict) and all(
            isinstance(runtimes.get(key), dict) and runtimes[key].get("state") == "available"
            for key in ("embedding", "ocr", "classifier", "pdfium")
        )
        capability_ready = isinstance(capabilities, dict) and all(
            isinstance(capabilities.get(key), dict) and capabilities[key].get("state") == "available"
            for key in ("text_import", "ocr_import", "index_publication")
        )
        if status.get("status") == "ok" and runtime_ready and capability_ready:
            return transport, status
        time.sleep(0.05)
    raise HarnessError("capability_attestation_timeout")
def host_load() -> dict[str, float | int]:
    cores = max(os.cpu_count() or 1, 1)
    one, five, _ = os.getloadavg()
    return {
        "logical_cores": cores,
        "one_minute": round(one, 4),
        "five_minute": round(five, 4),
        "normalized_one_minute": round(one / cores, 6),
    }
def run_once(variant: Variant, inputs: Inputs, private_root: Path | None = None) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="resume-ir-ocr-attribution-") as temporary:
        base = Path(temporary)
        data, root = base / "data", private_root or base / "cohort"
        data.mkdir()
        expected = None if private_root else build_cohort(root, inputs.direct_count, inputs.scanned)
        empty, classifier = base / "bootstrap-empty", base / "classifier-model.json"
        empty.mkdir(); shutil.copyfile(inputs.classifier, classifier); classifier.chmod(0o600)
        try:
            initialized = subprocess.run([str(variant.cli), "--data-dir", str(data), "import", "--root", str(empty), "--profile", "explicit", "--resume-classifier-model", str(classifier)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=inputs.timeout)
        except (OSError, subprocess.TimeoutExpired): raise HarnessError("store_initialization_failed") from None
        if initialized.returncode != 0: raise HarnessError("store_initialization_failed")
        environment = os.environ.copy()
        environment.update({
            "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(inputs.embedding_dir),
            "RESUME_IR_PDFIUM_RUNTIME_DIR": str(inputs.pdfium_dir),
            "TESSDATA_PREFIX": str(inputs.tessdata),
        })
        process = ManagedProcess.start(daemon_command(variant, inputs, data, inputs.classifier), environment)
        try:
            transport, ready = wait_ready(process, data, inputs.timeout)
            tracker = OcrOverlap(bounded_int(ready.get("ocr_queue_depth")))
            before = host_load()
            root_id, ack = start_managed_scan(transport, root, time.monotonic)
            deadline, rounds = ack.start + inputs.timeout, 0
            while time.monotonic() < deadline:
                round_start = time.monotonic()
                progress, roots, status, root_time = poll_round(transport)
                rounds += 1
                result = evaluate_round(root_id, ack.task, progress, roots, status, variant.attribution)
                depth = bounded_int(status.get("ocr_queue_depth"))
                if result is not None:
                    counts, stages, invariants = result
                    overlap, elapsed = tracker.classify(depth), (root_time - ack.start) * 1_000.0
                    break
                tracker.observe_before_complete(depth)
                if process.process.poll() is not None:
                    raise HarnessError("daemon_exited_during_import")
                time.sleep(max(0.0, inputs.poll_interval - (time.monotonic() - round_start)))
            else:
                raise HarnessError("import_endpoint_timeout")
            after = host_load()
        finally:
            process.cleanup()
    if expected is not None and any(counts[key] != value for key, value in expected.items()):
        raise HarnessError("synthetic_routing_mismatch")
    drift = abs(float(before["normalized_one_minute"]) - float(after["normalized_one_minute"]))
    return {
        "variant": variant.label,
        "elapsed_ms": round(elapsed, 3),
        "ack_ms": round(ack.milliseconds, 3),
        "poll_rounds": rounds,
        "ocr_overlap": overlap,
        "counts": counts,
        "stages": stages,
        "invariants": invariants,
        "host_load": {"before": before, "after": after, "normalized_drift": round(drift, 6)},
    }
def validate_report(report: dict[str, object]) -> None:
    if report.get("privacy") != PRIVACY:
        raise HarnessError("privacy_flags_invalid")
    forbidden = ("path", "name", "text", "hash", "query", "vector", "token", "cache", "runtime", "stderr", "endpoint", "address", "pid", "task_id", "file")
    def inspect(value: object, privacy: bool = False) -> None:
        if isinstance(value, dict):
            for key, item in value.items():
                child_privacy = privacy or key == "privacy"
                allowed_aggregate = key in SAFE_SENSITIVE_AGGREGATES
                if not child_privacy and not allowed_aggregate and any(fragment in key.lower() for fragment in forbidden):
                    raise HarnessError("redaction_violation")
                inspect(item, child_privacy)
        elif isinstance(value, list):
            for item in value:
                inspect(item, privacy)
        elif isinstance(value, str) and (
            "bearer " in value.lower() or "authorization:" in value.lower()
            or value.startswith(("/", "~", "file:")) or "\\" in value
        ):
            raise HarnessError("redaction_violation")
        elif isinstance(value, float) and not math.isfinite(value):
            raise HarnessError("non_finite_report_value")
    inspect(report)
    if len(json.dumps(report, allow_nan=False, separators=(",", ":")).encode()) > MAX_REPORT_BYTES:
        raise HarnessError("report_too_large")
def report_base(mode: str, direct_count: int) -> dict[str, object]:
    return {
        "schema_version": REPORT_SCHEMA,
        "mode": mode,
        "claim": "private_local_redacted" if mode == "private_local_redacted" else "synthetic_only",
        "direct_documents": direct_count,
        "product_workers": {"import": True, "ocr": True, "embedding": True, "index": True},
        "fixed_policy": {"embedding_batch_bound": BATCH_BOUND, "ocr_jobs_per_tick": 1},
        "privacy": PRIVACY,
    }
def overhead_decision(overheads: list[float], load_drift: float, signal_ms: float) -> str:
    if len(overheads) != 2 or any(not math.isfinite(value) for value in overheads):
        raise HarnessError("invalid_overhead_samples")
    if (
        signal_ms < MIN_SIGNAL_MS
        or load_drift > MAX_LOAD_DRIFT
        or abs(overheads[0] - overheads[1]) > MAX_PAIR_SPREAD_PP
    ):
        return "inconclusive"
    if max(overheads) <= OVERHEAD_LIMIT_PCT:
        return "pass"
    if min(overheads) > OVERHEAD_LIMIT_PCT:
        return "rollback_required"
    return "inconclusive"
def run_smoke(arguments: argparse.Namespace) -> dict[str, object]:
    inputs = inputs_from(arguments)
    private_root = load_private_root() if arguments.private else None
    run = run_once(Variant("candidate", arguments.daemon_bin, arguments.embedding_bin, arguments.resume_cli_bin, True), inputs, private_root)
    report = report_base("private_local_redacted" if private_root else "smoke", 0 if private_root else inputs.direct_count)
    report.update({"candidate_revision": CANDIDATE_REVISION, "runs": [run]})
    validate_report(report)
    return report
def run_witness(arguments: argparse.Namespace) -> dict[str, object]:
    inputs = inputs_from(arguments)
    private_root = load_private_root() if arguments.private else None
    control = Variant("control", arguments.control_daemon_bin, arguments.control_embedding_bin, arguments.control_resume_cli_bin, False)
    candidate = Variant("candidate", arguments.candidate_daemon_bin, arguments.candidate_embedding_bin, arguments.candidate_resume_cli_bin, True)
    runs = [run_once(variant, inputs, private_root) for variant in (control, candidate, candidate, control)]
    elapsed = [float(run["elapsed_ms"]) for run in runs]
    overheads = [(elapsed[1] / elapsed[0] - 1) * 100, (elapsed[2] / elapsed[3] - 1) * 100]
    drift = max(float(run["host_load"]["normalized_drift"]) for run in runs)  # type: ignore[index]
    decision = overhead_decision(overheads, drift, min(elapsed[0], elapsed[3]))
    geometric = (math.sqrt((elapsed[1] / elapsed[0]) * (elapsed[2] / elapsed[3])) - 1) * 100
    report = report_base("private_local_redacted" if private_root else "serialized_abba", 0 if private_root else inputs.direct_count)
    report.update({
        "control_revision": CONTROL_REVISION,
        "candidate_revision": CANDIDATE_REVISION,
        "sequence": ["control", "candidate", "candidate", "control"],
        "runs": runs,
        "comparison": {
            "paired_overhead_pct": [round(value, 3) for value in overheads],
            "geometric_overhead_pct": round(geometric, 3),
            "pair_spread_pp": round(abs(overheads[0] - overheads[1]), 3),
            "max_normalized_load_drift": round(drift, 6),
            "threshold_pct": OVERHEAD_LIMIT_PCT,
            "decision": decision,
        },
    })
    validate_report(report)
    return report
def inputs_from(arguments: argparse.Namespace) -> Inputs:
    paths = (
        arguments.embedding_runtime_dir, arguments.ocr_bin, arguments.tessdata_dir,
        arguments.pdf_render_bin, arguments.pdfium_runtime_dir, arguments.classifier_model,
        arguments.scanned_fixture,
    )
    if any(not path.exists() for path in paths):
        raise HarnessError("missing_required_input")
    resolved = tuple(path.resolve() for path in paths)
    return Inputs(*resolved, arguments.direct_documents, arguments.timeout_seconds, arguments.poll_ms / 1_000.0)
def test_payload(task: str) -> dict[str, object]:
    return {
        "task_id": task, "searchable_documents": 1,
        "ocr": {"required_documents": 1, "jobs_queued": 1},
        "embedding": {
            "batch_count": 1, "input_count": 1, "active_token_count": 3, "padded_token_count": 4,
            "queue_wait_us": 1, "ipc_wall_us": 2, "request_wall_us": 3, "child_total_us": 2,
            "tokenize_us": 1, "tensor_us": 1, "onnx_us": 1, "pool_us": 1, "normalize_us": 1,
        },
        "vector": {"publication_wall_us": 3, "non_embedding_wall_us": 1},
        "publication": {
            "owner_wait_us": 1, "metadata_decision_commit_us": 1,
            "fulltext": {key: 1 for key in (
                "setup_us", "documents_us", "commit_us", "plaintext_validation_us",
                "encrypted_publication_us", "encrypted_validation_us", "atomic_publication_us",
            )},
        },
    }
def test_round(root_id: str, task: str) -> tuple[dict[str, object], dict[str, object], dict[str, object]]:
    roots = {"roots": [{"root_id": root_id, "last_scan": {"scan_id": task, "phase": "complete", "completeness": "complete", "counts": {"discovered": 4, "searchable": 1, "ocr": 1, "failed": 1, "ignored": 1, "processed": 4}}}]}
    progress = {"latest_import_scan": {"files_discovered": 4, "searchable_documents": 1, "ocr_required_documents": 1, "failed_documents": 1, "ignored_entries": 1, "ocr_jobs_queued": 1}}
    status = {"ocr_jobs_queued": 0, "ocr_queue_depth": 1, "searchable_documents": 1, "indexed_documents": 1, "visible_epoch": 1}
    return progress, roots, status
class SelfTests(unittest.TestCase):
    def test_readiness_retries_only_transient_ipc_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory); paths = {"status": "/status", "source_root_register": "/source-roots/register", "source_root_scan": "/source-roots/scan", "import_progress": POLL_PATHS[0], "source_roots": POLL_PATHS[1]}
            endpoints = {"schema_version": IPC_PROTOCOL, "launch_id": "launch", "instance_id": "instance", **{key: f"http://127.0.0.1:321{path}" for key, path in paths.items()}}
            auth = {"launch_id": "launch", "instance_id": "instance", "token": "x" * 16}
            (data / "ipc.endpoints.json").write_text(json.dumps(endpoints)); (data / "ipc.auth").write_text(json.dumps(auth))
            process = mock.Mock(); process.process.poll.return_value = None
            ready = {"status": "ok", "optional_runtimes": {key: {"state": "available"} for key in ("embedding", "ocr", "classifier", "pdfium")}, "capabilities": {key: {"state": "available"} for key in ("text_import", "ocr_import", "index_publication")}}
            transport = mock.Mock(); transport.get_json.side_effect = [HarnessError("ipc_request_failed"), ready]
            with mock.patch(f"{__name__}.HttpTransport", return_value=transport), mock.patch.object(time, "sleep"):
                observed, _ = wait_ready(process, data, .1)
            self.assertIs(observed, transport); self.assertEqual(transport.get_json.call_count, 2)
            for key, value, reason in (("schema_version", "bad", "invalid_ipc_owner_file"), ("instance_id", "other", "invalid_ipc_owner_file"), ("status", "http://127.0.0.1:321/wrong", "invalid_ipc_endpoint")):
                broken = dict(endpoints); broken[key] = value; (data / "ipc.endpoints.json").write_text(json.dumps(broken))
                with self.assertRaisesRegex(HarnessError, reason): wait_ready(process, data, .01)
    def test_managed_scan_identity_and_t0(self) -> None:
        events: list[str] = []
        class Clock:
            values = iter((1.0, 1.025))
            def __call__(self) -> float: events.append("clock"); return next(self.values)
        class Transport:
            def post_json(self, path: str, payload: dict[str, object]) -> tuple[int, dict[str, object]]:
                events.append(path); self.payload = payload
                if path == "/source-roots/register": assert payload == {"schema_version": "resume-ir.source-root-register-request.v1", "requested_path": "/synthetic", "display_label": "Import attribution cohort"}; return 503, {}
                if path == "/source-roots/scan": assert payload == {"schema_version": "resume-ir.source-root-scan-request.v1", "root_id": "root-managed"}; return 200, {"root": {"root_id": "root-managed", "last_scan": {"scan_id": "managed-task"}}}
                raise AssertionError(path)
            def request(self, method: str, path: str) -> tuple[int, dict[str, object]]:
                events.append(path); self.method = method
                return 200, {"roots": [{"root_id": "root-managed", "display_label": "Import attribution cohort"}]}
        transport = Transport(); root_id, ack = start_managed_scan(transport, Path("/synthetic"), Clock())
        self.assertEqual(events, ["/source-roots/register", "/source-roots", "clock", "/source-roots/scan", "clock"])
        self.assertEqual(transport.method, "GET")
        self.assertEqual((root_id, ack.task), ("root-managed", "managed-task")); self.assertAlmostEqual(ack.milliseconds, 25)
    def test_poll_order_and_stale_attribution(self) -> None:
        class Transport:
            paths: list[str] = []
            def get_json(self, path: str) -> dict[str, object]: self.paths.append(path); return {}
        transport = Transport(); poll_round(transport); self.assertEqual(transport.paths, list(POLL_PATHS))
        progress, roots, status = test_round("root-current", "current")
        with self.assertRaisesRegex(HarnessError, "import_scope_mismatch"):
            broken = dict(progress); broken["latest_import_scan"] = dict(progress["latest_import_scan"]); broken["latest_import_scan"]["searchable_documents"] = 9; evaluate_round("root-current", "current", broken, roots, status, False)
        stale = dict(progress); stale["latest_import_attribution"] = test_payload("stale")
        with self.assertRaisesRegex(HarnessError, "attribution_task_mismatch"):
            evaluate_round("root-current", "current", stale, roots, status, True)
    def test_normal_product_daemon_arguments(self) -> None:
        inputs = Inputs(*(Path(f"/{value}") for value in ("embed", "ocr", "tess", "render", "pdfium", "model", "scan")), 8, 1, .1)
        command = daemon_command(Variant("candidate", Path("/daemon"), Path("/embedding"), Path("/cli"), True), inputs, Path("/data"), Path("/model"))
        for flag in (
            "--work-imports", "--work-ocr", "--work-index", "--rescan-completed-imports",
            "--watch-import-roots", "--import-rescan-min-age-seconds", "--expected-ipc-protocol",
            "--ocr-tesseract-command", "--ocr-jobs-per-tick", "--embedding-command",
        ):
            self.assertIn(flag, command)
        for forbidden in ("--parent-lifecycle-stdin", "--launch-id", "--max-requests", "--ocr-command"):
            self.assertNotIn(forbidden, command)
    def test_control_fence_and_queue_not_drain(self) -> None:
        progress, roots, status = test_round("root-current", "current")
        self.assertIsNotNone(evaluate_round("root-current", "current", progress, roots, status, False))
        for depth in (0, 1):
            status["ocr_queue_depth"] = depth
            candidate = dict(progress); candidate["latest_import_attribution"] = test_payload("current")
            self.assertIsNotNone(evaluate_round("root-current", "current", candidate, roots, status, True))
    def test_conservative_overlap(self) -> None:
        observed = OcrOverlap(0)
        for depth in (1, 2, 1): observed.observe_before_complete(depth)
        pending = OcrOverlap(0); pending.observe_before_complete(1)
        missed = OcrOverlap(0); missed.observe_before_complete(1)
        self.assertEqual((observed.classify(1), pending.classify(1), missed.classify(0)), ("observed", "not_observed", "unobservable"))
    def test_unique_cohort(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory); scanned = base / "scan.pdf"; scanned.write_bytes(b"synthetic")
            build_cohort(base / "cohort", 8, scanned)
            bodies = [path.read_bytes() for path in sorted((base / "cohort").glob("direct-*.txt"))]
            self.assertEqual(len(set(bodies)), 8)
    @unittest.skipUnless(hasattr(os, "killpg"), "requires POSIX")
    def test_timeout_cleanup(self) -> None:
        command = [sys.executable, "-c", "import subprocess,sys,time;subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)']);time.sleep(60)"]
        managed = ManagedProcess.start(command, os.environ.copy()); group = managed.group
        with self.assertRaises(subprocess.TimeoutExpired): managed.process.wait(timeout=0.05)
        managed.cleanup(); self.assertFalse(group_exists(group))
    def test_report_redaction(self) -> None:
        report = report_base("self_test", 1); report["stages"] = stage_metrics(test_payload("current")); validate_report(report)
        validate_report(report_base("private_local_redacted", 0))
        for bad in ({"local_path": "/private/example"}, {"detail": "Bearer secret"}, {"raw_text": "body"}):
            candidate = dict(report); candidate["bad"] = bad
            with self.assertRaises(HarnessError): validate_report(candidate)
    def test_private_root_contract(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(HarnessError, "private_root_unavailable"):
            load_private_root()
        with tempfile.TemporaryDirectory() as directory:
            value = Path(directory) / "file"; value.write_text("synthetic")
            with mock.patch.dict(os.environ, {"RESUME_IR_PRIVATE_RESUME_ROOT": str(value)}), self.assertRaisesRegex(HarnessError, "private_root_unavailable"):
                load_private_root()
            arguments = argparse.Namespace(embedding_runtime_dir=Path(directory), ocr_bin=value, tessdata_dir=Path(directory), pdf_render_bin=value, pdfium_runtime_dir=Path(directory), classifier_model=value, scanned_fixture=value, direct_documents=8, timeout_seconds=1.0, poll_ms=100.0)
            self.assertEqual(inputs_from(arguments).scanned, value.resolve())
    def test_conservative_overhead(self) -> None:
        self.assertEqual(overhead_decision([1, 2], 0.1, 1000), "pass")
        self.assertEqual(overhead_decision([4, 5], 0.1, 1000), "rollback_required")
        self.assertEqual(overhead_decision([1, 5], 0.1, 1000), "inconclusive")
        self.assertEqual(overhead_decision([1, 2], 0.1, 100), "inconclusive")
def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    commands.add_parser("self-test")
    fixture = Path(__file__).resolve().parents[2] / "tests/fixtures/resumes/synthetic-scanned-resume.pdf"
    def common(command: argparse.ArgumentParser, direct: int) -> None:
        command.add_argument("--embedding-runtime-dir", type=Path, required=True)
        command.add_argument("--ocr-bin", type=Path, required=True)
        command.add_argument("--tessdata-dir", type=Path, required=True)
        command.add_argument("--pdf-render-bin", type=Path, required=True)
        command.add_argument("--pdfium-runtime-dir", type=Path, required=True)
        command.add_argument("--classifier-model", type=Path, required=True)
        command.add_argument("--scanned-fixture", type=Path, default=fixture)
        command.add_argument("--direct-documents", type=int, default=direct)
        command.add_argument("--timeout-seconds", type=float, default=180.0)
        command.add_argument("--poll-ms", type=float, default=100.0)
    smoke = commands.add_parser("smoke"); common(smoke, 8)
    smoke.add_argument("--private", action="store_true")
    smoke.add_argument("--daemon-bin", type=Path, required=True)
    smoke.add_argument("--embedding-bin", type=Path, required=True); smoke.add_argument("--resume-cli-bin", type=Path, required=True)
    witness = commands.add_parser("witness"); common(witness, 32)
    witness.add_argument("--private", action="store_true")
    for variant in ("control", "candidate"):
        witness.add_argument(f"--{variant}-daemon-bin", type=Path, required=True)
        witness.add_argument(f"--{variant}-embedding-bin", type=Path, required=True); witness.add_argument(f"--{variant}-resume-cli-bin", type=Path, required=True)
    return root
def main() -> int:
    arguments = parser().parse_args()
    if arguments.command == "self-test":
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(SelfTests))
        return 0 if result.wasSuccessful() else 1
    try:
        report = run_smoke(arguments) if arguments.command == "smoke" else run_witness(arguments)
        print(json.dumps(report, sort_keys=True, separators=(",", ":")))
        return 0
    except HarnessError as error:
        print(json.dumps({"status": "failed", "reason": str(error)}, separators=(",", ":")))
        return 2
    except KeyboardInterrupt:
        print('{"status":"failed","reason":"interrupted"}')
        return 130
    except Exception:
        print('{"status":"failed","reason":"internal_failure"}')
        return 2
if __name__ == "__main__":
    raise SystemExit(main())
