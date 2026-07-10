# Parse-Result Cancel-Poll Test Isolation Spec

Status: approved by independent plan and gate-integrity review
Date: 2026-07-10
Linked issue: https://github.com/FrankQDWang/resume-ir/issues/145
Original capability issue: https://github.com/FrankQDWang/resume-ir/issues/96
Blocked PR: https://github.com/FrankQDWang/resume-ir/pull/144

## Problem

`import-pipeline::tests::recv_parse_result_polls_cancel_while_waiting` starts an
unsynchronized sender that sleeps 180 ms, while the receiver polls cancellation
only after each 50 ms `recv_timeout`. The test assumes the receiver observes at
least two timeouts. On a delayed parallel runner, the sender can enqueue the
result before the receiver completes its second timeout, so the assertion sees
fewer than two polls.

PR #144 changed no import-pipeline source: its base and head use the same blob,
`ebe399c1f99cdf09ba80b696519ced69053e604a`. That blob passed macOS in PR #139
and Windows in PR #144, but failed macOS run `29068476119`, job `86284945029`,
with 31 of 32 import-pipeline unit tests passing. This is scheduler-dependent
test evidence, not a demonstrated production cancellation regression.

## Decision

Freeze PR #144 at `b093b1a`: do not merge, edit, retarget, close/reopen, or rerun
it unchanged. Resolve the new blocker through three separate changes:

1. A contract-only #140 -> #145 PR selects a bounded test-only slice.
2. After that contract is main-reachable, a separate Rust PR replaces the fixed
   sleep with a deterministic release-channel handshake.
3. A separate #145 -> #140 contract PR restores the benchmark-contract slice;
   PR #144 is then refreshed from the materially changed `main` and revalidated.

The #143 redaction-counter repair and PR #142 remain downstream. Neither Rust
repair may be combined with a contract transition.

## Contract Scope

Ordinary #145 changes are exactly `crates/import-pipeline/src/lib.rs` and
`PROGRESS.md`. The active contract sets `contract_change_allowed=false`,
`production_semantics_change_allowed=false`, `test_only_change_required=true`,
`global_test_serialization_allowed=false`, `private_benchmark_allowed=false`,
and permits only an exact return target of #140.

The transition gate trusts the merge-base contract, not mutable head policy. It
requires an exact 15-file #140 -> #145 set and exact ten-file reverse set. A
gate run also requires the index and working tree to match and rejects untracked
files, preventing staged content from being masked locally. A same-issue Rust PR
must match this LF-normalized SHA-256 pair exactly:

- base: `7dbe7d72ed49d7062702a7d3e3d3ce98effad7fc028668870969f2f16d004609`
- fixed: `061b7cbfe557ef4cddb4065daa35b45a966ad017325be020555d413b52a49b9c`

The checker derives the fixed source from the base with three exact, single-match
test anchors, then verifies both derived bytes and digest. Those anchors add only
the release channel, sender handshake, second-poll release, and sender join.
Every other canonical source change fails closed.

## Test Design

The sender blocks on a zero-data release signal instead of elapsed wall time.
The cancellation callback increments the observed poll count and releases the
sender only when the second poll is observed. The sender then emits the existing
synthetic parse result, and the test joins it before asserting. The result cannot
arrive before two checks regardless of scheduling.

Production `recv_parse_result_with_cancel_poll`, its 50 ms interval, cancellation
semantics, import behavior, and performance thresholds remain unchanged. Sleeps,
global serialization, workflow filters, and weakened assertions are forbidden.

## Acceptance

The authorization PR requires all local contract/privacy checks and every hosted
check, including macOS and Windows Platform CI, green on its first evidence run.
If the same timing test fails, record `contract_conflict`; do not rerun unchanged.

The Rust PR requires the exact test repeated 50 times, 20 normal-parallel
import-pipeline lib suites, crate tests, fmt, clippy, rust-analyzer diagnostics,
the goal/privacy gates, and every hosted check. The reverse contract and refreshed
PR #144 have the same all-green requirement.

No step reads private data or claims mixed benchmark completion, classifier
readiness, D10K/D100K/D1M, GUI readiness, stable release, or `goal_complete`.
