# Redaction Counter Test Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Authorize and then fix #143 so parallel `index-fulltext` tests cannot observe another test's redaction-regex counter work, without changing production behavior or weakening CI.

**Architecture:** Land a contract-only #140 -> #143 PR first. The target contract allows ordinary changes only to `crates/index-fulltext/src/lib.rs` and `PROGRESS.md`; a merge-base-aware gate authorizes the one-time 15-file contract transition. After that contract is main-reachable, use a separate Rust PR with thread-local test observation, then a separate #143 -> #140 contract reconciliation before refreshing PR #142.

**Tech Stack:** Rust 2021, `thread_local!`, `Cell`, TOML, JSON, Python 3, GitHub Actions

## Global Constraints

- PR #142 stays unchanged and unmerged until #143 is main-reachable and all hosted checks are green.
- Contract PR: no Rust/Cargo/workflow/acceptance-matrix/doc-14 changes.
- Rust PR: only `crates/index-fulltext/src/lib.rs` and `PROGRESS.md`.
- No production redaction semantics, global test serialization, gate/threshold weakening, private-data reads, or completion claims.
- Run `./scripts/ci/guard-public-repo.sh` before every public push.

---

### Task 1: Land the #143 contract authorization

**Files:**
- Create: `docs/superpowers/specs/2026-07-10-redaction-counter-test-isolation.md`
- Create: `docs/superpowers/plans/2026-07-10-redaction-counter-test-isolation.md`
- Modify: `ACTIVE_GOAL.toml`, `MANIFEST.md`, `PROGRESS.md`
- Modify: `scripts/ci/check-autonomous-goal.py`, `scripts/ci/check-loop-state.py`, `scripts/ci/check-gate-integrity.py`
- Modify: `perf/current-loop-state.json`
- Modify: `perf/fixtures/valid/synthetic-smoke-baseline-report.json`
- Modify: `perf/fixtures/valid/synthetic-smoke-artifact-manifest.json`
- Modify: `03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md`

**Interfaces:**
- Consumes: main `4afd254d9b7989108d726a737a3cc939c9f45deb`, #140 terminal evidence, issue #143, open PR #142.
- Produces: a reviewed, main-reachable #143 test-only active contract.

- [ ] **Step 1: Write checker RED expectations**

Make `check-autonomous-goal.py` require exact #143 identity/linked files; booleans
`contract_change=false`, `production_code=true`, `production_semantics=false`,
`test_only=true`, `global_serialization=false`, `private_benchmark=false`; exact
paths `[lib.rs, PROGRESS.md]`; and exact transition target `[#140]`.

Require a `reconcile_contract_conflict` transition from `contract_conflict` to `slice_selected`, with evidence `contract_conflict_evidence`, `linked_issue`, `reviewed_spec`, `reviewed_plan`, `privacy_boundary`, no permissions, and actions `edit_contract`, `update_issue`, `select_slice`.

Run `python3 scripts/ci/check-autonomous-goal.py` and expect failure because the repository still selects #140.

- [ ] **Step 2: Select #143 in policy truth**

Set `[authority].spec` and `.plan` to the new files. Replace `[scope.active_slice]` with:

```toml
issue = "#143"
name = "remove_parallel_redaction_counter_test_race"
linked_spec = "docs/superpowers/specs/2026-07-10-redaction-counter-test-isolation.md"
linked_plan = "docs/superpowers/plans/2026-07-10-redaction-counter-test-isolation.md"
contract_change_allowed = false
production_code_allowed = true
production_semantics_change_allowed = false
test_only_change_required = true
global_test_serialization_allowed = false
private_benchmark_allowed = false
scope_exception = false
scope_exception_reason = "Issue #143 is a bounded test-only repair that restores truthful parallel CI for PR #142; it forbids production semantics, global serialization, gate weakening, private data, mixed-import behavior, and completion claims."
allowed_contract_transition_targets = ["#140"]
allowed_paths = ["crates/index-fulltext/src/lib.rs", "PROGRESS.md"]
```

Add the transition from Step 1. Add doc 18 plus the new spec/plan to `MANIFEST.md`.

- [ ] **Step 3: Enforce transition-aware diff scope**

In `check-gate-integrity.py`, load merge-base policy with:

```python
merge_base = git(["merge-base", select_base_ref(), "HEAD"])
base_goal = tomllib.loads(git(["show", f"{merge_base}:ACTIVE_GOAL.toml"]))
head_goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
base_slice = base_goal["scope"]["active_slice"]
head_slice = head_goal["scope"]["active_slice"]
paths = changed_paths_from_commit_index_worktree_and_untracked(merge_base)
```

Define exact forward and reverse contract file sets. Enforce:

```python
if base_slice["issue"] == head_slice["issue"]:
    if not paths.issubset(set(base_slice["allowed_paths"])):
        fail("same-issue diff exceeds active-slice allowed_paths")
    if any(is_gate_path(path) for path in paths):
        require_bool(base_slice.get("contract_change_allowed"), True, "base scope.active_slice.contract_change_allowed")
    if head_slice["issue"] == "#143":
        require_exact_index_fulltext_fix(merge_base)
elif (base_slice["issue"], head_slice["issue"]) == ("#140", "#143"):
    require_bool(base_slice.get("contract_change_allowed"), True, "base scope.active_slice.contract_change_allowed")
    if paths != FORWARD_CONTRACT_PATHS:
        fail("#140 -> #143 contract transition path mismatch")
elif (base_slice["issue"], head_slice["issue"]) == ("#143", "#140"):
    if "#140" not in base_slice.get("allowed_contract_transition_targets", []):
        fail("#143 contract does not authorize return to #140")
    if paths != REVERSE_CONTRACT_PATHS:
        fail("#143 -> #140 contract transition path mismatch")
else:
    fail("unauthorized active-slice transition")
```

`FORWARD_CONTRACT_PATHS` is exactly the 15 Task-1 files. It excludes every Rust, Cargo, workflow, acceptance-matrix, doc-14, and performance-checker path. `REVERSE_CONTRACT_PATHS` is the ten Task-3 files.

`changed_paths_from_commit_index_worktree_and_untracked` unions
`merge-base...HEAD`, staged, unstaged, and untracked names. For same-issue #143,
`require_exact_index_fulltext_fix` requires the merge-base source SHA-256
`2cb94f...afe9` and the approved fixed source SHA-256 `24a94c...f3fb`. The fixed
digest pins the exact `Cell` helpers, six helper call sites, and worker-thread
regression from Task 2; every other LF-normalized source change, including
production edits or global serialization, fails. A synthetic invocation must prove the approved
digest passes and a one-byte variation fails.

- [ ] **Step 4: Repair derived state and prose**

Update docs 10/13/17/18: #140 remains incomplete; #143 is temporary and must merge before #140 resumes. Add generic loop checks requiring active-goal issue == ledger primary issue and `current_slice` to begin with that issue.

Set current loop to `slice_selected`, `contract_locked`, `w0_docs`, current slice `#143 remove parallel redaction-counter test race`, primary issue #143, active PRs exactly `[#142]`, blockers #37/#140/#143, claim `partial`, and append the #140 terminal comment as transition evidence. Never add the authorization PR itself to the snapshot. Append S695 to `PROGRESS.md`; do not claim the race fixed.

- [ ] **Step 5: Refresh pins and verify**

Compute SHA-256 for `ACTIVE_GOAL.toml`, matrix, and three schemas; update current loop and both synthetic fixtures. Recompute the synthetic report hash and byte size in its manifest. Keep `git_head_sha=4afd254d9b7989108d726a737a3cc939c9f45deb`.

Run:

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

Expected: all pass; exactly 15 files, at most 800 changed lines, and no Rust diff.
Run `./scripts/ci/verify-local.sh` once as a negative control. Before #143 is
fixed, record—but do not no-change rerun—the sole accepted failure in one of the
three shared redaction-counter assertions. Any other failure closes the contract
PR build gate. Every hosted check must still be green before merge.

- [ ] **Step 6: Deliver the authorization PR**

Commit `Authorize isolated redaction counter test repair`, then rerun
`check-gate-integrity.py`, `check-pr-budget.py`, the public guard, and the
`merge-base...HEAD` no-Rust/no-Cargo/no-workflow assertion. Push
`codex/redaction-test-fix-contract` and open a ready PR linked to #143/#140; do
not add that PR to loop state. Merge only when all hosted checks, including
Windows and macOS Platform CI, are green.

---

### Task 2: Fix the counter after Task 1 reaches main

**Files:**
- Modify: `crates/index-fulltext/src/lib.rs`, `PROGRESS.md`

**Interfaces:**
- Consumes: main-reachable #143 contract.
- Produces: test-only per-thread counter observation; production recorder remains a no-op outside tests.

- [ ] **Step 1: Write deterministic RED**

Wrap the current atomic in `reset_redaction_regex_passes()` and `redaction_regex_passes()`. Change all three zero-count tests to the helpers. Add a test that resets the parent counter, spawns a thread which resets/increments/asserts one, joins it, and expects the parent still sees zero. Run its exact name; expect `left: 1, right: 0`.

- [ ] **Step 2: Implement thread-local GREEN**

```rust
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
std::thread_local! {
    static REDACTION_REGEX_PASSES: Cell<usize> = const { Cell::new(0) };
}
#[cfg(test)]
fn record_redaction_regex_pass() {
    REDACTION_REGEX_PASSES.with(|passes| passes.set(passes.get() + 1));
}
#[cfg(test)]
fn reset_redaction_regex_passes() {
    REDACTION_REGEX_PASSES.with(|passes| passes.set(0));
}
#[cfg(test)]
fn redaction_regex_passes() -> usize {
    REDACTION_REGEX_PASSES.with(Cell::get)
}
```

Keep `#[cfg(not(test))] fn record_redaction_regex_pass() {}` unchanged.

- [ ] **Step 3: Verify and deliver**

Run the deterministic test and all three existing zero-count tests by exact name; then:

```bash
for run in $(seq 1 20); do cargo test -p index-fulltext --lib --locked || exit 1; done
cargo test -p index-fulltext --locked
cargo fmt --all -- --check
cargo clippy -p index-fulltext --all-targets --all-features --locked -- -D warnings
rust-analyzer diagnostics .
python3 scripts/ci/check-performance-contracts.py
python3 scripts/ci/check-autonomous-goal.py
python3 scripts/ci/check-loop-state.py
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
git diff --check
```

Append passing evidence to S695, commit only the two allowed files on a fresh branch, and merge its #143 PR only after every hosted check is green.

---

### Task 3: Restore #140, then refresh PR #142

**Files:**
- Modify: `ACTIVE_GOAL.toml`, `PROGRESS.md`, `scripts/ci/check-autonomous-goal.py`,
  `perf/current-loop-state.json`, both smoke fixtures, and goal docs 10/13/17/18.

- [ ] **Step 1: Land the explicit #143 -> #140 contract PR**

From fresh post-fix main, restore #140 identity, authority, flags, and paths;
delete #143-only fields; update checker/docs/loop/pins/fixtures/S695. Keep generic
guards. The exact ten-path reverse whitelist is `ACTIVE_GOAL`, `PROGRESS`,
autonomous checker, loop, two smoke fixtures, and docs 10/13/17/18. Prove an
extra path fails, then require all Task-1 and hosted gates before squash merge.

- [ ] **Step 2: Refresh and merge PR #142**

Sync `codex/mixed-import-evidence-contract` to the restored #140 main, resolve pins/state in favor of live #140 truth, rerun all local and hosted checks, and merge only with Windows/macOS green. Record `same_lane_continues`: report schema/checker and frozen public corpus remain unfinished.
