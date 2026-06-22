# Performance Goal Documentation Contract Implementation Plan

**Status:** implemented; pending `fw-review`
**Date:** 2026-06-22
**Linked spec:** `docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md`
**Execution owner:** `fw-build`
**Scope:** documentation and public machine-readable contract files only

## Goal

Harden PR #10's documentation package into a reliable execution contract for the upcoming high-performance local search, GUI, and manual/Codex closed-loop phase.

This plan repairs the request-changes review findings without starting production performance, daemon, indexing, query parser, GUI, or private benchmark implementation.

## Allowed Paths

- `GOAL.md`
- `MANIFEST.md`
- `ACTIVE_GOAL.toml`
- `03_next_goal_高性能本地检索GUI闭环/`
- `docs/superpowers/specs/`
- `docs/superpowers/plans/`
- `perf/`

## Explicitly Out of Scope

- `crates/`
- `scripts/`
- `Cargo.toml`
- `Cargo.lock`
- test source files
- real resumes
- raw query files
- local artifacts
- diagnostics packages
- model caches
- private 10k import benchmark execution
- private query benchmark execution

## Task Checklist

- [x] **Task 1: Root authority and reading order**

  Updated `GOAL.md`, `MANIFEST.md`, and `00_阅读顺序.md` so the previous current-stage boundary no longer weakens the active next-goal contract.

- [x] **Task 2: Reviewer issue ledger**

  Updated `12_Review问题映射与修复责任.md` so R01-R21 include `status`, `closure_evidence`, and `closed_by`.

- [x] **Task 3: Loop Engineering state machine**

  Updated `13_Loop_Engineering状态机.md` to split `slice_complete` from `goal_complete`, add blocked-stop rules, and add performance experiment states from `contract_locked` through `complete`.

- [x] **Task 4: Query semantics and anti-overfit contract**

  Updated `05_Query_Benchmark与真实Query种子.md` so simple text query means required-all normalized terms, with exact metamorphic candidate-set rules, tune/holdout evidence, and drift policy.

- [x] **Task 5: Daemon IPC and diagnostics contract**

  Updated `06_Daemon_IPC与Diagnostics契约.md` so clients declare `client_capability`, while daemon derives internal `client_class`; added local transport/framing, request-size, benchmark registration, cancellation, overload, weighted fairness, and no-raw-query diagnostics rules.

- [x] **Task 6: Adaptive governor and failure modes**

  Updated `04_数据流与状态机.md` and `08_失败模式与恢复策略.md` with concrete governor thresholds, hysteresis/dwell rules, machine contract parse failure, full-computer traversal failures, and journal gap recovery.

- [x] **Task 7: W0/W1 acceptance matrix**

  Updated `14_W0_W1验收矩阵与证据协议.md` with TOML/JSON parse gates, W1 10k/8k/500-query redlines, bucket P95/P99 thresholds, stage thresholds, hot-path flags, zero-change incremental redlines, and GUI bakeoff pressure criteria.

- [x] **Task 8: GUI bakeoff redlines**

  Updated `07_GUI与手工Codex闭环.md` to require 100000 logical rows, at least 100 visible rows, 10Hz update pressure, 10qps interactive search mock, stable row height, and responsive/a11y coverage.

- [x] **Task 9: Security, encryption, and machine enforcement**

  Updated `09_安全隐私与本地证据边界.md` so encryption mode requires P3 ADR before storage/hot-path implementation, and anti-overfit/privacy rules have machine enforcement gates.

- [x] **Task 10: Implementation order**

  Updated `10_实施切片与验收门槛.md` so P3 is snapshot/encryption ADR before hot-path and storage-shape implementation; completion gates now reference `perf/acceptance-matrix.toml`.

- [x] **Task 11: Profiling contract**

  Added `15_性能观测与Profiling工具链.md` defining spans, histograms, profiler summaries, baseline sequence, and evidence redlines.

- [x] **Task 12: Platform discovery and journal contract**

  Added `16_跨平台全盘发现与增量Journal契约.md` defining root-set semantics, symlink/reparse/cloud/permission behavior, macOS FSEvents, Windows USN, and fallback reconciliation.

- [x] **Task 13: Machine-readable protocol doc**

  Added `17_机器可读Goal与Experiment协议.md` explaining active goal lock, experiment report rules, review closure, and parse gates.

- [x] **Task 14: Public machine contracts**

  Added `ACTIVE_GOAL.toml`, `perf/acceptance-matrix.toml`, `perf/loop-state.schema.json`, `perf/experiment-report.schema.json`, and `perf/README.md`.

## Verification Plan

Run before commit:

```bash
python3 - <<'PY'
import json
import pathlib
import tomllib

tomllib.loads(pathlib.Path("ACTIVE_GOAL.toml").read_text())
tomllib.loads(pathlib.Path("perf/acceptance-matrix.toml").read_text())
json.loads(pathlib.Path("perf/loop-state.schema.json").read_text())
json.loads(pathlib.Path("perf/experiment-report.schema.json").read_text())
PY

git diff --check -- GOAL.md MANIFEST.md ACTIVE_GOAL.toml docs/superpowers 03_next_goal_高性能本地检索GUI闭环 perf

{
  git diff --name-only origin/main...HEAD
  git diff --name-only --cached
  git diff --name-only
  git ls-files --others --exclude-standard
} | sort -u

./scripts/ci/guard-public-repo.sh
```

Expected:

1. TOML and JSON parse.
2. diff whitespace check passes.
3. changed paths stay inside allowed docs/data scope.
4. public repository guard passes.

## Review Handoff

After this plan is implemented, use `fw-review` for PR #10. Keep PR #10 as Draft until reviewer-requested documentation blockers are accepted. Do not start P1+ performance/daemon/GUI implementation from this plan.

## GSTACK REVIEW REPORT

| Review | Trigger | Status | Findings |
|---|---|---|---|
| CEO Review | `fw-ceo-review` | CLEAR | Direction accepted earlier for docs-only contract hardening before broad performance/GUI work |
| Eng Review | `fw-plan-review` | CLEAR | Earlier plan issues folded into docs-only hardening scope |
| Design Review | `fw-plan-review` | CLEAR | GUI IA, state matrix, journey, responsive/a11y, and design-token constraints included |
| Request-changes Repair | external PR review | IMPLEMENTED | Machine contracts, W1 thresholds, Loop experiment states, profiling, full-computer/journal, IPC fairness, and review closure fields added |
