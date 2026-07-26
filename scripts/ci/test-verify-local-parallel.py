#!/usr/bin/env python3
"""Regression checks for the resumable local parallel verification runner."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import pathlib
import subprocess
import sys
import tempfile
from types import SimpleNamespace


ROOT = pathlib.Path(__file__).resolve().parents[2]
RUNNER_PATH = ROOT / "scripts" / "ci" / "verify-local-parallel.py"


def load_runner() -> object:
    specification = importlib.util.spec_from_file_location("verify_local_parallel", RUNNER_PATH)
    if specification is None or specification.loader is None:
        raise AssertionError("could not load parallel verification runner")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def marker_command(
    name: str,
    *,
    barrier_peer: str | None = None,
    fail: bool = False,
    exclusive_lock: str | None = None,
) -> list[str]:
    program = f"""
from pathlib import Path
import os
import sys
import time

marker = Path('calls') / {name!r}
marker.parent.mkdir(parents=True, exist_ok=True)
count = int(marker.read_text(encoding='utf-8')) if marker.exists() else 0
marker.write_text(str(count + 1), encoding='utf-8')
"""
    if exclusive_lock:
        program += f"""
lock = Path({exclusive_lock!r})
try:
    descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
except FileExistsError:
    raise SystemExit(97)
time.sleep(0.08)
os.close(descriptor)
lock.unlink()
"""
    if barrier_peer:
        program += f"""
ready = Path({f'barrier-{name}'!r})
ready.touch()
for _ in range(100):
    if Path({f'barrier-{barrier_peer}'!r}).exists():
        break
    time.sleep(0.01)
else:
    raise SystemExit(98)
"""
    if fail:
        program += "raise SystemExit(5)\n"
    return [sys.executable, "-c", program]


def check_count(root: pathlib.Path, name: str, expected: int) -> None:
    observed = int((root / "calls" / name).read_text(encoding="utf-8"))
    if observed != expected:
        raise AssertionError(f"{name} ran {observed} times, expected {expected}")


def invoke(runner: object, root: pathlib.Path, *extra: str) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    arguments = [
        "--root",
        str(root),
        "--manifest",
        "manifest.json",
        "--state-dir",
        "state",
        "--jobs",
        "4",
        "--cargo-build-jobs",
        "2",
        *extra,
    ]
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        exit_code = runner.main(arguments)
    return exit_code, stdout.getvalue(), stderr.getvalue()


def main() -> int:
    runner = load_runner()
    one_cargo_check = [SimpleNamespace(resources=("cargo",))]
    three_cargo_checks = [SimpleNamespace(resources=("cargo",)) for _ in range(3)]
    if runner.cargo_worker_budgets(one_cargo_check, {"cargo": 3}, 10, 10) != (10, 10):
        raise AssertionError("a single selected Cargo check did not receive the full worker budget")
    if runner.cargo_worker_budgets(three_cargo_checks, {"cargo": 3}, 10, 10) != (3, 3):
        raise AssertionError("three selected Cargo checks did not share the worker budget")
    with tempfile.TemporaryDirectory(prefix="resume-ir-parallel-runner-test-") as temporary:
        root = pathlib.Path(temporary)
        for name in (
            "input-a.txt",
            "input-b.txt",
            "barrier-input.txt",
            "cargo-input.txt",
            "native-runtime-input.txt",
            "failure-input.txt",
        ):
            (root / name).write_text("v1\n", encoding="utf-8")
        legacy_state = root / "legacy-state.json"
        legacy_state.write_text(
            json.dumps(
                {
                    "schema_version": "resume-ir.local-verification-ledger.v1",
                    "checks": {"carried": {"outcome": "passed", "fingerprint": "f" * 64}},
                }
            ),
            encoding="utf-8",
        )
        migrated = runner.load_state(legacy_state)
        if (
            migrated["schema_version"] != "resume-ir.local-verification-ledger.v2"
            or migrated["carried_from_schema"]
            != "resume-ir.local-verification-ledger.v1"
            or migrated["checks"] != {"carried": {"outcome": "passed", "fingerprint": "f" * 64}}
        ):
            raise AssertionError("legacy passed evidence was not carried into the v2 ledger")
        manifest = {
            "schema_version": "resume-ir.verify-local-parallel-manifest.v2",
            "resources": {"cargo": 1, "native-runtime": 1, "packaging": 1},
            "input_sets": {},
            "checks": [
                {
                    "id": "a",
                    "behavior": "unchanged inputs reuse a successful result",
                    "command": marker_command("a"),
                    "inputs": ["input-a.txt"],
                },
                {
                    "id": "b",
                    "behavior": "dry-run preserves a reusable successful result",
                    "command": marker_command("b"),
                    "inputs": ["input-b.txt"],
                },
                {
                    "id": "parallel-a",
                    "behavior": "independent checks overlap when worker capacity permits",
                    "command": marker_command("parallel-a", barrier_peer="parallel-b"),
                    "inputs": ["barrier-input.txt"],
                },
                {
                    "id": "parallel-b",
                    "behavior": "independent checks overlap when worker capacity permits",
                    "command": marker_command("parallel-b", barrier_peer="parallel-a"),
                    "inputs": ["barrier-input.txt"],
                },
                {
                    "id": "cargo-a",
                    "behavior": "shared Cargo resource remains exclusive",
                    "command": marker_command("cargo-a", exclusive_lock="cargo-exclusive.lock"),
                    "inputs": ["cargo-input.txt"],
                    "resources": ["cargo"],
                },
                {
                    "id": "cargo-b",
                    "behavior": "shared Cargo resource remains exclusive",
                    "command": marker_command("cargo-b", exclusive_lock="cargo-exclusive.lock"),
                    "inputs": ["cargo-input.txt"],
                    "resources": ["cargo"],
                },
                {
                    "id": "native-runtime-a",
                    "behavior": "shared native runtime resource remains exclusive",
                    "command": marker_command(
                        "native-runtime-a", exclusive_lock="native-runtime-exclusive.lock"
                    ),
                    "inputs": ["native-runtime-input.txt"],
                    "resources": ["native-runtime"],
                },
                {
                    "id": "native-runtime-b",
                    "behavior": "shared native runtime resource remains exclusive",
                    "command": marker_command(
                        "native-runtime-b", exclusive_lock="native-runtime-exclusive.lock"
                    ),
                    "inputs": ["native-runtime-input.txt"],
                    "resources": ["native-runtime"],
                },
                {
                    "id": "fails",
                    "behavior": "failed checks retry instead of being reused",
                    "command": marker_command("fails", fail=True),
                    "inputs": ["failure-input.txt"],
                },
                {
                    "id": "worktree-package",
                    "behavior": "a worktree snapshot packaging check runs on dirty source",
                    "command": marker_command("worktree-package"),
                    "inputs": ["input-b.txt"],
                    "resources": ["packaging"],
                    "source_authority": "worktree_snapshot",
                    "evidence_lane": "gui_manual",
                    "claim": "manual_test",
                    "produces_artifact": True,
                },
                {
                    "id": "exact-main-package",
                    "behavior": "an exact-main packaging check blocks before command execution",
                    "command": marker_command("exact-main-package"),
                    "inputs": ["input-a.txt"],
                    "resources": ["packaging"],
                    "source_authority": "exact_main_commit",
                    "evidence_lane": "gui_manual",
                    "claim": "release_gate",
                    "produces_artifact": True,
                },
            ],
        }
        (root / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=root, check=True)
        subprocess.run(["git", "config", "user.email", "synthetic@example.invalid"], cwd=root, check=True)
        subprocess.run(["git", "config", "user.name", "Synthetic"], cwd=root, check=True)
        subprocess.run(["git", "add", "."], cwd=root, check=True)
        subprocess.run(["git", "commit", "-qm", "synthetic"], cwd=root, check=True)
        subprocess.run(
            ["git", "remote", "add", "origin", "https://github.com/FrankQDWang/resume-ir.git"],
            cwd=root,
            check=True,
        )
        subprocess.run(
            ["git", "update-ref", "refs/remotes/origin/main", "HEAD"],
            cwd=root,
            check=True,
        )
        (root / "dirty.txt").write_text("worktree snapshot input\n", encoding="utf-8")

        first, first_stdout, _ = invoke(runner, root)
        if first != 1:
            raise AssertionError(f"first run should report the intentional failure, got {first}")
        if "BLOCK exact-main-package (exact_main_source_required)" not in first_stdout:
            raise AssertionError("dirty exact-main packaging was not blocked before execution")
        for name in (
            "a",
            "b",
            "parallel-a",
            "parallel-b",
            "cargo-a",
            "cargo-b",
            "native-runtime-a",
            "native-runtime-b",
            "fails",
            "worktree-package",
        ):
            check_count(root, name, 1)
        if (root / "calls" / "exact-main-package").exists():
            raise AssertionError("blocked exact-main packaging command executed")

        second, _, _ = invoke(runner, root)
        if second != 1:
            raise AssertionError(f"second run should retry only the failure, got {second}")
        for name in (
            "a",
            "b",
            "parallel-a",
            "parallel-b",
            "cargo-a",
            "cargo-b",
            "native-runtime-a",
            "native-runtime-b",
            "worktree-package",
        ):
            check_count(root, name, 1)
        check_count(root, "fails", 2)

        (root / "input-a.txt").write_text("v2\n", encoding="utf-8")
        third, _, _ = invoke(runner, root)
        if third != 1:
            raise AssertionError(f"input invalidation run should retain the intentional failure, got {third}")
        check_count(root, "a", 2)
        for name in (
            "b",
            "parallel-a",
            "parallel-b",
            "cargo-a",
            "cargo-b",
            "native-runtime-a",
            "native-runtime-b",
            "worktree-package",
        ):
            check_count(root, name, 1)
        check_count(root, "fails", 3)

        dry_run, stdout, _ = invoke(runner, root, "--only", "b", "--dry-run")
        if dry_run != 0 or "REUSE b" not in stdout:
            raise AssertionError("dry-run did not preserve a reusable successful result")
        check_count(root, "b", 1)

        stale_owner = subprocess.Popen([sys.executable, "-c", "pass"])
        stale_owner.wait()
        lock_path = root / "state" / "runner.lock"
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        lock_path.write_text(f"pid={stale_owner.pid}\nstarted_at=stale\n", encoding="utf-8")
        stale_recovery, _, _ = invoke(runner, root, "--only", "b")
        if stale_recovery != 0:
            raise AssertionError("runner did not recover a lock whose owner exited")
        check_count(root, "b", 1)

        rounds = list((root / "state" / "runs").glob("*.json"))
        if len(rounds) != 4:
            raise AssertionError(f"expected four immutable round records, found {len(rounds)}")
        if (root / "state" / "runner.lock").exists():
            raise AssertionError("runner lock was not released")
    print("parallel verification runner self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
