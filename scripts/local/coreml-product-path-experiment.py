#!/usr/bin/env python3
"""Build, preflight, run, and clean macOS ARM product-composition experiments."""
from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import platform
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable
from urllib.parse import urlsplit

REPO = Path(__file__).resolve().parents[2]
TARGET = "aarch64-apple-darwin"
MODEL_ID = "intfloat-multilingual-e5-small-coreml-fp16-r1"
MODEL_DIMENSION = 384
IPC_SCHEMA = "resume-ir.daemon-ipc.v5"
MAX_JSON_BYTES = 64 * 1024
MAX_RESPONSE_BYTES = 2 * 1024 * 1024
RUNTIME_NAMES = {
    "daemon": "resume-daemon",
    "embedding": "resume-embedding-runtime",
    "pdf": "resume-pdf-render-runtime",
    "worker": "resume-coreml-embedding-worker",
}
RUNTIMES = ("embedding", "ocr", "classifier", "pdfium")
CAPABILITIES = ("text_import", "ocr_import", "index_publication")
PRIVACY = {
    "contains_private_paths": False,
    "contains_raw_resume_text": False,
    "contains_raw_queries": False,
    "contains_candidate_results": False,
    "contains_tokens_or_vectors": False,
}


class ExperimentError(RuntimeError):
    """A bounded public-safe experiment failure code."""


@dataclass(frozen=True)
class Composition:
    binaries: dict[str, Path]
    cli: Path
    embedding: Path
    coreml: Path
    ocr: Path
    tessdata: Path
    classifier: Path
    pdfium: Path
    source_commit: str
    source_dirty: bool


def read_json(path: Path) -> dict[str, object]:
    try:
        body = path.read_bytes()
        value = json.loads(body)
    except (OSError, json.JSONDecodeError):
        raise ExperimentError("invalid_json_contract") from None
    if len(body) > MAX_JSON_BYTES or not isinstance(value, dict):
        raise ExperimentError("invalid_json_contract")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_regular(path: Path, executable: bool = False) -> None:
    try:
        metadata = path.lstat()
    except OSError:
        raise ExperimentError("product_composition_missing") from None
    if not stat.S_ISREG(metadata.st_mode) or path.is_symlink():
        raise ExperimentError("product_composition_invalid")
    if executable and not os.access(path, os.X_OK):
        raise ExperimentError("product_executable_invalid")


def verify_pack(root: Path, schema: str, required: dict[str, object]) -> dict[str, object]:
    manifest = read_json(root / "runtime-pack.json")
    if manifest.get("schema_version") != schema or any(
        manifest.get(key) != value for key, value in required.items()
    ):
        raise ExperimentError("runtime_pack_identity_mismatch")
    files = manifest.get("files")
    if not isinstance(files, list) or not 0 < len(files) <= 128:
        raise ExperimentError("runtime_pack_files_invalid")
    for entry in files:
        if not isinstance(entry, dict):
            raise ExperimentError("runtime_pack_files_invalid")
        relative = entry.get("file")
        size = entry.get("bytes")
        expected = entry.get("sha256")
        if (
            not isinstance(relative, str)
            or not relative
            or Path(relative).is_absolute()
            or ".." in Path(relative).parts
            or not isinstance(size, int)
            or size <= 0
            or not isinstance(expected, str)
            or len(expected) != 64
        ):
            raise ExperimentError("runtime_pack_files_invalid")
        target = root / relative
        require_regular(target, entry.get("executable") is True)
        if target.stat().st_size != size or sha256(target) != expected:
            raise ExperimentError("runtime_pack_digest_mismatch")
    return manifest


def run_build(command: list[str], log: object) -> None:
    try:
        result = subprocess.run(
            command,
            cwd=REPO,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=1800,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        raise ExperimentError("product_composition_build_failed") from None
    if result.returncode != 0:
        raise ExperimentError("product_composition_build_failed")


def stage_executables(destination: Path, source_root: Path) -> dict[str, Path]:
    destination.mkdir(parents=True, mode=0o700)
    staged: dict[str, Path] = {}
    for role, final_name in RUNTIME_NAMES.items():
        source = source_root / f"{final_name}-{TARGET}"
        require_regular(source, executable=True)
        target = destination / final_name
        shutil.copy2(source, target)
        target.chmod(0o700)
        require_regular(target, executable=True)
        staged[role] = target
    return staged


def build_composition(session: Path) -> Composition:
    if sys.platform != "darwin" or platform.machine() != "arm64":
        raise ExperimentError("macos_arm_required")
    node, cargo = shutil.which("node"), shutil.which("cargo")
    if not node or not cargo:
        raise ExperimentError("build_tool_unavailable")
    cache = REPO / ".cache" / "resume-ir-product-experiments"
    cache.mkdir(parents=True, exist_ok=True)
    log_path = cache / "last-build.log"
    with log_path.open("wb") as log:
        run_build(
            [node, "apps/desktop/scripts/prepare-sidecar.mjs", "--release", "--target", TARGET],
            log,
        )
        run_build([cargo, "build", "-p", "resume-cli", "--bin", "resume-cli", "--release", "--locked"], log)

    sidecars = REPO / "target" / "tauri-sidecars"
    resources = REPO / "target" / "tauri-resources"
    binaries = stage_executables(session / "composition" / "bin", sidecars)
    cli = REPO / "target" / "release" / "resume-cli"
    require_regular(cli, executable=True)
    embedding = resources / "embedding-runtime-pack"
    coreml = embedding / "coreml"
    ocr = resources / "ocr-runtime-pack"
    classifier = resources / "classifier-model-pack"
    pdfium = resources / "pdfium-static-runtime-pack"
    verify_pack(
        embedding,
        "resume-ir.embedding-runtime-pack.v1",
        {"model_id": "intfloat-multilingual-e5-small-qint8-r1", "dimension": MODEL_DIMENSION},
    )
    coreml_manifest = verify_pack(
        coreml,
        "resume-ir.coreml-embedding-runtime-pack.v1",
        {"model_id": MODEL_ID, "dimension": MODEL_DIMENSION, "target_triple": TARGET},
    )
    if coreml_manifest.get("fixed_shapes") != ["B1x512", "B4x512"]:
        raise ExperimentError("runtime_pack_identity_mismatch")
    verify_pack(ocr, "resume-ir.desktop-ocr-runtime-pack.v1", {"target_triple": TARGET})
    verify_pack(classifier, "resume-ir.desktop-classifier-model-pack.v1", {})
    verify_pack(pdfium, "resume-ir.pdfium-static-runtime-pack.v1", {"target_triple": TARGET})
    source_commit = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=REPO, text=True, timeout=5
    ).strip()
    if len(source_commit) != 40:
        raise ExperimentError("source_identity_invalid")
    source_dirty = any(subprocess.run(["git", *args], cwd=REPO).returncode != 0
                       for args in (("diff", "--quiet"), ("diff", "--cached", "--quiet")))
    log_path.unlink(missing_ok=True)
    return Composition(
        binaries=binaries,
        cli=cli,
        embedding=embedding,
        coreml=coreml,
        ocr=ocr / "tesseract",
        tessdata=ocr / "tessdata",
        classifier=classifier / "linear-promotion-model.json",
        pdfium=pdfium,
        source_commit=source_commit,
        source_dirty=source_dirty,
    )


class Transport:
    def __init__(self, endpoint: str, token: str) -> None:
        parsed = urlsplit(endpoint)
        if parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "::1"} or not parsed.port:
            raise ExperimentError("ipc_owner_invalid")
        self.host, self.port, self.token = parsed.hostname, parsed.port, token

    def request(self, method: str, path: str, payload: dict[str, object] | None = None) -> tuple[int, dict[str, object]]:
        encoded = None if payload is None else json.dumps(payload).encode()
        headers = {"Authorization": f"Bearer {self.token}"}
        if encoded is not None:
            headers["Content-Type"] = "application/json"
        connection = http.client.HTTPConnection(self.host, self.port, timeout=5)
        try:
            connection.request(method, path, encoded, headers)
            response = connection.getresponse()
            body = response.read(MAX_RESPONSE_BYTES + 1)
        except (OSError, http.client.HTTPException):
            raise ExperimentError("ipc_request_failed") from None
        finally:
            connection.close()
        if len(body) > MAX_RESPONSE_BYTES:
            raise ExperimentError("ipc_response_too_large")
        try:
            lines = [line for line in body.splitlines() if line.strip()]
            value = json.loads(lines[-1] if path == "/imports/progress" else body)
        except (UnicodeDecodeError, json.JSONDecodeError):
            raise ExperimentError("ipc_response_invalid") from None
        if not isinstance(value, dict):
            raise ExperimentError("ipc_response_invalid")
        return response.status, value

    def get(self, path: str) -> dict[str, object]:
        status, value = self.request("GET", path)
        if status != 200:
            raise ExperimentError("ipc_get_rejected")
        return value


class Daemon:
    def __init__(self, composition: Composition, run_root: Path) -> None:
        self.composition = composition
        self.run_root = run_root
        self.data = run_root / "data"
        self.data.mkdir(parents=True, mode=0o700)
        self.stdout = (run_root / "daemon.stdout").open("wb")
        self.stderr = (run_root / "daemon.stderr").open("wb")
        environment = os.environ.copy()
        environment.update(
            {
                "RESUME_IR_COREML_WORKER_BIN": str(composition.binaries["worker"]),
                "RESUME_IR_COREML_RUNTIME_DIR": str(composition.coreml),
                "RESUME_IR_EMBEDDING_RUNTIME_DIR": str(composition.embedding),
                "RESUME_IR_PDFIUM_RUNTIME_DIR": str(composition.pdfium),
                "TESSDATA_PREFIX": str(composition.tessdata),
            }
        )
        command = [
            str(composition.binaries["daemon"]), "--data-dir", str(self.data), "run",
            "--foreground", "--work-imports", "--work-ocr", "--work-index",
            "--ipc-listen", "127.0.0.1:0", "--max-requests", "10000",
            "--resume-classifier-model", str(composition.classifier),
            "--ocr-tesseract-command", str(composition.ocr),
            "--pdf-render-command", str(composition.binaries["pdf"]),
            "--ocr-lang", "eng+chi_sim", "--embedding-command",
            str(composition.binaries["embedding"]), "--embedding-model-id", MODEL_ID,
            "--embedding-dimension", str(MODEL_DIMENSION), "--embedding-timeout-ms", "30000",
            "--worker-interval-ms", "100",
        ]
        try:
            self.process = subprocess.Popen(
                command,
                cwd=REPO,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=self.stdout,
                stderr=self.stderr,
                process_group=0,
            )
        except OSError:
            self.stdout.close()
            self.stderr.close()
            raise ExperimentError("daemon_start_failed") from None

    def ready(self, timeout: float) -> tuple[Transport, dict[str, object]]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise ExperimentError("daemon_exited_before_ready")
            try:
                endpoints, auth = read_json(self.data / "ipc.endpoints.json"), read_json(self.data / "ipc.auth")
            except ExperimentError:
                time.sleep(0.1)
                continue
            if (
                endpoints.get("schema_version") != IPC_SCHEMA
                or endpoints.get("instance_id") != auth.get("instance_id")
                or endpoints.get("launch_id") != auth.get("launch_id")
                or not isinstance(auth.get("token"), str)
            ):
                raise ExperimentError("ipc_owner_invalid")
            transport = Transport(str(endpoints.get("status", "")), str(auth["token"]))
            try:
                status = transport.get("/status")
            except ExperimentError as error:
                if str(error) != "ipc_request_failed":
                    raise
                time.sleep(0.1)
                continue
            runtimes, capabilities = status.get("optional_runtimes"), status.get("capabilities")
            if isinstance(runtimes, dict):
                for key in RUNTIMES:
                    runtime = runtimes.get(key)
                    if isinstance(runtime, dict) and runtime.get("state") == "unavailable":
                        reason = runtime.get("reason")
                        if reason not in {"invalid", "missing", "start_failed", "unsupported"}:
                            reason = "unavailable"
                        raise ExperimentError(f"{key}_runtime_{reason}")
            if (
                isinstance(runtimes, dict)
                and isinstance(capabilities, dict)
                and isinstance(status.get("core"), dict)
                and status["core"].get("state") == "ready"  # type: ignore[index]
                and all(isinstance(runtimes.get(key), dict) and runtimes[key].get("state") == "available" for key in RUNTIMES)
                and all(isinstance(capabilities.get(key), dict) and capabilities[key].get("state") == "available" for key in CAPABILITIES)
            ):
                return transport, status
            time.sleep(0.1)
        raise ExperimentError("capability_attestation_timeout")

    def stop(self) -> bool:
        graceful = False
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=60)
                graceful = True
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=10)
        else:
            graceful = self.process.returncode == 0
        self.stdout.close()
        self.stderr.close()
        deadline = time.monotonic() + 5
        while process_group_exists(self.process.pid) and time.monotonic() < deadline:
            time.sleep(0.05)
        return graceful and not process_group_exists(self.process.pid)


def process_group_exists(group: int) -> bool:
    try:
        os.killpg(group, 0)
    except (ProcessLookupError, PermissionError):
        return False
    return True


def process_tree_rss(root_pid: int) -> int:
    try:
        output = subprocess.check_output(["ps", "-axo", "pid=,ppid=,rss="], text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return 0
    rows: list[tuple[int, int, int]] = []
    for line in output.splitlines():
        try:
            pid, parent, rss = (int(value) for value in line.split())
        except (ValueError, TypeError):
            continue
        rows.append((pid, parent, rss))
    descendants = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, parent, _ in rows:
            if parent in descendants and pid not in descendants:
                descendants.add(pid)
                changed = True
    return sum(rss for pid, _, rss in rows if pid in descendants)


def register_and_scan(
    transport: object,
    private_root: Path,
    clock: Callable[[], float] = time.monotonic,
) -> tuple[str, str, float]:
    status, body = transport.request("POST", "/source-roots/register", {  # type: ignore[attr-defined]
        "schema_version": "resume-ir.source-root-register-request.v1",
        "requested_path": str(private_root),
        "display_label": "Product experiment corpus",
    })
    managed = body.get("root") if isinstance(body, dict) else None
    root_id = managed.get("root_id") if isinstance(managed, dict) else None
    if status != 200 or not isinstance(root_id, str) or not root_id:
        raise ExperimentError("source_root_registration_failed")
    scan_started = clock()
    try:
        status, body = transport.request("POST", "/source-roots/scan", {  # type: ignore[attr-defined]
            "schema_version": "resume-ir.source-root-scan-request.v1", "root_id": root_id,
        })
    except ExperimentError:
        raise ExperimentError("scan_submission_unknown") from None
    managed = body.get("root") if isinstance(body, dict) else None
    scan = managed.get("last_scan") if isinstance(managed, dict) else None
    scan_id = scan.get("scan_id") if isinstance(scan, dict) else None
    if status != 200 or managed.get("root_id") != root_id or not isinstance(scan_id, str) or not scan_id:  # type: ignore[union-attr]
        raise ExperimentError("scan_submission_rejected")
    return root_id, scan_id, scan_started


def terminal_counts(root_id: str, scan_id: str, progress: dict[str, object], roots: dict[str, object], status: dict[str, object]) -> dict[str, int] | None:
    candidates = roots.get("roots")
    managed = next((item for item in candidates if isinstance(item, dict) and item.get("root_id") == root_id), None) if isinstance(candidates, list) else None
    scan = managed.get("last_scan") if isinstance(managed, dict) else None
    if not isinstance(scan, dict) or scan.get("scan_id") != scan_id:
        return None
    if scan.get("phase") != "complete" or scan.get("completeness") != "complete":
        return None
    counts, latest = scan.get("counts"), progress.get("latest_import_scan")
    if not isinstance(counts, dict) or not isinstance(latest, dict):
        raise ExperimentError("import_status_invalid")
    mapping = {
        "discovered": "files_discovered", "searchable": "searchable_documents",
        "ocr": "ocr_required_documents", "failed": "failed_documents", "ignored": "ignored_entries",
    }
    result: dict[str, int] = {}
    for short, long_name in mapping.items():
        left, right = counts.get(short), latest.get(long_name)
        if not isinstance(left, int) or isinstance(left, bool) or left < 0 or left != right:
            raise ExperimentError("import_count_mismatch")
        result[long_name] = left
    if counts.get("processed") != counts.get("discovered"):
        raise ExperimentError("import_count_mismatch")
    if not (
        status.get("import_tasks_queued") == 0
        and status.get("import_tasks_recoverable") == 0
        and status.get("recovery_queue_depth") == 0
        and status.get("embedding_queue_depth") == 0
        and status.get("index_health") == "ready"
        and isinstance(status.get("indexed_documents"), int)
        and status["indexed_documents"] >= result["searchable_documents"]
    ):
        return None
    return result


def run_import(composition: Composition, run_root: Path, private_root: Path, ready_timeout: float, run_timeout: float) -> dict[str, object]:
    daemon = Daemon(composition, run_root)
    graceful = False
    peak_kib = 0
    try:
        transport, _ = daemon.ready(ready_timeout)
        root_id, scan_id, started = register_and_scan(transport, private_root)
        stable, counts = 0, None
        deadline, next_progress = time.monotonic() + run_timeout, time.monotonic()
        while time.monotonic() < deadline:
            if daemon.process.poll() is not None:
                raise ExperimentError("daemon_exited_during_import")
            peak_kib = max(peak_kib, process_tree_rss(daemon.process.pid))
            progress, roots, status = transport.get("/imports/progress"), transport.get("/source-roots"), transport.get("/status")
            candidate = terminal_counts(root_id, scan_id, progress, roots, status)
            stable = stable + 1 if candidate is not None else 0
            counts = candidate or counts
            if time.monotonic() >= next_progress:
                latest = progress.get("latest_import_scan") or {}
                print(json.dumps({
                    "event": "progress", "elapsed_seconds": round(time.monotonic() - started, 1),
                    "files_discovered": latest.get("files_discovered", 0),
                    "searchable_documents": status.get("searchable_documents", 0),
                    "embedding_queue_depth": status.get("embedding_queue_depth", 0),
                    "ocr_queue_depth": status.get("ocr_queue_depth", 0),
                }, separators=(",", ":")), file=sys.stderr, flush=True)
                next_progress = time.monotonic() + 30
            if stable >= 2 and counts is not None:
                elapsed = time.monotonic() - started
                return {
                    "elapsed_seconds": round(elapsed, 3), "counts": counts,
                    "peak_process_tree_mib": round(peak_kib / 1024, 3),
                    "ocr_queue_depth_at_endpoint": status.get("ocr_queue_depth"),
                }
            time.sleep(1)
        raise ExperimentError("import_endpoint_timeout")
    finally:
        graceful = daemon.stop()
        if not graceful and sys.exc_info()[0] is None:
            raise ExperimentError("process_cleanup_failed")


def preflight(composition: Composition, run_root: Path, timeout: float) -> dict[str, object]:
    daemon = Daemon(composition, run_root)
    cleanup = False
    try:
        _, status = daemon.ready(timeout)
        return {
            "schema_version": "resume-ir.product-experiment-preflight.v1",
            "status": "pass", "source_commit": composition.source_commit,
            "source_dirty": composition.source_dirty,
            "target_triple": TARGET, "model_id": MODEL_ID,
            "runtime_states": {key: status["optional_runtimes"][key]["state"] for key in RUNTIMES},
            "capability_states": {key: status["capabilities"][key]["state"] for key in CAPABILITIES},
            "privacy": PRIVACY,
        }
    finally:
        cleanup = daemon.stop()
        if not cleanup and sys.exc_info()[0] is None:
            raise ExperimentError("process_cleanup_failed")


def private_root_from_environment() -> Path:
    value = os.environ.get("RESUME_IR_PRIVATE_RESUME_ROOT")
    if not value:
        env_file = REPO / ".env"
        try:
            lines = env_file.read_text(encoding="utf-8").splitlines()
        except OSError:
            lines = []
        for line in lines:
            candidate = line.strip()
            if candidate.startswith("export "):
                candidate = candidate[7:].lstrip()
            if not candidate.startswith("RESUME_IR_PRIVATE_RESUME_ROOT="):
                continue
            raw = candidate.split("=", 1)[1]
            try:
                parts = shlex.split(raw, comments=True, posix=True)
            except ValueError:
                raise ExperimentError("private_root_invalid") from None
            if len(parts) == 1:
                value = parts[0]
            break
    if not value:
        raise ExperimentError("private_root_unavailable")
    root = Path(value)
    if not root.is_absolute() or not root.is_dir() or not os.access(root, os.R_OK):
        raise ExperimentError("private_root_unavailable")
    return root.resolve()


def bounded_runs(value: str) -> int:
    try:
        count = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError("runs must be an integer") from None
    if not 1 <= count <= 10:
        raise argparse.ArgumentTypeError("runs must be between 1 and 10")
    return count


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    quick = commands.add_parser("preflight")
    quick.add_argument("--readiness-timeout-seconds", type=float, default=90.0)
    run = commands.add_parser("run")
    run.add_argument("--runs", type=bounded_runs, default=2)
    run.add_argument("--keep-last-for-query-review", action="store_true")
    run.add_argument("--readiness-timeout-seconds", type=float, default=90.0)
    run.add_argument("--run-timeout-seconds", type=float, default=1800.0)
    return root


def execute(arguments: argparse.Namespace) -> dict[str, object]:
    cache = REPO / ".cache" / "resume-ir-product-experiments"
    cache.mkdir(parents=True, exist_ok=True)
    session = Path(tempfile.mkdtemp(prefix="session-", dir=cache))
    keep_session = False
    try:
        print('{"event":"building_product_composition"}', file=sys.stderr, flush=True)
        composition = build_composition(session)
        preflight_root = session / "preflight"
        preflight_report = preflight(composition, preflight_root, arguments.readiness_timeout_seconds)
        shutil.rmtree(preflight_root, ignore_errors=True)
        if arguments.command == "preflight":
            return preflight_report
        private_root = private_root_from_environment()
        runs = []
        for index in range(arguments.runs):
            run_root = session / f"run-{index + 1:03d}"
            result = run_import(
                composition, run_root, private_root,
                arguments.readiness_timeout_seconds, arguments.run_timeout_seconds,
            )
            result["run_number"] = index + 1
            runs.append(result)
            if index + 1 != arguments.runs or not arguments.keep_last_for_query_review:
                shutil.rmtree(run_root, ignore_errors=True)
        keep_session = arguments.keep_last_for_query_review
        if keep_session:
            pointer = cache / "last-retained.json"
            pointer.write_text(json.dumps({
                "schema_version": "resume-ir.product-experiment-retained.v1",
                "session": str(session), "data": str(session / f"run-{arguments.runs:03d}" / "data"),
                "source_commit": composition.source_commit, "model_id": MODEL_ID,
            }, separators=(",", ":")) + "\n", encoding="utf-8")
            pointer.chmod(0o600)
        return {
            "schema_version": "resume-ir.product-experiment-run.v1",
            "status": "pass", "source_commit": composition.source_commit,
            "source_dirty": composition.source_dirty,
            "target_triple": TARGET, "model_id": MODEL_ID,
            "run_count": len(runs), "runs": runs,
            "retained_for_query_review": keep_session, "privacy": PRIVACY,
        }
    finally:
        if not keep_session:
            shutil.rmtree(session, ignore_errors=True)


def main() -> int:
    arguments = parser().parse_args()
    if arguments.readiness_timeout_seconds <= 0 or (
        arguments.command == "run" and arguments.run_timeout_seconds <= 0
    ):
        print('{"status":"failed","reason":"invalid_timeout"}')
        return 2
    try:
        report = execute(arguments)
        print(json.dumps(report, sort_keys=True, separators=(",", ":")))
        return 0
    except ExperimentError as error:
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
