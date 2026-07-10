# Parse-Result Cancel-Poll Test Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task.

**Goal:** Unblock #144 by making the parse-result cancellation-poll test
scheduler-independent without changing production behavior or weakening CI.

**Architecture:** Land a contract-only #140 -> #145 PR, then an exact test-only
Rust PR, then a #145 -> #140 restoration PR. Refresh frozen PR #144 only after
the fix is main-reachable; resume #143/#142 afterward.

**Tech Stack:** Rust 2021, `std::sync::mpsc`, TOML, JSON, Python 3, GitHub Actions

## Global Constraints

- Freeze PR #144 at `b093b1a`; no unchanged rerun, bypass, or scope mutation.
- Contract PR: no Rust/Cargo/workflow/acceptance-matrix/doc-14 change.
- Rust PR: only `crates/import-pipeline/src/lib.rs` and `PROGRESS.md`.
- No production semantics, sleeps, global serialization, threshold/gate change,
  private-data read, performance claim, or goal-completion claim.
- Run `./scripts/ci/guard-public-repo.sh` before every public push.

---

### Task 1: Land the #145 contract authorization

**Files:**
- Create: `docs/superpowers/specs/2026-07-10-parse-result-cancel-poll-test-isolation.md`
- Create: `docs/superpowers/plans/2026-07-10-parse-result-cancel-poll-test-isolation.md`
- Modify: `ACTIVE_GOAL.toml`, `MANIFEST.md`, `PROGRESS.md`
- Modify: `scripts/ci/check-autonomous-goal.py`, `scripts/ci/check-loop-state.py`, `scripts/ci/check-gate-integrity.py`
- Modify: `perf/current-loop-state.json`
- Modify: `perf/fixtures/valid/synthetic-smoke-baseline-report.json`
- Modify: `perf/fixtures/valid/synthetic-smoke-artifact-manifest.json`
- Modify: goal docs 10, 13, 17, and 18

**Consumes:** main `4afd254d9b7989108d726a737a3cc939c9f45deb`, issue #145,
PR #144 terminal evidence, open PR #142, and open issues #140/#143.

- [ ] **Step 1: Write contract RED expectations**

Make `check-autonomous-goal.py` require issue #145, exact linked spec/plan,
`contract_change=false`, `production_code=true`, `production_semantics=false`,
`test_only=true`, `global_serialization=false`, `private_benchmark=false`, exact
paths `[crates/import-pipeline/src/lib.rs, PROGRESS.md]`, and return target #140.
Require the reviewed `reconcile_contract_conflict` transition with no permission.
Run the checker; expect stale #140 failure before changing policy truth.

- [ ] **Step 2: Select #145 in policy truth**

Point `[authority]` at the new files. Set active issue/name to #145 /
`make_parse_result_cancel_poll_test_scheduler_independent`, the booleans and paths
from Step 1, `scope_exception=false`, and an explicit reason forbidding production
semantics, global serialization, gate weakening, private data, or completion.
Add the spec/plan to `MANIFEST.md`.

- [ ] **Step 3: Enforce transition and exact Rust scope**

In `check-gate-integrity.py`, collect merge-base committed, staged, unstaged, and
untracked paths, but reject a gate run unless index/worktree content matches and
no untracked file remains. Same-issue scope comes from merge-base `allowed_paths`;
any gate change also requires merge-base `contract_change_allowed=true`.

Require exact #140 -> #145 and #145 -> #140 path sets. For same-issue #145,
require `crates/import-pipeline/src/lib.rs` to move from SHA-256
`7dbe7d...4609` to `061b7c...b9c`. The checker itself must derive the fixed bytes
from three exact single-match test anchors; a synthetic invocation must match
the fixed digest and reject a one-byte variation.

- [ ] **Step 4: Reconcile loop state and prose**

Docs 10/13/17/18 must say #145 temporarily blocks #144; after #145 fix and #140
restoration, refresh #144 before #143/#142. Set loop state to `slice_selected`,
`contract_locked`, `w0_docs`, current slice #145, primary issue #145, active PRs
exactly `[#142, #144]`, and blockers `#37/#140/#143/#144/#145`. Record PR #144
terminal evidence and append S696 to `PROGRESS.md`. Do not add the new
authorization PR itself to the snapshot or claim the timing test fixed.

- [ ] **Step 5: Refresh pins and verify**

Refresh `ACTIVE_GOAL.toml`, matrix, and schema hashes; refresh the paired
synthetic-smoke report/manifest digest and byte size. Keep
`git_head_sha=4afd254d9b7989108d726a737a3cc939c9f45deb`.

Stage the exact 15 files so index and working tree match, then run:

```bash
python3 scripts/ci/check-performance-contracts.py
python3 scripts/ci/check-autonomous-goal.py
python3 scripts/ci/check-loop-state.py
python3 scripts/ci/check-pr-budget.py
python3 scripts/ci/check-gate-integrity.py
python3 scripts/ci/check-private-evidence-redaction.py
python3 -m py_compile scripts/ci/check-autonomous-goal.py scripts/ci/check-loop-state.py scripts/ci/check-gate-integrity.py
./scripts/ci/check-workflows.sh
./scripts/ci/guard-public-repo.sh
git diff --check
git diff --quiet -- crates/ Cargo.toml Cargo.lock .github/workflows/ perf/acceptance-matrix.toml 03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md scripts/ci/check-performance-contracts.py
```

Expected: all pass; exactly 15 files, at most 800 changed lines, no prohibited
diff. Every hosted check must be green. If the known test fails, record the
machine terminal and do not rerun unchanged.

- [ ] **Step 6: Deliver authorization PR**

Commit `Authorize deterministic parse-result cancel-poll test repair`; rerun
post-commit gate-integrity, PR-budget, public guard, exact path/count checks, and
the prohibited merge-base diff assertion. Push a ready PR linked to #145/#140.
Merge only with all hosted checks and reviews green.

---

### Task 2: Make the existing test deterministic after Task 1 reaches main

**Files:** `crates/import-pipeline/src/lib.rs`, `PROGRESS.md`

- [ ] **Step 1: Preserve RED evidence**

Bind to macOS run `29068476119`, job `86284945029`, and identical base/head blob
`ebe399c1f99cdf09ba80b696519ced69053e604a`. Do not manufacture a timing failure
or rerun PR #144. Confirm code truth: 50 ms receive timeout, 180 ms unsynchronized
sender sleep, and `>= 2` assertion.

- [ ] **Step 2: Implement exact GREEN handshake**

Add a release channel beside the result channel. The sender blocks on
`release_rx.recv()` instead of sleeping. The cancellation callback increments
the poll count and sends the release signal exactly when the second poll is
observed. Join the sender after receiving the result. Do not change production
functions, intervals, or assertions.

- [ ] **Step 3: Verify and deliver**

Run:

```bash
for run in $(seq 1 50); do cargo test -p import-pipeline recv_parse_result_polls_cancel_while_waiting --locked -- --exact || exit 1; done
for run in $(seq 1 20); do cargo test -p import-pipeline --lib --locked || exit 1; done
cargo test -p import-pipeline --locked
cargo fmt --all -- --check
cargo clippy -p import-pipeline --all-targets --all-features --locked -- -D warnings
rust-analyzer diagnostics .
python3 scripts/ci/check-performance-contracts.py
python3 scripts/ci/check-autonomous-goal.py
python3 scripts/ci/check-loop-state.py
python3 scripts/ci/check-gate-integrity.py
python3 scripts/ci/check-private-evidence-redaction.py
./scripts/ci/guard-public-repo.sh
```

Require exact approved source digest, clean public boundary, independent review,
and every hosted check green. Comment #145 and update `PROGRESS.md`; no
performance or product-completion claim.

---

### Task 3: Restore #140 and resume the blocked chain

- [ ] **Step 1:** In a separate ten-file contract PR, restore active issue #140,
  its authority/spec/plan/checker/state/pins/docs, record #145 completion, and
  remove #145 from open blockers. No Rust or workflow change.
- [ ] **Step 2:** Merge only when every hosted check is green.
- [ ] **Step 3:** Refresh PR #144 from materially changed `main`; re-run scope,
  pin, budget, privacy, review, and hosted gates. If it no longer fits the exact
  reviewed transition, close it and create a fresh #143 authorization PR.
- [ ] **Step 4:** Continue the existing separate sequence: #143 authorization,
  redaction-counter fix, #140 restoration, then refresh PR #142.
