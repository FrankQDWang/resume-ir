# Autonomous Delivery 与 Issue-Led Slice Train

This document is the autonomous delivery entrypoint.
If it conflicts with older goal documents, `ACTIVE_GOAL.toml` and this document win over older prose.

## 1. Purpose

本文件定义高性能本地检索、GUI、私有 benchmark 和 Codex 闭环验证阶段的无人值守交付合同。运行中不再依赖中途人类确认；所有权限、边界、证据和停止条件必须在启动前写入机器合同。

## 2. Policy Truth

1. `ACTIVE_GOAL.toml`
2. `perf/acceptance-matrix.toml`
3. `perf/*.schema.json`
4. `03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md`
5. `03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md`
6. 其他当前目标文档
7. 历史文档

Until later autonomous-contract tasks update schemas and guards, existing `perf/loop-state.schema.json`, `perf/experiment-report.schema.json`, and `scripts/ci/check-performance-contracts.py` remain the machine-enforced public contract. This document defines the target autonomous delivery contract.

## 3. Execution Truth

1. GitHub PR/issue ledger
2. git branch/base sha
3. benchmark artifact hashes
4. `perf/current-loop-state.json`
5. current conversation context

## 4. Main State Path

```text
goal_authorized
-> baseline_captured
-> discovery_profile_issue_opened
-> hypothesis_recorded
-> slice_selected
-> branch_active
-> implementation_active
-> verification_active
-> pr_opened
-> base_synced
-> pr_review_ready
-> ci_green
-> local_gate_green
-> privacy_gate_green
-> merge_method_selected
-> pr_merged
-> issue_closed_with_evidence
-> next_issue_or_goal_complete
```

## 5. Non-Negotiable Gates

- No baseline, no optimization issue.
- No hypothesis, no implementation.
- No frozen query set, no agent replay claim.
- No main-reachable accepted evidence, no `goal_complete`.
- No admin bypass or direct push to `main`.
- No gate-changing diff in a performance optimization PR.
- No raw private data, raw query, raw trace, local path, token, diagnostic package, OCR text, or resume text in git or GitHub prose.

The local-path ban covers absolute or private local paths. It does not ban repo-relative public evidence paths or opaque hashes.

## 6. PR Budget

Default budget:

```toml
max_commits = 5
max_changed_files = 15
max_net_lines = 800
max_issue_count = 2
require_single_primary_lane = true
require_single_primary_hypothesis = true
allow_scope_exception_auto_merge = false
```

## 7. Benchmark Lanes

Required lanes:

- `first_searchable`
- `full_import_ocr_backlog`
- `query_hot_path`
- `agent_query_replay`
- `repeat_amplification_control`

## 8. Blocked Policy

The same blocker may receive at most three effective retries. Each effective retry must introduce a new `evidence_path`; re-running the same failed command is not an effective retry. Ineffective repeats are forbidden. After the same no-new-evidence blocker recurs three times, enter `blocked`.

## 9. Completion

`goal_complete` is computed by `scripts/ci/check-goal-complete.py`; markdown prose cannot claim completion.
