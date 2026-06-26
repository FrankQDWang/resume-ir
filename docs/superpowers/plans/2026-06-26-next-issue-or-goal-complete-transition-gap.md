# Next Issue Or Goal Complete Transition Gap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore a lawful machine transition after `issue_closed_with_evidence` by adding the missing `next_issue_or_goal_complete` edge and checker coverage.

**Architecture:** Keep the fix contract-only. Update the machine transition graph in `ACTIVE_GOAL.toml`, teach `scripts/ci/check-autonomous-goal.py` to require the new transition, refresh the derived public loop-state hash, and verify with the existing contract gates.

**Tech Stack:** TOML, Python 3 contract checker, JSON state snapshot

---

### Task 1: Repair the final autonomous transition edge

**Files:**
- Modify: `ACTIVE_GOAL.toml`
- Modify: `scripts/ci/check-autonomous-goal.py`
- Modify: `perf/current-loop-state.json`
- Test: `scripts/ci/check-autonomous-goal.py`
- Test: `scripts/ci/check-loop-state.py`
- Test: `scripts/ci/check-performance-contracts.py`

- [ ] **Step 1: Reproduce the contract gap**

Run:

```bash
rg -n 'next_issue_or_goal_complete' ACTIVE_GOAL.toml perf/loop-state.schema.json
python scripts/ci/check-autonomous-goal.py
```

Expected:
- `perf/loop-state.schema.json` mentions `next_issue_or_goal_complete`
- `ACTIVE_GOAL.toml` does not define a matching transition entry
- `check-autonomous-goal.py` still passes because it does not require the missing transition yet

- [ ] **Step 2: Add the missing transition to the machine graph**

Insert a new `[[autonomous_delivery.transitions]]` block immediately after `close_issue_with_evidence` in `ACTIVE_GOAL.toml`:

```toml
[[autonomous_delivery.transitions]]
name = "advance_to_next_issue_or_goal_complete"
from = ["issue_closed_with_evidence"]
to = "next_issue_or_goal_complete"
required_permissions = []
required_evidence = ["goal_completion_assessment", "next_issue_selection_or_completion_decision"]
allowed_actions = ["assess_goal_completion", "record_next_issue"]
```

- [ ] **Step 3: Make the autonomous checker enforce the repaired graph**

Update the expected transition-name list in `scripts/ci/check-autonomous-goal.py` to require:

```python
"advance_to_next_issue_or_goal_complete",
```

Keep the validation shape unchanged: only the required transition-name set should expand.

- [ ] **Step 4: Refresh the derived public loop-state contract pin**

Update `perf/current-loop-state.json` so `contract_pins.active_goal_sha256` matches the edited `ACTIVE_GOAL.toml`.

- [ ] **Step 5: Run focused contract verification**

Run:

```bash
python scripts/ci/check-autonomous-goal.py
python scripts/ci/check-loop-state.py
python scripts/ci/check-performance-contracts.py
```

Expected: all commands exit 0.

- [ ] **Step 6: Run the public boundary gate**

Run:

```bash
./scripts/ci/guard-public-repo.sh
```

Expected: exit 0 with no raw private-data findings.

- [ ] **Step 7: Commit the contract repair**

Run:

```bash
git add ACTIVE_GOAL.toml scripts/ci/check-autonomous-goal.py perf/current-loop-state.json docs/superpowers/plans/2026-06-26-next-issue-or-goal-complete-transition-gap.md
git commit -m "contract: add next issue completion transition"
```

Expected: one focused contract-only commit on the issue branch.
