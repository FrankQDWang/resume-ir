#!/usr/bin/env python3
"""Run local verification checks concurrently without re-running valid results.

The canonical ``verify-local.sh`` remains the exact serial release gate. This
local developer/recovery runner uses a manifest, input fingerprints, and a
local-only ledger under ``.cache/`` to fill independent worker slots without
running concurrent commands that share Cargo or packaging state.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import os
import pathlib
import subprocess
import sys
import time
import uuid
from collections.abc import Sequence

from verify_local_parallel_support import (
    Check,
    CheckResult,
    DEFAULT_MANIFEST,
    DEFAULT_STATE_DIR,
    InputHasher,
    RunnerError,
    acquire_lock,
    command_for,
    fail,
    fingerprint_for,
    is_reusable,
    load_state,
    read_manifest,
    release_lock,
    resolve_cargo,
    resolve_under,
    source_context_block,
    tail,
    update_state,
    utc_now,
    write_json_atomically,
    write_round,
)


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least one")
    return parsed


def select_checks(checks: Sequence[Check], selected: Sequence[str]) -> tuple[Check, ...]:
    if not selected:
        return tuple(checks)
    requested = {identifier for value in selected for identifier in value.split(",") if identifier}
    known = {check.identifier for check in checks}
    unknown = sorted(requested - known)
    if unknown:
        fail(f"unknown parallel verification check: {', '.join(unknown)}")
    return tuple(check for check in checks if check.identifier in requested)


def reusable_result(check: Check, fingerprint: str) -> CheckResult:
    return CheckResult(
        check=check,
        fingerprint=fingerprint,
        outcome="reused",
        exit_code=0,
        duration_seconds=0.0,
        log_path=None,
    )


def blocked_context_result(check: Check, fingerprint: str, reason: str) -> CheckResult:
    return CheckResult(
        check=check,
        fingerprint=fingerprint,
        outcome="blocked_context",
        exit_code=None,
        duration_seconds=0.0,
        log_path=None,
        context_reason=reason,
    )


def planned_checks(
    checks: Sequence[Check],
    fingerprints: dict[str, str],
    state: dict[str, object],
    resume: bool,
) -> tuple[list[Check], list[CheckResult]]:
    planned: list[Check] = []
    reused: list[CheckResult] = []
    records = state["checks"]
    if not isinstance(records, dict):
        fail("local verification ledger checks must be an object")
    for check in checks:
        fingerprint = fingerprints[check.identifier]
        if resume and is_reusable(records.get(check.identifier), fingerprint):
            reused.append(reusable_result(check, fingerprint))
        else:
            planned.append(check)
    return planned, reused


def execute_check(
    root: pathlib.Path,
    check: Check,
    command: tuple[str, ...],
    fingerprint: str,
    state_dir: pathlib.Path,
    run_id: str,
    cargo_build_jobs: int,
    rust_test_threads: int,
) -> CheckResult:
    log_dir = state_dir / "logs" / run_id
    log_dir.mkdir(parents=True, exist_ok=True)
    log_path = log_dir / f"{check.identifier}.log"
    environment = os.environ.copy()
    environment["CARGO_BUILD_JOBS"] = str(cargo_build_jobs)
    environment["RUST_TEST_THREADS"] = str(rust_test_threads)
    started = time.monotonic()
    with log_path.open("wb") as handle:
        try:
            completed = subprocess.run(
                command,
                cwd=root,
                env=environment,
                stdout=handle,
                stderr=subprocess.STDOUT,
                check=False,
            )
            exit_code: int | None = completed.returncode
        except OSError as error:
            handle.write(f"could not start command: {error}\n".encode("utf-8", errors="replace"))
            exit_code = None
    return CheckResult(
        check=check,
        fingerprint=fingerprint,
        outcome="passed" if exit_code == 0 else "failed",
        exit_code=exit_code,
        duration_seconds=time.monotonic() - started,
        log_path=log_path.relative_to(state_dir).as_posix(),
    )


def print_result(result: CheckResult) -> None:
    if result.outcome == "reused":
        print(f"REUSE {result.check.identifier}")
    elif result.outcome == "not_scheduled":
        print(f"SKIP  {result.check.identifier} (fail-fast)")
    elif result.outcome == "blocked_context":
        print(f"BLOCK {result.check.identifier} ({result.context_reason})")
    elif result.outcome == "passed":
        print(f"PASS  {result.check.identifier} ({result.duration_seconds:.1f}s)")
    else:
        print(f"FAIL  {result.check.identifier} exit={result.exit_code} ({result.duration_seconds:.1f}s)")


def resource_available(check: Check, in_use: dict[str, int], limits: dict[str, int]) -> bool:
    return all(in_use[name] < limits[name] for name in check.resources)


def reserve(check: Check, in_use: dict[str, int], delta: int) -> None:
    for name in check.resources:
        in_use[name] += delta


def cargo_worker_budgets(
    checks: Sequence[Check],
    limits: dict[str, int],
    jobs: int,
    cargo_build_jobs: int,
) -> tuple[int, int]:
    selected_cargo_checks = sum("cargo" in check.resources for check in checks)
    cargo_slots = min(limits.get("cargo", 1), max(1, selected_cargo_checks))
    return (
        max(1, cargo_build_jobs // cargo_slots),
        max(1, jobs // cargo_slots),
    )


def failed_result(check: Check, fingerprint: str, error: Exception) -> CheckResult:
    print(f"parallel verification: internal runner failure for {check.identifier}: {error}", file=sys.stderr)
    return CheckResult(check, fingerprint, "failed", None, 0.0, None)


def run_scheduled_checks(
    root: pathlib.Path,
    state_dir: pathlib.Path,
    state_path: pathlib.Path,
    state: dict[str, object],
    checks: Sequence[Check],
    commands: dict[str, tuple[str, ...]],
    fingerprints: dict[str, str],
    limits: dict[str, int],
    args: argparse.Namespace,
    run_id: str,
) -> list[CheckResult]:
    pending = list(checks)
    results: list[CheckResult] = []
    running: dict[concurrent.futures.Future[CheckResult], Check] = {}
    in_use = {name: 0 for name in limits}
    cargo_build_jobs, rust_test_threads = cargo_worker_budgets(
        checks,
        limits,
        args.jobs,
        args.cargo_build_jobs,
    )
    failed = False
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
        while pending or running:
            scheduled = False
            if not (args.fail_fast and failed):
                for check in list(pending):
                    if len(running) >= args.jobs or not resource_available(check, in_use, limits):
                        continue
                    reserve(check, in_use, 1)
                    future = executor.submit(
                        execute_check,
                        root,
                        check,
                        commands[check.identifier],
                        fingerprints[check.identifier],
                        state_dir,
                        run_id,
                        cargo_build_jobs,
                        rust_test_threads,
                    )
                    running[future] = check
                    pending.remove(check)
                    scheduled = True
            if scheduled:
                continue
            if not running:
                if args.fail_fast and failed:
                    results.extend(
                        CheckResult(check, fingerprints[check.identifier], "not_scheduled", None, 0.0, None)
                        for check in pending
                    )
                    pending.clear()
                    break
                fail(f"parallel verification could not schedule checks: {', '.join(check.identifier for check in pending)}")
            done, _ = concurrent.futures.wait(running, return_when=concurrent.futures.FIRST_COMPLETED)
            for future in done:
                check = running.pop(future)
                reserve(check, in_use, -1)
                try:
                    result = future.result()
                except Exception as error:  # defensive: a runner crash must not strand the ledger lock
                    result = failed_result(check, fingerprints[check.identifier], error)
                results.append(result)
                update_state(state, result)
                write_json_atomically(state_path, state)
                print_result(result)
                if result.outcome == "failed":
                    failed = True
                    if result.log_path and (output := tail(state_dir / result.log_path)):
                        print(output.rstrip(), file=sys.stderr)
    return results


def run(args: argparse.Namespace) -> int:
    root = pathlib.Path(args.root).resolve()
    manifest_path = resolve_under(root, args.manifest).resolve()
    state_dir = resolve_under(root, args.state_dir).resolve()
    limits, input_sets, checks = read_manifest(manifest_path)
    checks = select_checks(checks, args.only)
    cargo = resolve_cargo() if any("{cargo}" in part for check in checks for part in check.command) else ""
    commands = {check.identifier: command_for(check, cargo) for check in checks}
    input_hasher = InputHasher(root)
    fingerprints = {
        check.identifier: fingerprint_for(root, check, input_sets, commands[check.identifier], input_hasher)
        for check in checks
    }
    if args.list:
        for check in checks:
            resources = ",".join(check.resources) if check.resources else "none"
            print(
                f"{check.identifier}\tbehavior={check.behavior}\t"
                f"resources={resources}\t{' '.join(commands[check.identifier])}"
            )
        return 0

    state_path = state_dir / "state.json"
    context_blocks = [
        blocked_context_result(check, fingerprints[check.identifier], reason)
        for check in checks
        if (reason := source_context_block(root, check)) is not None
    ]
    context_block_ids = {result.check.identifier for result in context_blocks}
    runnable_checks = tuple(
        check for check in checks if check.identifier not in context_block_ids
    )
    if args.dry_run:
        planned, reused = planned_checks(
            runnable_checks, fingerprints, load_state(state_path), not args.no_resume
        )
        for result in reused:
            print_result(result)
        for result in context_blocks:
            print_result(result)
        for check in planned:
            resources = ",".join(check.resources) if check.resources else "none"
            print(f"RUN   {check.identifier} resources={resources}")
        return 1 if context_blocks else 0

    lock_path = acquire_lock(state_dir)
    run_id = f"{dt.datetime.now(dt.UTC):%Y%m%dT%H%M%SZ}-{uuid.uuid4().hex[:8]}"
    started_at = utc_now()
    started = time.monotonic()
    results: list[CheckResult] = []
    try:
        state = load_state(state_path)
        planned, reused = planned_checks(
            runnable_checks, fingerprints, state, not args.no_resume
        )
        results.extend(reused)
        for result in reused:
            print_result(result)
        results.extend(context_blocks)
        for result in context_blocks:
            update_state(state, result)
            print_result(result)
        if context_blocks:
            write_json_atomically(state_path, state)
        results.extend(
            run_scheduled_checks(
                root, state_dir, state_path, state, planned, commands, fingerprints, limits, args, run_id
            )
        )
        elapsed = time.monotonic() - started
        work_seconds = sum(
            result.duration_seconds for result in results if result.outcome not in {"reused", "not_scheduled"}
        )
        write_round(state_dir, run_id, started_at, results, args.jobs, elapsed, work_seconds)
    finally:
        release_lock(lock_path)

    executed = [
        result
        for result in results
        if result.outcome not in {"reused", "not_scheduled", "blocked_context"}
    ]
    failures = [
        result for result in results if result.outcome in {"failed", "blocked_context"}
    ]
    work_seconds = sum(result.duration_seconds for result in executed)
    elapsed = time.monotonic() - started
    parallelism = work_seconds / elapsed if elapsed else 0.0
    print(
        "SUMMARY "
        f"executed={len(executed)} "
        f"reused={sum(result.outcome == 'reused' for result in results)} "
        f"blocked_context={sum(result.outcome == 'blocked_context' for result in results)} "
        f"failed={len(failures)} wall={elapsed:.1f}s work={work_seconds:.1f}s "
        f"speedup_vs_serial={parallelism:.2f}"
    )
    return 1 if failures else 0


def parse_arguments(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root (default: current directory)")
    parser.add_argument("--manifest", default=str(DEFAULT_MANIFEST), help="manifest path relative to root")
    parser.add_argument("--state-dir", default=str(DEFAULT_STATE_DIR), help="local ledger directory relative to root")
    parser.add_argument("--jobs", type=positive_int, default=min(os.cpu_count() or 1, 10))
    parser.add_argument("--cargo-build-jobs", type=positive_int, default=min(os.cpu_count() or 1, 10))
    parser.add_argument("--only", action="append", default=[], help="comma-separated check ids to run")
    parser.add_argument("--no-resume", action="store_true", help="ignore matching passed records")
    parser.add_argument("--dry-run", action="store_true", help="print reusable and scheduled checks without running")
    parser.add_argument("--list", action="store_true", help="list selected checks without reading the ledger")
    parser.add_argument("--fail-fast", action="store_true", help="stop scheduling new checks after the first failure")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    try:
        return run(parse_arguments(sys.argv[1:] if argv is None else argv))
    except RunnerError as error:
        print(f"parallel verification: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
