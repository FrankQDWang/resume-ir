# Redaction Counter Test Isolation Spec

Status: approved by independent plan review
Date: 2026-07-10
Linked issue: https://github.com/FrankQDWang/resume-ir/issues/143
Blocked PR: https://github.com/FrankQDWang/resume-ir/pull/142

## Problem

`index-fulltext` uses one process-global test-only `AtomicUsize` to count contact
redaction regex passes. Three tests reset and read that counter while other tests
can increment it concurrently. Normal parallel test execution therefore lets one
test observe another test's work.

Fresh evidence establishes the race rather than a production regression:

1. Windows Platform CI failed the same zero-counter assertion twice.
2. The exact failing test passes in isolation.
3. A normal local parallel `index-fulltext` lib suite fails a sibling zero-counter
   assertion.
4. The serialized 22-test suite passes.

PR #142 changes only mixed-import contracts and documentation. It must not absorb
the unrelated Rust repair, and it must not merge while Platform CI remains red.

## Decision

Select issue #143 as a temporary, bounded test-infrastructure slice. The
authorization PR and Rust repair PR remain separate:

1. A contract-only PR selects #143, sets ordinary allowed paths to exactly
   `crates/index-fulltext/src/lib.rs` and `PROGRESS.md`, marks the slice as
   test-only, forbids production-semantics, global-test-serialization, further
   arbitrary contract changes, and private-benchmark work, and records the
   #140/PR #142 dependency.
2. Only after that contract is main-reachable may a new #143 implementation branch
   replace the global test counter with per-thread test observation.
3. After #143 merges, a contract-only reconciliation returns the active slice to
   #140; PR #142 then refreshes from `main` and reruns all affected gates.

The counter remains compiled only under `cfg(test)`. Production redaction behavior,
regex selection, output, privacy boundaries, and public APIs do not change.

## Contract Scope

Ordinary #143 changes are exactly `crates/index-fulltext/src/lib.rs` and
`PROGRESS.md`, limited to test-only counter helpers, regression coverage, and
public evidence. The contract pins linked spec/plan paths,
`contract_change_allowed=false`, no production semantics/global serialization,
and an exact return target of #140. Loop checks reject issue/current-slice drift.

The contract PR itself is authorized by the merge-base #140 contract, not by the
new #143 head contract. `check-gate-integrity.py` must load both contracts and
enforce an exact #140 -> #143 transition file whitelist. Ordinary same-issue #143
PRs must use the merge-base allowed paths and an exact approved before/after
SHA-256 pair for `lib.rs`; this pins the thread-local helpers, call sites, and
regression test without trusting a textual Rust parser. The reverse #143 -> #140
reconciliation must be permitted only by #143's explicit transition target and
exact reverse file set.

Scope checks cover committed, staged, unstaged, and untracked paths and rerun
after commit. The snapshot keeps only open PR #142. Production semantics, global
serialization, workflow/gate/threshold changes, private reads, mixed-import,
GUI/query/L4 work, and changes to PR #142 are forbidden.

## Test-Instrumentation Design

Use test-only thread-local `Cell<usize>` helpers to record, reset, and read each
thread. A deterministic worker-thread regression proves parent isolation. Global
mutexes or `--test-threads=1` would mask rather than fix the race and are rejected.

## Acceptance

The contract-authorization PR is accepted only when all local contract/privacy
gates and every hosted check, including macOS and Windows Platform CI, are green.
Run the default full local verifier once as a negative control; until #143 is
fixed, its sole permitted failure is one of the three shared-counter assertions.
That known failure must be recorded, not rerun without new evidence, and does not
replace the requirement for every hosted check to be green before merge.

The Rust PR requires the deterministic and three existing assertions, twenty
normal-parallel lib-suite passes, focused tests, fmt/clippy/rust-analyzer, goal
and privacy gates, and all hosted checks. The exact approved source digest keeps
the `#[cfg(not(test))]` recorder and production code unchanged and excludes
`--test-threads=1` and `RUST_TEST_THREADS`.

No step in this repair claims mixed-import contract completion, classifier
readiness, private calibration, D10K/D100K/D1M, GUI readiness, stable release, or
`goal_complete`.
