"""Manifest, fingerprint, and local-ledger primitives for parallel verification."""

from __future__ import annotations

import dataclasses
import datetime as dt
import glob
import hashlib
import json
import os
import pathlib
import shutil
import stat
import subprocess
import tempfile
from collections.abc import Iterable, Sequence
from typing import Any


LEGACY_LEDGER_SCHEMA_VERSION = "resume-ir.local-verification-ledger.v1"
LEDGER_SCHEMA_VERSION = "resume-ir.local-verification-ledger.v2"
MANIFEST_SCHEMA_VERSION = "resume-ir.verify-local-parallel-manifest.v2"
DEFAULT_MANIFEST = pathlib.Path("scripts/ci/verify-local-parallel-manifest.json")
DEFAULT_STATE_DIR = pathlib.Path(".cache/verify-local-parallel")
MAX_FAILURE_TAIL_BYTES = 4_096
SOURCE_AUTHORITIES = frozenset({"repository", "worktree_snapshot", "exact_main_commit"})
EVIDENCE_LANES = frozenset({"smoke", "w0_docs", "w1_private", "soak_fault", "gui_manual"})
CLAIMS = frozenset({"verification", "manual_test", "release_gate", "installed_acceptance"})
EXPECTED_ORIGIN = "https://github.com/FrankQDWang/resume-ir.git"
GIT_HEAD = frozenset("0123456789abcdef")


class RunnerError(RuntimeError):
    """Raised when a local verification configuration is invalid."""


@dataclasses.dataclass(frozen=True)
class Check:
    """One independently resumable local verification command."""

    identifier: str
    behavior: str
    command: tuple[str, ...]
    inputs: tuple[str, ...]
    input_sets: tuple[str, ...]
    resources: tuple[str, ...]
    source_authority: str
    evidence_lane: str
    claim: str
    produces_artifact: bool


@dataclasses.dataclass(frozen=True)
class CheckResult:
    """A completed check result kept in the local ledger."""

    check: Check
    fingerprint: str
    outcome: str
    exit_code: int | None
    duration_seconds: float
    log_path: str | None
    context_reason: str | None = None


def fail(message: str) -> None:
    raise RunnerError(message)


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def resolve_under(root: pathlib.Path, value: str) -> pathlib.Path:
    candidate = pathlib.Path(value)
    return candidate if candidate.is_absolute() else root / candidate


def require_relative_pattern(pattern: str) -> None:
    if pattern == "@worktree":
        return
    candidate = pathlib.PurePosixPath(pattern)
    if candidate.is_absolute() or ".." in candidate.parts:
        fail(f"manifest input pattern must stay below the repository root: {pattern}")


def read_manifest(path: pathlib.Path) -> tuple[dict[str, int], dict[str, tuple[str, ...]], tuple[Check, ...]]:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        fail(f"parallel verification manifest is missing: {path}")
    except json.JSONDecodeError as error:
        fail(f"parallel verification manifest is invalid JSON: {error}")
    if not isinstance(raw, dict) or raw.get("schema_version") != MANIFEST_SCHEMA_VERSION:
        fail("parallel verification manifest has an unsupported schema version")

    raw_resources = raw.get("resources")
    if not isinstance(raw_resources, dict) or not raw_resources:
        fail("parallel verification manifest must declare resource capacities")
    resources: dict[str, int] = {}
    for name, limit in raw_resources.items():
        if not isinstance(name, str) or not name or not isinstance(limit, int) or limit < 1:
            fail("parallel verification manifest contains an invalid resource capacity")
        resources[name] = limit

    raw_sets = raw.get("input_sets", {})
    if not isinstance(raw_sets, dict):
        fail("parallel verification manifest input_sets must be an object")
    input_sets: dict[str, tuple[str, ...]] = {}
    for name, patterns in raw_sets.items():
        if not isinstance(name, str) or not isinstance(patterns, list) or not patterns:
            fail("parallel verification manifest contains an invalid input set")
        if any(not isinstance(pattern, str) or not pattern for pattern in patterns):
            fail("parallel verification manifest input sets contain an invalid pattern")
        for pattern in patterns:
            require_relative_pattern(pattern)
        input_sets[name] = tuple(patterns)

    raw_checks = raw.get("checks")
    if not isinstance(raw_checks, list) or not raw_checks:
        fail("parallel verification manifest must declare checks")
    checks: list[Check] = []
    identifiers: set[str] = set()
    for entry in raw_checks:
        if not isinstance(entry, dict):
            fail("parallel verification manifest check entries must be objects")
        identifier = entry.get("id")
        behavior = entry.get("behavior")
        command = entry.get("command")
        inputs = entry.get("inputs", [])
        input_set_names = entry.get("input_sets", [])
        check_resources = entry.get("resources", [])
        source_authority = entry.get("source_authority", "repository")
        evidence_lane = entry.get("evidence_lane", "w0_docs")
        claim = entry.get("claim", "verification")
        produces_artifact = entry.get("produces_artifact", False)
        valid = (
            isinstance(identifier, str)
            and bool(identifier)
            and identifier not in identifiers
            and isinstance(behavior, str)
            and bool(behavior)
            and isinstance(command, list)
            and bool(command)
            and all(isinstance(part, str) and part for part in command)
            and isinstance(inputs, list)
            and all(isinstance(pattern, str) and pattern for pattern in inputs)
            and isinstance(input_set_names, list)
            and all(isinstance(name, str) and name in input_sets for name in input_set_names)
            and isinstance(check_resources, list)
            and all(isinstance(name, str) and name in resources for name in check_resources)
            and len(set(check_resources)) == len(check_resources)
            and source_authority in SOURCE_AUTHORITIES
            and evidence_lane in EVIDENCE_LANES
            and claim in CLAIMS
            and isinstance(produces_artifact, bool)
        )
        if not valid:
            fail(f"parallel verification manifest check is invalid: {identifier!r}")
        if not inputs and not input_set_names:
            fail(f"parallel verification check must declare inputs: {identifier}")
        for pattern in inputs:
            require_relative_pattern(pattern)
        if produces_artifact:
            if "packaging" not in check_resources:
                fail(
                    "parallel verification artifact producers require the packaging resource: "
                    f"{identifier}"
                )
            if source_authority == "repository" or claim == "verification":
                fail(
                    "parallel verification artifact producers must declare source authority "
                    f"and evidence claim: {identifier}"
                )
        elif source_authority != "repository":
            fail(
                "parallel verification source authority is only valid for artifact producers: "
                f"{identifier}"
            )
        identifiers.add(identifier)
        checks.append(
            Check(
                identifier=identifier,
                behavior=behavior,
                command=tuple(command),
                inputs=tuple(inputs),
                input_sets=tuple(input_set_names),
                resources=tuple(check_resources),
                source_authority=source_authority,
                evidence_lane=evidence_lane,
                claim=claim,
                produces_artifact=produces_artifact,
            )
        )
    return resources, input_sets, tuple(checks)


def resolve_cargo() -> str:
    configured = os.environ.get("CARGO")
    candidates = [configured] if configured else []
    candidates.extend(("/Users/frankqdwang/.cargo/bin/cargo", shutil.which("cargo")))
    for candidate in candidates:
        if candidate and os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    fail("parallel verification requires an executable cargo; set CARGO to its path")


def command_for(check: Check, cargo: str) -> tuple[str, ...]:
    command: list[str] = []
    for part in check.command:
        if part == "{cargo}":
            command.append(cargo)
        elif "{cargo}" in part:
            fail(f"cargo placeholder must occupy one complete command argument: {check.identifier}")
        else:
            command.append(part)
    return tuple(command)


def source_context_block(root: pathlib.Path, check: Check) -> str | None:
    """Return a closed reason when a packaging check cannot run in this source context."""

    if check.source_authority == "repository":
        return None

    def git(*arguments: str) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            ["git", *arguments],
            cwd=root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    head = git("rev-parse", "--verify", "HEAD")
    head_text = head.stdout.decode("ascii", errors="ignore").strip()
    if (
        head.returncode != 0
        or len(head_text) != 40
        or any(character not in GIT_HEAD for character in head_text)
    ):
        return "source_head_unavailable"

    unmerged = git("diff", "--name-only", "--diff-filter=U", "-z")
    files = git("ls-files", "--cached", "--others", "--exclude-standard", "-z")
    if unmerged.returncode != 0 or unmerged.stdout:
        return "source_index_unmerged"
    if files.returncode != 0 or not files.stdout:
        return "source_file_set_unavailable"
    if check.source_authority == "worktree_snapshot":
        return None

    status = git("status", "--porcelain=v1", "--untracked-files=all")
    branch = git("branch", "--show-current")
    upstream = git("rev-parse", "--verify", "refs/remotes/origin/main")
    origin = git("remote", "get-url", "--all", "origin")
    if (
        status.returncode != 0
        or status.stdout
        or branch.returncode != 0
        or branch.stdout.decode("utf-8", errors="ignore").strip() not in {"", "main"}
        or upstream.returncode != 0
        or upstream.stdout.decode("ascii", errors="ignore").strip() != head_text
        or origin.returncode != 0
        or origin.stdout.decode("utf-8", errors="ignore") != f"{EXPECTED_ORIGIN}\n"
    ):
        return "exact_main_source_required"
    return None


class InputHasher:
    """Caches immutable-at-start input fingerprints across one runner invocation."""

    def __init__(self, root: pathlib.Path) -> None:
        self.root = root
        self.pattern_entries: dict[str, list[tuple[str, bytes]]] = {}
        self.file_digests: dict[str, bytes] = {}

    def expand(self, patterns: Iterable[str]) -> list[tuple[str, bytes]]:
        entries: list[tuple[str, bytes]] = []
        for pattern in patterns:
            cached = self.pattern_entries.get(pattern)
            if cached is None:
                cached = self.expand_pattern(pattern)
                self.pattern_entries[pattern] = cached
            entries.extend(cached)
        return entries

    def expand_pattern(self, pattern: str) -> list[tuple[str, bytes]]:
        entries: list[tuple[str, bytes]] = []
        if pattern == "@worktree":
            matches = self.worktree_files()
        else:
            matches = [pathlib.Path(match) for match in sorted(glob.glob(str(self.root / pattern), recursive=True))]
        if not matches:
            return [(f"missing:{pattern}", b"")]
        for path in matches:
            try:
                relative = path.relative_to(self.root).as_posix()
            except ValueError:
                continue
            if {".git", ".cache", "target", "node_modules", "dist"} & set(relative.split("/")):
                continue
            try:
                mode = path.lstat().st_mode
            except FileNotFoundError:
                continue
            if stat.S_ISLNK(mode):
                entries.append((relative, f"symlink:{os.readlink(path)}".encode("utf-8")))
            elif stat.S_ISREG(mode):
                entries.append((relative, self.file_digest(path, relative)))
        return entries

    def worktree_files(self) -> list[pathlib.Path]:
        completed = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
            cwd=self.root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode != 0:
            fail("could not enumerate the Git worktree for the public-repo guard fingerprint")
        return [self.root / item.decode("utf-8") for item in completed.stdout.split(b"\0") if item]

    def file_digest(self, path: pathlib.Path, relative: str) -> bytes:
        cached = self.file_digests.get(relative)
        if cached is not None:
            return cached
        digest = hashlib.sha256()
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        value = digest.digest()
        self.file_digests[relative] = value
        return value


def fingerprint_for(
    root: pathlib.Path,
    check: Check,
    input_sets: dict[str, tuple[str, ...]],
    command: Sequence[str],
    input_hasher: InputHasher | None = None,
) -> str:
    patterns = [*check.inputs, *(pattern for name in check.input_sets for pattern in input_sets[name])]
    patterns.extend(command_source_inputs(command))
    digest = hashlib.sha256()
    has_explicit_execution_context = (
        check.source_authority != "repository"
        or check.evidence_lane != "w0_docs"
        or check.claim != "verification"
        or check.produces_artifact
    )
    digest.update(
        (
            LEDGER_SCHEMA_VERSION
            if has_explicit_execution_context
            else LEGACY_LEDGER_SCHEMA_VERSION
        ).encode("utf-8")
    )
    digest.update(check.identifier.encode("utf-8"))
    digest.update(json.dumps(command, separators=(",", ":")).encode("utf-8"))
    if has_explicit_execution_context:
        digest.update(check.source_authority.encode("utf-8"))
        digest.update(check.evidence_lane.encode("utf-8"))
        digest.update(check.claim.encode("utf-8"))
        digest.update(str(check.produces_artifact).encode("ascii"))
    for name, content_digest in sorted((input_hasher or InputHasher(root)).expand(patterns)):
        digest.update(name.encode("utf-8"))
        digest.update(b"\0")
        digest.update(content_digest)
        digest.update(b"\0")
    return digest.hexdigest()


def command_source_inputs(command: Sequence[str]) -> tuple[str, ...]:
    return tuple(
        normalized
        for part in command
        if (normalized := part.removeprefix("./")).startswith(
            ("scripts/", "apps/", "crates/", "tests/", "perf/", ".github/")
        )
    )


def load_state(path: pathlib.Path) -> dict[str, Any]:
    if not path.exists():
        return {"schema_version": LEDGER_SCHEMA_VERSION, "checks": {}}
    try:
        state = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        fail(f"local verification ledger is invalid JSON: {error}")
    if not isinstance(state, dict) or not isinstance(state.get("checks"), dict):
        fail("local verification ledger has an unsupported schema version")
    if state.get("schema_version") == LEGACY_LEDGER_SCHEMA_VERSION:
        return {
            "schema_version": LEDGER_SCHEMA_VERSION,
            "carried_from_schema": LEGACY_LEDGER_SCHEMA_VERSION,
            "checks": state["checks"],
        }
    if state.get("schema_version") != LEDGER_SCHEMA_VERSION:
        fail("local verification ledger has an unsupported schema version")
    return state


def write_json_atomically(path: pathlib.Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, prefix=f".{path.name}.", delete=False
    ) as handle:
        json.dump(payload, handle, indent=2, sort_keys=True)
        handle.write("\n")
        temporary = pathlib.Path(handle.name)
    temporary.replace(path)


def lock_owner_pid(lock_path: pathlib.Path) -> int:
    try:
        mode = lock_path.lstat().st_mode
    except FileNotFoundError:
        return 0
    if not stat.S_ISREG(mode) or stat.S_ISLNK(mode):
        fail(f"parallel verification lock is not a regular file: {lock_path}")
    try:
        lines = lock_path.read_text(encoding="utf-8").splitlines()
    except OSError:
        fail(f"parallel verification lock is unreadable: {lock_path}")
    if not lines or not lines[0].startswith("pid="):
        fail(f"parallel verification lock is malformed: {lock_path}")
    try:
        pid = int(lines[0].removeprefix("pid="))
    except ValueError:
        fail(f"parallel verification lock is malformed: {lock_path}")
    if pid < 1:
        fail(f"parallel verification lock is malformed: {lock_path}")
    return pid


def process_is_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def acquire_lock(state_dir: pathlib.Path) -> pathlib.Path:
    state_dir.mkdir(parents=True, exist_ok=True)
    lock_path = state_dir / "runner.lock"
    while True:
        try:
            descriptor = os.open(lock_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            break
        except FileExistsError:
            owner_pid = lock_owner_pid(lock_path)
            if owner_pid and not process_is_alive(owner_pid):
                try:
                    lock_path.unlink()
                except FileNotFoundError:
                    continue
                continue
            fail(f"another parallel verification runner owns this ledger: {lock_path}")
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        handle.write(f"pid={os.getpid()}\nstarted_at={utc_now()}\n")
    return lock_path


def release_lock(lock_path: pathlib.Path) -> None:
    try:
        lock_path.unlink()
    except FileNotFoundError:
        pass


def is_reusable(record: Any, fingerprint: str) -> bool:
    return (
        isinstance(record, dict)
        and record.get("outcome") == "passed"
        and record.get("fingerprint") == fingerprint
    )


def update_state(state: dict[str, Any], result: CheckResult) -> None:
    if result.outcome == "reused":
        return
    state["checks"][result.check.identifier] = {
        "behavior": result.check.behavior,
        "command": list(result.check.command),
        "finished_at": utc_now(),
        "fingerprint": result.fingerprint,
        "log_path": result.log_path,
        "outcome": result.outcome,
        "exit_code": result.exit_code,
        "duration_seconds": round(result.duration_seconds, 6),
        "source_authority": result.check.source_authority,
        "evidence_lane": result.check.evidence_lane,
        "claim": result.check.claim,
        "produces_artifact": result.check.produces_artifact,
        "context_reason": result.context_reason,
    }


def write_round(
    state_dir: pathlib.Path,
    run_id: str,
    started_at: str,
    results: Sequence[CheckResult],
    jobs: int,
    elapsed_seconds: float,
    work_seconds: float,
) -> None:
    payload = {
        "schema_version": LEDGER_SCHEMA_VERSION,
        "run_id": run_id,
        "started_at": started_at,
        "finished_at": utc_now(),
        "jobs": jobs,
        "wall_seconds": round(elapsed_seconds, 6),
        "work_seconds": round(work_seconds, 6),
        "speedup_vs_serial": round(work_seconds / elapsed_seconds, 6) if elapsed_seconds else 0.0,
        "checks": [
            {
                "id": result.check.identifier,
                "behavior": result.check.behavior,
                "command": list(result.check.command),
                "input_fingerprint": result.fingerprint,
                "resources": list(result.check.resources),
                "source_authority": result.check.source_authority,
                "evidence_lane": result.check.evidence_lane,
                "claim": result.check.claim,
                "produces_artifact": result.check.produces_artifact,
                "outcome": result.outcome,
                "exit_code": result.exit_code,
                "duration_seconds": round(result.duration_seconds, 6),
                "log_path": result.log_path,
                "context_reason": result.context_reason,
            }
            for result in results
        ],
    }
    write_json_atomically(state_dir / "runs" / f"{run_id}.json", payload)


def tail(path: pathlib.Path) -> str:
    try:
        with path.open("rb") as handle:
            handle.seek(0, os.SEEK_END)
            handle.seek(max(0, handle.tell() - MAX_FAILURE_TAIL_BYTES))
            return handle.read().decode("utf-8", errors="replace")
    except FileNotFoundError:
        return ""
