#!/usr/bin/env python3
"""Build and freeze a private mixed-document benchmark without public details."""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import pathlib
import secrets
import shutil
import stat
import sys
import tempfile
import tomllib
from collections import Counter
from dataclasses import dataclass
from typing import TextIO

ROOT = pathlib.Path(__file__).resolve().parents[2]
SUPPORTED_EXTENSIONS = {".doc", ".docx", ".pdf", ".txt"}
MAX_FILE_BYTES = 32 * 1024 * 1024
HEAD_TAIL_BYTES = 64 * 1024
KEY_FILE = ".mixed-import-freeze-hmac-key"
LAYOUT_FILE = "mixed-source-layout.local.json"
CALIBRATION_FILE = "private-calibration.local.json"
HOLDOUT_FILE = "blind-holdout.local.json"
SUMMARY_FILE = "mixed-import-freeze-summary.json"
EXCLUDED_DIR_NAMES = {
    "applications", "auth", "backup", "backups", "browser", "build",
    "cache", "caches", "chrome", "chromium", "credentials", "database",
    "databases", "diagnostics", "dist", "firefox", "index", "indexes",
    "keychain", "keychains", "keys", "library", "logs", "model-cache",
    "models", "movies", "music", "node_modules", "out", "passwords",
    "pictures", "runtime", "safari", "secrets", "site-packages", "system",
    "target", "temp", "tmp", "vendor", "venv",
}
PRIVACY = {
    "aggregate_only": True,
    "contains_raw_resume_text": False,
    "contains_raw_query_text": False,
    "contains_candidate_results": False,
    "contains_local_paths": False,
    "contains_filenames": False,
    "contains_label_details": False,
    "contains_raw_file_hashes": False,
    "contains_tokens": False,
    "contains_diagnostics_package": False,
    "contains_private_manifest": False,
}

class PermissionBlocked(Exception): """Required local roots or permissions are unavailable."""
class FreezeMismatch(Exception): """An existing freeze no longer matches current membership or content."""
class ScopeInvalid(Exception): """Configured roots violate the private-source boundary."""

@dataclass(frozen=True)
class Config:
    private_root: pathlib.Path
    mixed_root: pathlib.Path
    evidence_dir: pathlib.Path
    home_root: pathlib.Path
    home_authorized: bool

    @classmethod
    def from_environment(cls, environ: dict[str, str] | os._Environ[str] = os.environ) -> Config:
        values: dict[str, pathlib.Path] = {}
        for field, key in [
            ("private_root", "RESUME_IR_PRIVATE_RESUME_ROOT"),
            ("mixed_root", "RESUME_IR_MIXED_SOURCE_ROOT"),
            ("evidence_dir", "RESUME_IR_LOCAL_EVIDENCE_DIR"),
        ]:
            raw = environ.get(key)
            if not raw:
                raise PermissionBlocked
            configured = pathlib.Path(os.path.expandvars(raw)).expanduser()
            if configured.is_symlink():
                raise ScopeInvalid
            values[field] = configured.resolve()
        active_goal = tomllib.loads((ROOT / "ACTIVE_GOAL.toml").read_text(encoding="utf-8"))
        active_slice = active_goal.get("scope", {}).get("active_slice", {})
        config = cls(**values, home_root=pathlib.Path.home().resolve(),
                     home_authorized=active_slice.get("home_mixed_root_authorized") is True)
        config.validate()
        return config

    def validate(self) -> None:
        if not self.private_root.is_dir() or self.private_root.is_symlink():
            raise PermissionBlocked
        if not readable_directory(self.private_root):
            raise PermissionBlocked
        pairs = [(self.private_root, self.mixed_root), (self.private_root, self.evidence_dir),
                 (self.mixed_root, self.evidence_dir)]
        for left, right in pairs:
            if overlaps(left, right):
                raise ScopeInvalid
        if not self.home_authorized:
            raise ScopeInvalid
        if self.mixed_root.exists() and self.mixed_root.is_symlink():
            raise ScopeInvalid
        if self.evidence_dir.exists() and self.evidence_dir.is_symlink():
            raise ScopeInvalid

@dataclass(frozen=True)
class Candidate:
    source: pathlib.Path
    source_role: str
    label: str
    extension: str
    size_bytes: int
    mtime_ns: int
    fingerprint: str
    stable_id: str

def readable_directory(path: pathlib.Path) -> bool:
    mode = path.stat().st_mode
    return bool(mode & 0o555) and os.access(path, os.R_OK | os.X_OK)

def overlaps(left: pathlib.Path, right: pathlib.Path) -> bool:
    return is_within(left, right) or is_within(right, left)

def is_within(path: pathlib.Path, root: pathlib.Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False

def canonical_bytes(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()

def opaque_hmac(key: bytes, domain: str, payload: str | bytes) -> str:
    data = payload.encode() if isinstance(payload, str) else payload
    return hmac.new(key, domain.encode() + b"\0" + data, hashlib.sha256).hexdigest()

def ensure_private_dir(path: pathlib.Path) -> None:
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    if not path.is_dir() or path.is_symlink():
        raise ScopeInvalid
    os.chmod(path, 0o700)

def load_or_create_key(evidence_dir: pathlib.Path) -> bytes:
    ensure_private_dir(evidence_dir)
    path = evidence_dir / KEY_FILE
    if path.exists():
        if path.is_symlink() or not path.is_file():
            raise ScopeInvalid
        key = path.read_bytes()
        if len(key) != 32:
            raise ScopeInvalid
        os.chmod(path, 0o600)
        return key
    key = secrets.token_bytes(32)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(key)
    return key

def secure_write_json(path: pathlib.Path, value: object) -> None:
    data = canonical_bytes(value)
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(6)}.tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
        os.replace(temporary, path)
        os.chmod(path, 0o600)
    finally:
        temporary.unlink(missing_ok=True)

def quick_fingerprint(path: pathlib.Path, size: int, key: bytes) -> str:
    with path.open("rb") as handle:
        head = handle.read(HEAD_TAIL_BYTES)
        tail = b""
        if size > HEAD_TAIL_BYTES:
            handle.seek(max(0, size - HEAD_TAIL_BYTES))
            tail = handle.read(HEAD_TAIL_BYTES)
    payload = size.to_bytes(8, "big") + head + b"\0" + tail
    return opaque_hmac(key, "quick-fingerprint-v1", payload)

def keyed_content_digest(path: pathlib.Path, key: bytes) -> str:
    digest = hmac.new(key, b"full-content-freeze-v1\0", hashlib.sha256)
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()

def should_prune(path: pathlib.Path, excluded_roots: tuple[pathlib.Path, ...]) -> bool:
    folded_name = path.name.casefold()
    return (
        path.name.startswith(".")
        or folded_name in EXCLUDED_DIR_NAMES
        or folded_name.startswith("resume-ir-mixed-benchmark")
        or any(is_within(path.resolve(), root) for root in excluded_roots)
        or any(os.path.lexists(path / marker) for marker in [".git", ".hg", ".svn"])
    )

def discover(
    root: pathlib.Path,
    source_role: str,
    label: str,
    key: bytes,
    excluded_roots: tuple[pathlib.Path, ...],
) -> tuple[list[Candidate], Counter[str]]:
    candidates: list[Candidate] = []
    observations: Counter[str] = Counter()
    for current_text, directory_names, file_names in os.walk(root, topdown=True, followlinks=False):
        current = pathlib.Path(current_text)
        if current != root and any(is_within(current.resolve(), excluded) for excluded in excluded_roots):
            directory_names[:] = []
            continue
        kept_directories = []
        for name in directory_names:
            child = current / name
            if any(is_within(child.resolve(), root) for root in excluded_roots):
                continue
            if child.is_symlink() or should_prune(child, excluded_roots):
                observations["excluded_directories"] += 1
            else:
                kept_directories.append(name)
        directory_names[:] = kept_directories
        for name in file_names:
            path = current / name
            if name.startswith(".") or path.suffix.casefold() not in SUPPORTED_EXTENSIONS:
                observations["unsupported_or_hidden_files"] += 1
                continue
            try:
                info = path.lstat()
                if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
                    observations["symlink_or_non_regular_files"] += 1
                    continue
                if not info.st_mode & 0o444 or not os.access(path, os.R_OK):
                    observations["permission_errors"] += 1
                    continue
                if not 0 < info.st_size <= MAX_FILE_BYTES:
                    observations["size_exclusions"] += 1
                    continue
                fingerprint = quick_fingerprint(path, info.st_size, key)
                relative = path.relative_to(root).as_posix()
                identity_payload = "\0".join(
                    [source_role, relative, str(info.st_dev), str(info.st_ino), str(info.st_size), str(info.st_mtime_ns)]
                )
                candidates.append(Candidate(
                    source=path,
                    source_role=source_role,
                    label=label,
                    extension=path.suffix.casefold(),
                    size_bytes=info.st_size,
                    mtime_ns=info.st_mtime_ns,
                    fingerprint=fingerprint,
                    stable_id=opaque_hmac(key, "stable-source-v1", identity_payload),
                ))
            except OSError:
                observations["io_errors"] += 1
    candidates.sort(key=lambda item: item.stable_id)
    observations["eligible_files"] = len(candidates)
    return candidates, observations

def directory_pools() -> tuple[dict[int, list[pathlib.PurePosixPath]], dict[int, list[pathlib.PurePosixPath]]]:
    resume_only: dict[int, list[pathlib.PurePosixPath]] = {1: [], 2: [], 3: []}
    mixed: dict[int, list[pathlib.PurePosixPath]] = {1: [], 2: [], 3: []}
    for sector in range(1, 4):
        top = pathlib.PurePosixPath(f"sector-{sector:02d}")
        target = resume_only if sector == 1 else mixed
        target[1].append(top)
        for unit in range(1, 3):
            middle = top / f"unit-{unit:02d}"
            target[2].append(middle)
            for cell in range(1, 3):
                target[3].append(middle / f"cell-{cell:02d}")
    return resume_only, mixed

def assign_layout(candidates: list[Candidate], key: bytes) -> list[tuple[Candidate, pathlib.PurePosixPath]]:
    resume_only, mixed = directory_pools()
    counters: Counter[tuple[str, int]] = Counter()
    assignments: list[tuple[Candidate, pathlib.PurePosixPath]] = []
    by_role = {role: [item for item in candidates if item.source_role == role]
               for role in ["known_resume_source", "ordinary_document_source"]}
    for role, items in by_role.items():
        for index, candidate in enumerate(items):
            depth = index % 4
            parent = pathlib.PurePosixPath()
            if depth:
                if role == "ordinary_document_source":
                    pool = mixed[depth]
                else:
                    pool = resume_only[depth] + mixed[depth]
                cursor = counters[(role, depth)]
                parent = pool[cursor % len(pool)]
                counters[(role, depth)] += 1
            name = f"item-{opaque_hmac(key, 'destination-name-v1', candidate.stable_id)[:24]}{candidate.extension}"
            assignments.append((candidate, parent / name))
    assignments.sort(key=lambda item: item[1].as_posix())
    return assignments

def directory_contract(assignments: list[tuple[Candidate, pathlib.PurePosixPath]]) -> dict[str, object]:
    direct_roles: dict[pathlib.PurePosixPath, set[str]] = {}
    all_directories: set[pathlib.PurePosixPath] = {pathlib.PurePosixPath()}
    for candidate, relative in assignments:
        parent = relative.parent
        direct_roles.setdefault(parent, set()).add(candidate.source_role)
        while parent != pathlib.PurePosixPath():
            all_directories.add(parent)
            parent = parent.parent
    non_leaf = {directory for directory in all_directories if any(child.parent == directory for child in all_directories - {directory})}
    if any(directory not in direct_roles for directory in non_leaf):
        raise ScopeInvalid
    resume_only_count = sum(roles == {"known_resume_source"} for roles in direct_roles.values())
    mixed_count = sum(roles == {"known_resume_source", "ordinary_document_source"} for roles in direct_roles.values())
    if not resume_only_count or not mixed_count:
        raise ScopeInvalid
    return {
        "max_depth": max(len(relative.parent.parts) for _, relative in assignments),
        "non_leaf_directories_have_files_and_children": True,
        "resume_only_directory_count": resume_only_count,
        "mixed_directory_count": mixed_count,
    }

def build_source(
    config: Config,
    *,
    minimum_files: int,
    maximum_files: int,
    mixed_source_label: str,
) -> dict[str, object]:
    key = load_or_create_key(config.evidence_dir)
    excluded = (config.private_root, config.mixed_root, config.evidence_dir)
    resumes, resume_observations = discover(
        config.private_root, "known_resume_source", "known_resume", key, (config.mixed_root, config.evidence_dir)
    )
    ordinary, ordinary_observations = discover(
        config.home_root, "ordinary_document_source", mixed_source_label, key, excluded
    )
    resume_fingerprints = {(item.size_bytes, item.fingerprint) for item in resumes}
    ordinary = [item for item in ordinary if (item.size_bytes, item.fingerprint) not in resume_fingerprints]
    duplicate_count = ordinary_observations["eligible_files"] - len(ordinary)
    candidates = resumes + ordinary
    candidates.sort(key=lambda item: (item.source_role, item.stable_id))
    if maximum_files > 0 and len(candidates) > maximum_files:
        if maximum_files < 2:
            raise ScopeInvalid
        resume_limit = min(len(resumes), maximum_files - 1,
                           max(1, round(maximum_files * len(resumes) / len(candidates))))
        ordinary_limit = min(len(ordinary), maximum_files - resume_limit)
        resume_limit = min(len(resumes), maximum_files - ordinary_limit)
        candidates = resumes[:resume_limit] + ordinary[:ordinary_limit]
    if not candidates:
        raise PermissionBlocked
    assignments = assign_layout(candidates, key)
    contract = directory_contract(assignments)
    entries = [{
        "relative_path": relative.as_posix(),
        "source_role": candidate.source_role,
        "label": candidate.label,
        "extension": candidate.extension.lstrip("."),
        "size_bytes": candidate.size_bytes,
        "mtime_ns": candidate.mtime_ns,
        "quick_fingerprint": candidate.fingerprint,
        "keyed_content_digest": keyed_content_digest(candidate.source, key),
        "stable_source_id": candidate.stable_id,
    } for candidate, relative in assignments]
    depth_counts = Counter(f"depth_{len(relative.parent.parts)}" for _, relative in assignments)
    layout = {
        "schema_version": "resume-ir.mixed-source-layout.v1",
        "frozen_before_classifier": True,
        "minimum_target_files": minimum_files,
        "minimum_target_met": len(entries) >= minimum_files,
        "file_count": len(entries),
        "depth_counts": dict(sorted(depth_counts.items())),
        "directory_contract": contract,
        "source_counts": dict(sorted(Counter(item.source_role for item in candidates).items())),
        "label_counts": dict(sorted(Counter(item.label for item in candidates).items())),
        "exclusions": {
            "resume": dict(sorted(resume_observations.items())),
            "ordinary": dict(sorted(ordinary_observations.items())),
            "ordinary_duplicates_of_known_resume": duplicate_count,
        },
        "entries": entries,
    }
    layout["opaque_layout_id"] = opaque_hmac(key, "layout-v1", canonical_bytes(entries))
    layout_path = config.evidence_dir / LAYOUT_FILE
    if layout_path.exists():
        if layout_path.read_bytes() != canonical_bytes(layout):
            previous = json.loads(layout_path.read_text(encoding="utf-8"))
            changed_sections = sorted(key for key in layout if previous.get(key) != layout.get(key))
            raise FreezeMismatch(",".join(changed_sections))
        if not config.mixed_root.is_dir():
            raise FreezeMismatch
        return public_build_summary(layout)
    if config.mixed_root.exists() and any(config.mixed_root.iterdir()):
        raise ScopeInvalid
    config.mixed_root.parent.mkdir(parents=True, exist_ok=True)
    staging = config.mixed_root.parent / f".{config.mixed_root.name}.staging-{secrets.token_hex(6)}"
    staging.mkdir(mode=0o700)
    try:
        for directory in set(relative.parent for _, relative in assignments):
            destination = staging / pathlib.Path(directory.as_posix())
            destination.mkdir(parents=True, exist_ok=True, mode=0o700)
            os.chmod(destination, 0o700)
        for candidate, relative in assignments:
            destination = staging / pathlib.Path(relative.as_posix())
            shutil.copy2(candidate.source, destination)
            os.chmod(destination, 0o600)
        os.replace(staging, config.mixed_root)
        os.chmod(config.mixed_root, 0o700)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    secure_write_json(layout_path, layout)
    return public_build_summary(layout)

def public_build_summary(layout: dict[str, object]) -> dict[str, object]:
    return {
        "schema_version": "resume-ir.mixed-source-build-summary.v1",
        "status": "built",
        "file_count": layout["file_count"],
        "minimum_target_files": layout["minimum_target_files"],
        "minimum_target_met": layout["minimum_target_met"],
        "depth_counts": layout["depth_counts"],
        "directory_contract": layout["directory_contract"],
        "source_counts": layout["source_counts"],
        "exclusion_counts": {
            "resume": sum(layout["exclusions"]["resume"].values()),
            "ordinary": sum(layout["exclusions"]["ordinary"].values()),
            "ordinary_duplicates_of_known_resume": layout["exclusions"]["ordinary_duplicates_of_known_resume"],
        },
        "privacy": PRIVACY,
    }

def freeze_benchmark(config: Config, *, holdout_percent: int) -> dict[str, object]:
    key = load_or_create_key(config.evidence_dir)
    layout_path = config.evidence_dir / LAYOUT_FILE
    if not layout_path.is_file() or not config.mixed_root.is_dir():
        raise PermissionBlocked
    layout = json.loads(layout_path.read_text(encoding="utf-8"))
    expected = {entry["relative_path"]: entry for entry in layout.get("entries", [])}
    observed: dict[str, dict[str, object]] = {}
    for current_text, directory_names, file_names in os.walk(config.mixed_root, followlinks=False):
        current = pathlib.Path(current_text)
        directory_names[:] = [name for name in directory_names if not (current / name).is_symlink()]
        for name in file_names:
            path = current / name
            if path.is_symlink() or not path.is_file():
                raise FreezeMismatch
            relative = path.relative_to(config.mixed_root).as_posix()
            info = path.stat()
            observed[relative] = {
                "size_bytes": info.st_size,
                "mtime_ns": info.st_mtime_ns,
                "quick_fingerprint": quick_fingerprint(path, info.st_size, key),
                "keyed_content_digest": keyed_content_digest(path, key),
            }
    if set(observed) != set(expected):
        raise FreezeMismatch
    records = []
    for relative, source in sorted(expected.items()):
        current = observed[relative]
        for field in ["size_bytes", "mtime_ns", "quick_fingerprint", "keyed_content_digest"]:
            if current[field] != source[field]:
                raise FreezeMismatch
        sample_id = opaque_hmac(key, "sample-id-v1", f"{relative}\0{source['keyed_content_digest']}")
        records.append({
            "sample_id": sample_id,
            "relative_path": relative,
            "source_role": source["source_role"],
            "label": source["label"],
            "extension": source["extension"],
            "size_bytes": source["size_bytes"],
            "mtime_ns": source["mtime_ns"],
            "quick_fingerprint": source["quick_fingerprint"],
            "keyed_content_digest": source["keyed_content_digest"],
            "stable_source_id": source["stable_source_id"],
        })
    holdout_ids: set[str] = set()
    for role in sorted({record["source_role"] for record in records}):
        stratum = [record for record in records if record["source_role"] == role]
        stratum.sort(key=lambda record: opaque_hmac(key, "layer-assignment-v1", record["sample_id"]))
        holdout_count = max(1, round(len(stratum) * holdout_percent / 100)) if len(stratum) > 1 else 0
        holdout_ids.update(record["sample_id"] for record in stratum[:holdout_count])
    calibration_entries = [record for record in records if record["sample_id"] not in holdout_ids]
    holdout_entries = [record for record in records if record["sample_id"] in holdout_ids]
    calibration = local_manifest("private_calibration", "local_tuning", calibration_entries, key, holdout_percent)
    holdout = local_manifest("blind_holdout", "acceptance_only", holdout_entries, key, holdout_percent)
    depth_counts = Counter(f"depth_{len(pathlib.PurePosixPath(record['relative_path']).parent.parts)}" for record in records)
    summary = {
        "schema_version": "resume-ir.mixed-import-freeze-summary.v1",
        "status": "frozen",
        "identity_scheme": "hmac_sha256_opaque_manifest_v1",
        "frozen_before_classifier": True,
        "mutation_after_freeze_allowed": False,
        "blind_holdout_visible_during_calibration": False,
        "sample_count": len(records),
        "layer_counts": {
            "private_calibration": len(calibration_entries),
            "blind_holdout": len(holdout_entries),
        },
        "source_counts": dict(sorted(Counter(record["source_role"] for record in records).items())),
        "label_counts": dict(sorted(Counter(record["label"] for record in records).items())),
        "extension_buckets": dict(sorted(Counter(record["extension"] for record in records).items())),
        "directory_depth_buckets": dict(sorted(depth_counts.items())),
        "opaque_manifest_ids": {
            "private_calibration": calibration["opaque_manifest_id"],
            "blind_holdout": holdout["opaque_manifest_id"],
        },
        "privacy": PRIVACY,
    }
    outputs = {
        config.evidence_dir / CALIBRATION_FILE: calibration,
        config.evidence_dir / HOLDOUT_FILE: holdout,
        config.evidence_dir / SUMMARY_FILE: summary,
    }
    existing = [path.exists() for path in outputs]
    if any(existing):
        if not all(existing) or any(path.read_bytes() != canonical_bytes(value) for path, value in outputs.items()):
            raise FreezeMismatch
    else:
        for path, value in outputs.items():
            secure_write_json(path, value)
    return summary

def local_manifest(layer: str, visibility: str, entries: list[dict[str, object]], key: bytes, holdout_percent: int) -> dict[str, object]:
    manifest = {
        "schema_version": "resume-ir.private-mixed-manifest.v1",
        "benchmark_layer": layer,
        "visibility": visibility,
        "holdout_percent": holdout_percent,
        "frozen": True,
        "mutation_after_freeze_allowed": False,
        "entries": entries,
    }
    manifest["opaque_manifest_id"] = opaque_hmac(key, f"manifest-{layer}-v1", canonical_bytes(entries))
    return manifest

def run_synthetic_smoke() -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="resume-ir-mixed-smoke-") as temporary_text:
        temporary = pathlib.Path(temporary_text).resolve()
        home = temporary / "home"
        private = home / "clean-input"
        target = home / "MLE" / "resume-ir-mixed-benchmark-smoke-main"
        evidence = temporary / "evidence"
        private.mkdir(parents=True)
        ordinary_root = home / "ordinary" / "nested" / "level"
        ordinary_root.mkdir(parents=True)
        extensions = [".doc", ".docx", ".pdf", ".txt"]
        for index in range(32):
            path = private / f"private-sentinel-{index:03d}{extensions[index % 4]}"
            path.write_bytes(f"synthetic resume {index}".encode())
        for index in range(40):
            path = ordinary_root / f"ordinary-sentinel-{index:03d}{extensions[index % 4]}"
            path.write_bytes(f"synthetic ordinary document {index}".encode())
        for excluded in [".ssh", "Library", "System", "chrome", "databases", "node_modules", "target", "runtime", "diagnostics", "models", ".git"]:
            directory = home / excluded
            directory.mkdir(parents=True, exist_ok=True)
            (directory / "excluded-sentinel.pdf").write_bytes(b"excluded synthetic payload")
        worktree = home / "worktree-marker"
        worktree.mkdir()
        (worktree / ".git").write_text("gitdir: synthetic", encoding="utf-8")
        (worktree / "worktree-excluded-sentinel.pdf").write_bytes(b"excluded worktree payload")
        prior_benchmark = home / "resume-ir-mixed-benchmark-v1"
        prior_benchmark.mkdir()
        (prior_benchmark / "prior-freeze-excluded-sentinel.pdf").write_bytes(b"excluded prior freeze")
        unreadable = ordinary_root / "permission-sentinel.txt"
        unreadable.write_bytes(b"permission synthetic payload")
        unreadable.chmod(0)
        (ordinary_root / "unsupported-sentinel.sqlite").write_bytes(b"unsupported")
        (ordinary_root / "media-sentinel.jpg").write_bytes(b"media")
        large = ordinary_root / "large-middle-mutation-sentinel.pdf"
        large.write_bytes(b"A" * (HEAD_TAIL_BYTES * 3))
        (ordinary_root / "symlink-target.txt").write_bytes(b"symlink target")
        try:
            (ordinary_root / "symlink-sentinel.txt").symlink_to(ordinary_root / "symlink-target.txt")
        except OSError:
            pass
        bounded = Config(private, home / "MLE" / "resume-ir-mixed-benchmark-smoke-bounded",
                         temporary / "bounded-evidence", home, True)
        bounded_build = build_source(bounded, minimum_files=20, maximum_files=20, mixed_source_label="unknown")
        if bounded_build["file_count"] != 20 or min(bounded_build["source_counts"].values()) < 1:
            raise ValueError("synthetic smoke: bounded selection lost a source role")
        config = Config(private, target, evidence, home, True)
        config.validate()
        build = build_source(config, minimum_files=64, maximum_files=0, mixed_source_label="known_non_resume")
        summary = freeze_benchmark(config, holdout_percent=20)
        layout = json.loads((evidence / LAYOUT_FILE).read_text(encoding="utf-8"))
        vcs_fixture = worktree / "worktree-excluded-sentinel.pdf"
        vcs_fingerprint = quick_fingerprint(vcs_fixture, vcs_fixture.stat().st_size, (evidence / KEY_FILE).read_bytes())
        if any(entry["quick_fingerprint"] == vcs_fingerprint for entry in layout["entries"]):
            raise ValueError("synthetic smoke: Git worktree content entered the freeze")
        if not 73 <= summary["sample_count"] <= 75 or build["exclusion_counts"]["ordinary"] < 11:
            raise ValueError("synthetic smoke: discovery or exclusions drifted")
        first_bytes = {name: (evidence / name).read_bytes() for name in [LAYOUT_FILE, CALIBRATION_FILE, HOLDOUT_FILE, SUMMARY_FILE]}
        build_again = build_source(config, minimum_files=64, maximum_files=0, mixed_source_label="known_non_resume")
        summary_again = freeze_benchmark(config, holdout_percent=20)
        if build != build_again or summary != summary_again:
            raise ValueError("synthetic smoke: unchanged rerun drifted")
        if any((evidence / name).read_bytes() != data for name, data in first_bytes.items()):
            raise ValueError("synthetic smoke: unchanged freeze bytes drifted")
        calibration = json.loads((evidence / CALIBRATION_FILE).read_text(encoding="utf-8"))
        holdout = json.loads((evidence / HOLDOUT_FILE).read_text(encoding="utf-8"))
        calibration_ids = {entry["sample_id"] for entry in calibration["entries"]}
        holdout_ids = {entry["sample_id"] for entry in holdout["entries"]}
        if not calibration_ids or not holdout_ids or calibration_ids & holdout_ids:
            raise ValueError("synthetic smoke: benchmark layers overlap or are empty")
        depth_counts = summary["directory_depth_buckets"]
        if set(depth_counts) != {"depth_0", "depth_1", "depth_2", "depth_3"} or max(depth_counts.values()) - min(depth_counts.values()) > 1:
            raise ValueError("synthetic smoke: depth buckets are not balanced")
        serialized = canonical_bytes(summary)
        forbidden = [
            str(temporary).encode(), b"sentinel", b"synthetic resume", b"entries",
            b"relative_path", b"classification", b"indexed", (evidence / KEY_FILE).read_bytes(),
        ]
        if len(serialized) > 32_768 or any(value in serialized for value in forbidden):
            raise ValueError("synthetic smoke: public summary leaked private detail")
        added = target / "mutation-sentinel.txt"
        added.write_bytes(b"membership mutation")
        try:
            freeze_benchmark(config, holdout_percent=20)
        except FreezeMismatch:
            pass
        else:
            raise ValueError("synthetic smoke: membership mutation was accepted")
        if any((evidence / name).read_bytes() != data for name, data in first_bytes.items()):
            raise ValueError("synthetic smoke: failed mutation rewrote freeze")
        added.unlink()
        changed_entry = next(
            entry for entry in layout["entries"] if entry["size_bytes"] > HEAD_TAIL_BYTES * 2
        )
        changed_path = target / changed_entry["relative_path"]
        original = changed_path.read_bytes()
        changed = bytearray(original)
        changed[HEAD_TAIL_BYTES + 1] ^= 1
        changed_path.write_bytes(changed)
        current = changed_path.stat()
        os.utime(changed_path, ns=(current.st_atime_ns, changed_entry["mtime_ns"]))
        try:
            freeze_benchmark(config, holdout_percent=20)
        except FreezeMismatch:
            pass
        else:
            raise ValueError("synthetic smoke: same-size content mutation was accepted")
        if any((evidence / name).read_bytes() != data for name, data in first_bytes.items()):
            raise ValueError("synthetic smoke: content mutation rewrote freeze")
        changed_path.write_bytes(original)
        os.utime(changed_path, ns=(current.st_atime_ns, changed_entry["mtime_ns"]))
        try:
            Config(temporary / "missing", target, evidence, home, True).validate()
        except PermissionBlocked:
            pass
        else:
            raise ValueError("synthetic smoke: missing private root was accepted")
        try:
            Config(private, target, evidence, home, False).validate()
        except ScopeInvalid:
            pass
        else:
            raise ValueError("synthetic smoke: HOME authorization was not enforced")
        try:
            Config(private, target, target / "evidence", home, True).validate()
        except ScopeInvalid:
            pass
        else:
            raise ValueError("synthetic smoke: overlapping roots were accepted")
        return {
            "sample_count": summary["sample_count"],
            "layer_counts": summary["layer_counts"],
            "directory_depth_buckets": depth_counts,
            "privacy": PRIVACY,
        }

def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description="Build and freeze a local mixed-document benchmark.")
    subcommands = result.add_subparsers(dest="command", required=True)
    prepare = subcommands.add_parser("prepare")
    prepare.add_argument("--minimum-files", type=int, default=20_000)
    prepare.add_argument("--maximum-files", type=int, default=0)
    prepare.add_argument("--blind-holdout-percent", type=int, default=20)
    prepare.add_argument("--mixed-source-label", choices=["unknown", "known_non_resume"], default="unknown")
    verify = subcommands.add_parser("verify")
    verify.add_argument("--blind-holdout-percent", type=int)
    subcommands.add_parser("synthetic-smoke")
    return result

def main(argv: list[str] | None = None, *, stdout: TextIO = sys.stdout, stderr: TextIO = sys.stderr) -> int:
    arguments = parser().parse_args(argv)
    try:
        if arguments.command == "synthetic-smoke":
            result = run_synthetic_smoke()
            print(json.dumps({"status": "passed", **result}, sort_keys=True), file=stdout)
            return 0
        config = Config.from_environment()
        if arguments.command == "prepare":
            if arguments.minimum_files < 1 or arguments.maximum_files < 0 or not 1 <= arguments.blind_holdout_percent <= 50:
                raise ScopeInvalid
            build_source(
                config,
                minimum_files=arguments.minimum_files,
                maximum_files=arguments.maximum_files,
                mixed_source_label=arguments.mixed_source_label,
            )
            result = freeze_benchmark(config, holdout_percent=arguments.blind_holdout_percent)
        else:
            holdout_percent = arguments.blind_holdout_percent
            if holdout_percent is None:
                manifest_path = config.evidence_dir / CALIBRATION_FILE
                if not manifest_path.is_file():
                    raise PermissionBlocked
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
                holdout_percent = manifest.get("holdout_percent")
            if not isinstance(holdout_percent, int) or not 1 <= holdout_percent <= 50:
                raise ScopeInvalid
            result = freeze_benchmark(config, holdout_percent=holdout_percent)
        print(json.dumps(result, sort_keys=True), file=stdout)
        return 0
    except PermissionBlocked:
        print("mixed import benchmark: blocked_permission", file=stderr)
        return 3
    except FreezeMismatch:
        print("mixed import benchmark: freeze_mismatch", file=stderr)
        return 4
    except (OSError, ScopeInvalid, ValueError, json.JSONDecodeError):
        print("mixed import benchmark: invalid_scope_or_local_state", file=stderr)
        return 5

if __name__ == "__main__":
    raise SystemExit(main())
