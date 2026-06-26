# Autonomous Delivery Issue-Led Slice Train Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the approved autonomous delivery spec into repo documents, machine-readable contracts, GitHub templates, and CI guards that can drive a no-mid-run-human long Codex goal.

**Architecture:** Keep policy in `ACTIVE_GOAL.toml` and target docs, state in `perf/current-loop-state.json`, structure in JSON schemas, execution evidence in GitHub issue/PR templates, and enforcement in focused `scripts/ci/check-*.py` guards. Preserve `scripts/ci/check-performance-contracts.py` as the aggregate public gate while splitting guard logic into focused modules.

**Tech Stack:** Markdown, TOML parsed by Python `tomllib`, JSON Schema draft 2020-12 structure, standard-library Python CI guards, GitHub issue/PR templates, existing shell privacy guard.

---

## Scope Check

This plan implements the autonomous delivery contract layer only. It does not start search performance optimization, GUI implementation, daemon IPC changes, private benchmark execution, or frozen query-set generation.

The spec spans several surfaces, but they are one cohesive contract slice: docs define intent, TOML/schema define allowed state, GitHub templates record ledger evidence, and CI guards reject invalid states. Implement these in the order below so each commit leaves the public contract stronger and verifiable.

## File Structure

Create:

- `03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md` - autonomous delivery entrypoint and truth-priority contract.
- `.github/ISSUE_TEMPLATE/profile_issue.md` - profile ledger issue template with baseline, hypothesis, and closure anchors.
- `.github/ISSUE_TEMPLATE/gui_manual_loop.md` - GUI/manual loop evidence issue template.
- `.github/ISSUE_TEMPLATE/benchmark_infra.md` - benchmark harness and evidence-infra issue template.
- `.github/ISSUE_TEMPLATE/contract_change.md` - isolated gate, schema, threshold, workflow, or guard change issue template.
- `.github/ISSUE_TEMPLATE/privacy_boundary.md` - privacy/redaction boundary issue template.
- `scripts/ci/check-autonomous-goal.py` - validate `ACTIVE_GOAL.toml` autonomous delivery fields.
- `scripts/ci/check-loop-state.py` - validate loop-state schema/current-state invariants not covered by schema shape alone.
- `scripts/ci/check-experiment-report.py` - validate experiment report fixture claim boundaries.
- `scripts/ci/check-pr-budget.py` - validate PR template anchors and local PR budget config where available.
- `scripts/ci/check-benchmark-lanes.py` - validate benchmark lane declarations and allowed claims.
- `scripts/ci/check-private-evidence-redaction.py` - validate public evidence files contain only redacted aggregate fields.
- `scripts/ci/check-gate-integrity.py` - detect gate-changing diffs and require isolated contract-change context.
- `scripts/ci/check-goal-complete.py` - compute whether `goal_complete` is allowed.

Modify:

- `ACTIVE_GOAL.toml` - add autonomous delivery permissions, budget, merge policy, private sources, GitHub ledger, contract integrity, benchmark lanes, and blocked policy.
- `perf/acceptance-matrix.toml` - add five benchmark lanes and autonomous delivery redlines.
- `perf/loop-state.schema.json` - add autonomous workflow states, integrity fields, retry attempts, evidence-cell metadata, and main-reachable evidence rules.
- `perf/experiment-report.schema.json` - add lane-specific evidence fields and claim boundaries for first-searchable, OCR backlog, hot path, agent replay, repeat amplification, and goal completion.
- `perf/current-loop-state.json` - update current public state to the autonomous contract-planning slice without claiming implementation completion.
- `scripts/ci/check-performance-contracts.py` - keep as aggregate entrypoint and call the focused guards.
- `.github/PULL_REQUEST_TEMPLATE.md` - add contract anchors, performance evidence, privacy boundary, merge readiness, and gate-change section.
- `.github/workflows/pr.yml` - add focused contract guard steps with stable names for branch protection.
- `03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md` - make document 18 the autonomous entrypoint.
- `03_next_goal_高性能本地检索GUI闭环/05_Query_Benchmark与真实Query种子.md` - lock agent replay to static `source_search` query extraction from `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log`.
- `03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md` - add read/summarize/hash/commit/GitHub boundaries for private resumes and SeekTalent artifacts.
- `03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md` - require baseline plus hypothesis before performance implementation slices.
- `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md` - replace human-gated delivery with autonomous states, reconciliation, authority/integrity validation, and retry policy.
- `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md` - add five benchmark lanes and completion evidence requirements.
- `03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md` - make GitHub profile issues the profiling ledger.
- `03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md` - define relationships among policy truth, execution truth, schemas, templates, and guards.

Do not modify:

- Production Rust code under `crates/`.
- Private resume data.
- Raw SeekTalent artifacts.
- Raw benchmark outputs.
- Tokens, diagnostics, model caches, local runtime logs, or local paths beyond home-relative source labels in docs.

### Task 1: Add Autonomous Entrypoint Document

**Files:**
- Create: `03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md`
- Reference: `docs/superpowers/specs/2026-06-25-autonomous-delivery-issue-led-slice-train.md`

- [ ] **Step 1: Create the entrypoint document**

Create `03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md` with this top-level structure:

````markdown
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

The same blocker may receive at most three effective retries. Each effective retry must introduce a new `evidence_path`; re-running the same failed command is not an effective retry.

## 9. Completion

`goal_complete` is computed by `scripts/ci/check-goal-complete.py`; markdown prose cannot claim completion.
````

- [ ] **Step 2: Update reading order**

Modify `03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md` so any future implementation run reads document 18 first after `ACTIVE_GOAL.toml`. Add this sentence near the top:

```markdown
无人值守执行阶段先读 `ACTIVE_GOAL.toml` 和 `18_Autonomous_Delivery与Issue_Led_Slice_Train.md`；若旧目标文档与 autonomous delivery 合同冲突，以 `ACTIVE_GOAL.toml`、`perf/acceptance-matrix.toml`、schema 和 18 号入口文档为准。
```

- [ ] **Step 3: Verify no raw private path**

Run:

```bash
rg -n "/Users/|raw query|trace.log" 03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md
```

Expected: no `/Users/` matches. `trace.log` may appear only as `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log`.

- [ ] **Step 4: Commit**

Run:

```bash
git add 03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md 03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md
git commit -m "docs: add autonomous delivery entrypoint"
```

### Task 2: Update Goal Authority And Benchmark Documents

**Files:**
- Modify: `03_next_goal_高性能本地检索GUI闭环/05_Query_Benchmark与真实Query种子.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md`

- [ ] **Step 1: Update query benchmark source contract**

In `05_Query_Benchmark与真实Query种子.md`, add an `Agent Query Replay` section with this contract:

````markdown
## Agent Query Replay 静态基准

`agent_query_replay` 只使用 SeekTalent 真实运行中已经产生的 source search 查询，不从 JD、prompt、候选人资料或 trace 上下文构造 query。

Allowed source:

```text
source_root = $RESUME_IR_QUERY_ARTIFACT_ROOT
source_glob = **/runtime/trace.log
event_filter = tool_called
tool_filter = source_search
query_source = source_search invocation argument only
query_extraction_version = trace_source_search_v1
```

`$RESUME_IR_QUERY_ARTIFACT_ROOT` is a private local environment variable that resolves to the SeekTalent run artifact root. Public contracts and evidence must keep the symbolic variable name and must not write the resolved local path.

Forbidden sources:

```text
artifacts/benchmarks/*.jsonl job_description or hiring_notes
raw transcript
full prompt
candidate profile
resume text
file path
URL
provider payload
token
raw log line outside the source_search invocation
debug blob
screenshot OCR
```

Query set 必须先从真实 `source_search` 调用中抽取候选，再筛选一组在 D10K 私有库上可用于稳定比较的固定集合。少量 zero-result query 可以保留为单独 bucket；benchmark 不能被大量搜不到人的 query 主导。冻结后以 `query_set_sha256` 锁定。修改 extraction/redaction 规则必须生成新的 `query_set_sha256`，旧结果不得直接做 before/after 对比。
````

- [ ] **Step 2: Update privacy boundary**

In `09_安全隐私与本地证据边界.md`, add this table:

```markdown
| Material | Local read | Summarize | Hash | Commit | GitHub issue/PR |
|---|---:|---:|---:|---:|---:|
| `$RESUME_IR_PRIVATE_RESUME_ROOT` raw files | yes | redacted aggregate only | yes | no | no |
| OCR raw text | local only | counts/errors only | yes | no | no |
| `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log` | yes, query extraction only | query-set aggregate only | yes | no | no |
| frozen raw query set | local only | hash and bucket counts only | yes | no | no |
| synthetic fixtures | yes | yes | yes | yes | yes |
| percentiles/stage latency/counts | yes | yes | yes | yes | yes |
```

Add this rule below the table:

```markdown
GitHub issue/comment 只能写 redacted evidence。issue 是执行账本，不是私有数据存储位置。
```

- [ ] **Step 3: Update implementation gate**

In `10_实施切片与验收门槛.md`, add:

```markdown
No performance implementation slice may start before baseline and hypothesis are recorded in the linked GitHub profile issue. If no baseline exists, the only allowed work is instrumentation, benchmark harness, or baseline capture.
```

- [ ] **Step 4: Update loop state machine prose**

In `13_Loop_Engineering状态机.md`, add the autonomous loop sequence and fact priorities from Task 1. Also add:

```markdown
`base_drift` 是 reconciliation action，不消耗普通 retry。runner 先同步或 rebase 最新 `main` 并重跑 affected gates；只有相同失败仍复现时才开始计入 retry。
```

- [ ] **Step 5: Update benchmark lanes and profiling ledger**

In `14_W0_W1验收矩阵与证据协议.md`, add the five required benchmark lanes and state that lanes cannot claim each other's results.

In `15_性能观测与Profiling工具链.md`, add:

```markdown
profile issue is the profiling ledger
negative experiment is valid evidence
benchmark regression is experiment_negative unless it also violates a release gate
no profile, no optimization
```

- [ ] **Step 6: Update machine protocol relationship**

In `17_机器可读Goal与Experiment协议.md`, add:

```markdown
Policy truth lives in `ACTIVE_GOAL.toml`, `perf/acceptance-matrix.toml`, schemas, and the autonomous entrypoint document. Execution truth lives in GitHub PR/issue state, git branch/base sha, benchmark artifact hashes, and only then `perf/current-loop-state.json`.
```

- [ ] **Step 7: Verify docs**

Run:

```bash
git diff --check -- 03_next_goal_高性能本地检索GUI闭环
rg -n "/Users/|raw candidate|provider payload" 03_next_goal_高性能本地检索GUI闭环
```

Expected: diff check passes. No `/Users/` match. `provider payload` may appear only in forbidden-source lists.

- [ ] **Step 8: Commit**

Run:

```bash
git add 03_next_goal_高性能本地检索GUI闭环
git commit -m "docs: define autonomous benchmark and loop contracts"
```

### Task 3: Extend Machine-Readable Goal And Acceptance Matrix

**Files:**
- Modify: `ACTIVE_GOAL.toml`
- Modify: `perf/acceptance-matrix.toml`

- [ ] **Step 1: Add autonomous delivery sections to `ACTIVE_GOAL.toml`**

Append these sections:

```toml
[autonomous_delivery.permissions]
production_code_allowed = true
private_benchmark_allowed = true
private_resume_root_read_allowed = true
seektalent_artifacts_query_read_allowed = true
github_issue_write_allowed = true
github_pr_write_allowed = true
commit_push_allowed = true
auto_merge_allowed = true
direct_main_push_allowed = false
admin_bypass_allowed = false
raw_private_data_commit_allowed = false
raw_query_commit_allowed = false
gate_bypass_allowed = false
threshold_relaxation_allowed = false

[autonomous_delivery.pr_budget]
max_commits = 5
max_changed_files = 15
max_net_lines = 800
max_issue_count = 2
require_single_primary_lane = true
require_single_primary_hypothesis = true
allow_scope_exception_auto_merge = false

[autonomous_delivery.merge_policy]
default_merge_method = "squash"
require_base_synced = true
require_merge_method_selected = true
require_no_admin_bypass = true
require_no_direct_main_push = true

[autonomous_delivery.private_sources]
private_resume_root = "$RESUME_IR_PRIVATE_RESUME_ROOT"
seektalent_runs_root = "$RESUME_IR_QUERY_ARTIFACT_ROOT"
seektalent_trace_glob = "**/runtime/trace.log"
private_resume_root_env_var = "RESUME_IR_PRIVATE_RESUME_ROOT"
seektalent_runs_root_env_var = "RESUME_IR_QUERY_ARTIFACT_ROOT"
allowed_trace_event = "tool_called"
allowed_trace_tool = "source_search"

[autonomous_delivery.github_ledger]
profile_issue_template = ".github/ISSUE_TEMPLATE/profile_issue.md"
pr_template = ".github/PULL_REQUEST_TEMPLATE.md"
require_issue_closure_comment = true
require_machine_anchors = true

[autonomous_delivery.contract_integrity]
goal_version = "2026-06-25-autonomous-c"
contract_schema_version = "v1"
require_goal_contract_hash = true
require_acceptance_matrix_hash = true
require_runner_version = true
require_benchmark_runner_version = true
require_main_reachable_evidence = true

[autonomous_delivery.benchmark_lanes]
required = [
  "first_searchable",
  "full_import_ocr_backlog",
  "query_hot_path",
  "agent_query_replay",
  "repeat_amplification_control",
]
forbid_lane_claim_mixing = true

[autonomous_delivery.blocked_policy]
max_same_condition_effective_retries = 3
require_new_evidence_path_per_retry = true
```

- [ ] **Step 2: Add benchmark lane declarations to `perf/acceptance-matrix.toml`**

Append:

```toml
[autonomous_delivery_lanes.first_searchable]
may_claim = ["first_searchable_latency", "initial_searchable_count"]
cannot_claim = ["ocr_completion_performance", "goal_complete"]

[autonomous_delivery_lanes.full_import_ocr_backlog]
may_claim = ["ocr_queue_throughput", "ocr_failure_recovery", "background_scheduling"]
cannot_claim = ["query_p95_success", "goal_complete"]

[autonomous_delivery_lanes.query_hot_path]
may_claim = ["hot_index_latency", "stage_latency", "resident_daemon_performance"]
cannot_claim = ["ocr_completion_performance", "human_search_quality"]

[autonomous_delivery_lanes.agent_query_replay]
may_claim = [
  "real_agent_workload_latency",
  "real_agent_query_parser_compatibility",
  "intersection_query_behavior",
  "zero_result_regression",
  "duplicate_result_regression",
  "top_k_stability_under_agent_queries",
]
cannot_claim = [
  "human_search_quality",
  "full_resume_relevance_quality",
  "D1M_real_distribution_quality",
  "OCR_completion_performance",
]

[autonomous_delivery_lanes.repeat_amplification_control]
may_claim = ["pathology_only"]
cannot_claim = ["real_distribution_quality", "goal_complete"]
```

- [ ] **Step 3: Run current aggregate validator**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
```

Expected: PASS. If it fails because the current validator rejects additional TOML sections, update Task 6 guard code to allow and validate them before committing this task.

- [ ] **Step 4: Commit**

Run:

```bash
git add ACTIVE_GOAL.toml perf/acceptance-matrix.toml
git commit -m "docs: add autonomous delivery machine contract"
```

### Task 4: Update Loop And Experiment Schemas

**Files:**
- Modify: `perf/loop-state.schema.json`
- Modify: `perf/experiment-report.schema.json`
- Modify: `perf/current-loop-state.json`
- Modify or add: `perf/fixtures/valid/*.json`
- Modify or add: `perf/fixtures/invalid/*.json`

- [ ] **Step 1: Add loop states**

In `perf/loop-state.schema.json`, extend `workflow_state.enum` with:

```json
[
  "goal_authorized",
  "baseline_captured",
  "discovery_profile_issue_opened",
  "hypothesis_recorded",
  "branch_active",
  "pr_opened",
  "base_synced",
  "pr_review_ready",
  "ci_green",
  "local_gate_green",
  "privacy_gate_green",
  "merge_method_selected",
  "pr_merged",
  "issue_closed_with_evidence",
  "next_issue_or_goal_complete",
  "blocked_needs_external_input"
]
```

- [ ] **Step 2: Add required autonomous loop properties**

Add these properties to `perf/loop-state.schema.json`:

```json
"contract_integrity": { "$ref": "#/$defs/contract_integrity" },
"github_ledger": { "$ref": "#/$defs/github_ledger" },
"retry_policy": { "$ref": "#/$defs/retry_policy" },
"evidence_cells": {
  "type": "array",
  "items": { "$ref": "#/$defs/evidence_cell" },
  "uniqueItems": true
}
```

Add these definitions:

```json
"contract_integrity": {
  "type": "object",
  "additionalProperties": false,
  "required": [
    "goal_contract_hash",
    "acceptance_matrix_hash",
    "runner_version",
    "benchmark_runner_version"
  ],
  "properties": {
    "goal_contract_hash": { "$ref": "#/$defs/hex64" },
    "acceptance_matrix_hash": { "$ref": "#/$defs/hex64" },
    "runner_version": { "type": "string", "minLength": 1 },
    "benchmark_runner_version": { "type": "string", "minLength": 1 }
  }
},
"github_ledger": {
  "type": "object",
  "additionalProperties": false,
  "required": ["primary_issue", "active_prs", "open_blockers"],
  "properties": {
    "primary_issue": { "type": "string", "pattern": "^#[0-9]+$" },
    "active_prs": {
      "type": "array",
      "items": { "type": "string", "pattern": "^#[0-9]+$" }
    },
    "open_blockers": {
      "type": "array",
      "items": { "type": "string", "pattern": "^#[0-9]+$" }
    }
  }
},
"retry_policy": {
  "type": "object",
  "additionalProperties": false,
  "required": ["blocker_key", "same_condition_count", "attempts"],
  "properties": {
    "blocker_key": { "type": "string", "minLength": 1 },
    "same_condition_count": { "type": "integer", "minimum": 0, "maximum": 3 },
    "attempts": {
      "type": "array",
      "items": { "$ref": "#/$defs/retry_attempt" }
    }
  }
},
"retry_attempt": {
  "type": "object",
  "additionalProperties": false,
  "required": ["attempt_id", "evidence_path", "action", "result"],
  "properties": {
    "attempt_id": { "type": "string", "minLength": 1 },
    "evidence_path": { "type": "string", "minLength": 1 },
    "action": { "type": "string", "minLength": 1 },
    "result": { "enum": ["passed", "failed", "blocked"] }
  }
},
"evidence_cell": {
  "type": "object",
  "additionalProperties": false,
  "required": [
    "cell",
    "goal_contract_hash",
    "acceptance_matrix_hash",
    "runner_version",
    "benchmark_runner_version",
    "main_reachable_commit"
  ],
  "properties": {
    "cell": {
      "enum": [
        "D10K_private_real",
        "D100K_synthetic_scale",
        "D1M_synthetic_scale",
        "first_searchable",
        "full_import_ocr_backlog",
        "query_hot_path",
        "agent_query_replay",
        "repeat_amplification_control",
        "soak_fault",
        "gui_manual"
      ]
    },
    "goal_contract_hash": { "$ref": "#/$defs/hex64" },
    "acceptance_matrix_hash": { "$ref": "#/$defs/hex64" },
    "runner_version": { "type": "string", "minLength": 1 },
    "benchmark_runner_version": { "type": "string", "minLength": 1 },
    "query_set_sha256": { "$ref": "#/$defs/hex64" },
    "corpus_profile_hash": { "$ref": "#/$defs/hex64" },
    "main_reachable_commit": { "$ref": "#/$defs/git_head" }
  }
}
```

- [ ] **Step 3: Add experiment report lane fields**

In `perf/experiment-report.schema.json`, add lane-specific report definitions for:

```json
"first_searchable": { "$ref": "#/$defs/first_searchable" },
"full_import_ocr_backlog": { "$ref": "#/$defs/full_import_ocr_backlog" },
"agent_query_replay": { "$ref": "#/$defs/agent_query_replay" },
"repeat_amplification_control": { "$ref": "#/$defs/repeat_amplification_control" }
```

Define `agent_query_replay` with required fields:

```json
"agent_query_replay": {
  "type": "object",
  "additionalProperties": false,
  "required": [
    "query_set_sha256",
    "query_extraction_version",
    "query_set_size",
    "zero_result_rate",
    "intersection_clause_count",
    "query_parse_error_rate",
    "allowed_claims"
  ],
  "properties": {
    "query_set_sha256": { "$ref": "#/$defs/hex64" },
    "query_extraction_version": { "const": "trace_source_search_v1" },
    "query_set_size": { "type": "integer", "minimum": 1 },
    "zero_result_rate": { "type": "number", "minimum": 0, "maximum": 1 },
    "intersection_clause_count": { "type": "integer", "minimum": 0 },
    "query_parse_error_rate": { "type": "number", "minimum": 0, "maximum": 1 },
    "allowed_claims": {
      "type": "array",
      "items": {
        "enum": [
          "real_agent_workload_latency",
          "real_agent_query_parser_compatibility",
          "intersection_query_behavior",
          "zero_result_regression",
          "duplicate_result_regression",
          "top_k_stability_under_agent_queries"
        ]
      },
      "minItems": 1,
      "uniqueItems": true
    }
  }
}
```

- [ ] **Step 4: Add positive and negative fixtures**

Add at least these fixtures:

```text
perf/fixtures/valid/autonomous-loop-slice-complete.json
perf/fixtures/valid/agent-query-replay-report.json
perf/fixtures/invalid/agent-query-replay-without-query-set-hash.json
perf/fixtures/invalid/goal-complete-with-unmerged-evidence.json
perf/fixtures/invalid/performance-implementation-without-hypothesis.json
```

Each invalid fixture must fail `scripts/ci/check-performance-contracts.py` after Task 6. Each valid fixture must pass.

- [ ] **Step 5: Update current loop state conservatively**

Update `perf/current-loop-state.json` to reference this contract landing slice without claiming production implementation:

```json
{
  "workflow_state": "plan_ready",
  "experiment_state": "contract_locked",
  "evidence_lane": "w0_docs",
  "current_slice": "autonomous delivery contract planning"
}
```

Preserve existing privacy false fields and do not add private data refs.

- [ ] **Step 6: Verify**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
python3 -m json.tool perf/loop-state.schema.json >/dev/null
python3 -m json.tool perf/experiment-report.schema.json >/dev/null
python3 -m json.tool perf/current-loop-state.json >/dev/null
```

Expected: all commands exit 0 after Task 6 guard updates are complete.

- [ ] **Step 7: Commit**

Run:

```bash
git add perf/loop-state.schema.json perf/experiment-report.schema.json perf/current-loop-state.json perf/fixtures
git commit -m "docs: extend autonomous loop evidence schemas"
```

### Task 5: Add GitHub Issue And PR Templates With Anchors

**Files:**
- Create: `.github/ISSUE_TEMPLATE/profile_issue.md`
- Create: `.github/ISSUE_TEMPLATE/gui_manual_loop.md`
- Create: `.github/ISSUE_TEMPLATE/benchmark_infra.md`
- Create: `.github/ISSUE_TEMPLATE/contract_change.md`
- Create: `.github/ISSUE_TEMPLATE/privacy_boundary.md`
- Modify: `.github/PULL_REQUEST_TEMPLATE.md`

- [ ] **Step 1: Create profile issue template**

Create `.github/ISSUE_TEMPLATE/profile_issue.md`:

```markdown
---
name: Profile issue
about: Track baseline, bottleneck, hypothesis, and closing evidence without private data
title: "profile: "
labels: performance, profile
assignees: FrankQDWang
---

<!-- contract:profile_lane -->
## Profile Lane

<!-- contract:dataset -->
## Dataset

<!-- contract:corpus_profile_hash -->
## Corpus Profile Hash

<!-- contract:query_set_sha256 -->
## Query Set SHA-256

<!-- contract:baseline_command -->
## Baseline Command

<!-- contract:baseline_evidence -->
## Baseline Evidence

<!-- contract:profiler_evidence -->
## Profiler Evidence

<!-- contract:hypothesis -->
## Hypothesis

<!-- contract:target_metric -->
## Target Metric

<!-- contract:success_threshold -->
## Success Threshold

<!-- contract:failure_regression_guard -->
## Failure / Regression Guard

<!-- contract:privacy_boundary -->
## Privacy Boundary

Do not include raw resumes, raw query text, raw trace lines, local paths, tokens, provider payloads, diagnostic archives, or OCR text.

<!-- contract:linked_prs -->
## Linked PRs

<!-- contract:closing_evidence -->
## Closing Evidence
```

- [ ] **Step 2: Create remaining issue templates**

Create `.github/ISSUE_TEMPLATE/contract_change.md`:

```markdown
---
name: Contract change
about: Isolated change to guard, workflow, schema, threshold, or goal contract
title: "contract: "
labels: contract
assignees: FrankQDWang
---

<!-- contract:scope -->
## Scope

<!-- contract:authorized_change -->
## Authorized Change

<!-- contract:affected_gates -->
## Affected Gates

<!-- contract:why_isolated -->
## Why This Is Isolated

<!-- contract:verification -->
## Verification

<!-- contract:rollback_plan -->
## Rollback Plan

<!-- contract:privacy_boundary -->
## Privacy Boundary
```

Create `.github/ISSUE_TEMPLATE/gui_manual_loop.md`, `.github/ISSUE_TEMPLATE/benchmark_infra.md`, and `.github/ISSUE_TEMPLATE/privacy_boundary.md` with the same anchor pattern: `contract:scope`, `contract:evidence`, `contract:verification`, `contract:privacy_boundary`, and `contract:rollback_plan`.

- [ ] **Step 3: Update PR template**

Replace `.github/PULL_REQUEST_TEMPLATE.md` with an anchored template containing:

```markdown
<!-- contract:scope -->
## Scope

<!-- contract:linked_issue -->
## Linked Issue

<!-- contract:hypothesis_baseline -->
## Hypothesis / Baseline

<!-- contract:changes -->
## Changes

<!-- contract:out_of_scope -->
## Out of Scope

<!-- contract:verification -->
## Verification

<!-- contract:performance_evidence -->
## Performance Evidence

<!-- contract:privacy_boundary -->
## Privacy Boundary

<!-- contract:rollback_plan -->
## Rollback Plan

<!-- contract:contract_gate_changes -->
## Contract / Gate Changes

- [ ] No gate/guard/threshold changes
- [ ] Gate changes are isolated in this PR
- [ ] Gate changes are explicitly authorized by linked issue

<!-- contract:merge_readiness -->
## Merge Readiness

- [ ] CI green
- [ ] Local required gates green
- [ ] Privacy guard green
- [ ] PR budget respected or Scope Exception documented
- [ ] No admin bypass
- [ ] No direct push to main
```

- [ ] **Step 4: Verify anchors**

Run:

```bash
rg -n "<!-- contract:" .github/PULL_REQUEST_TEMPLATE.md .github/ISSUE_TEMPLATE
```

Expected: output includes anchors in every new template.

- [ ] **Step 5: Commit**

Run:

```bash
git add .github/PULL_REQUEST_TEMPLATE.md .github/ISSUE_TEMPLATE
git commit -m "docs: add autonomous delivery GitHub templates"
```

### Task 6: Split Contract Guards Into Focused Python Scripts

**Files:**
- Create: `scripts/ci/check-autonomous-goal.py`
- Create: `scripts/ci/check-loop-state.py`
- Create: `scripts/ci/check-experiment-report.py`
- Create: `scripts/ci/check-pr-budget.py`
- Create: `scripts/ci/check-benchmark-lanes.py`
- Create: `scripts/ci/check-private-evidence-redaction.py`
- Create: `scripts/ci/check-gate-integrity.py`
- Create: `scripts/ci/check-goal-complete.py`
- Modify: `scripts/ci/check-performance-contracts.py`

- [ ] **Step 1: Add shared script pattern**

Each new script must be standard-library only and executable through `python3`. Use this structure:

```python
#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]

def fail(message: str) -> None:
    raise ValueError(message)

def load_toml(path: pathlib.Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)

def main() -> int:
    if not ROOT.exists():
        fail(f"repository root does not exist: {ROOT}")
    print(f"{pathlib.Path(__file__).name} passed")
    return 0

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"{pathlib.Path(__file__).name}: {exc}", file=sys.stderr)
        raise SystemExit(1)
```

- [ ] **Step 2: Implement `check-autonomous-goal.py`**

The script must load `ACTIVE_GOAL.toml` and validate:

```python
goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
auto = goal.get("autonomous_delivery")
if not isinstance(auto, dict):
    fail("ACTIVE_GOAL.toml missing [autonomous_delivery]")

permissions = auto.get("permissions", {})
required_true = [
    "production_code_allowed",
    "private_benchmark_allowed",
    "private_resume_root_read_allowed",
    "seektalent_artifacts_query_read_allowed",
    "github_issue_write_allowed",
    "github_pr_write_allowed",
    "commit_push_allowed",
    "auto_merge_allowed",
]
for key in required_true:
    if permissions.get(key) is not True:
        fail(f"autonomous_delivery.permissions.{key} must be true")

required_false = [
    "direct_main_push_allowed",
    "admin_bypass_allowed",
    "raw_private_data_commit_allowed",
    "raw_query_commit_allowed",
    "gate_bypass_allowed",
    "threshold_relaxation_allowed",
]
for key in required_false:
    if permissions.get(key) is not False:
        fail(f"autonomous_delivery.permissions.{key} must be false")
```

- [ ] **Step 3: Implement `check-benchmark-lanes.py`**

The script must load `perf/acceptance-matrix.toml` and verify required lanes:

```python
matrix = load_toml(ROOT / "perf" / "acceptance-matrix.toml")
lanes = matrix.get("autonomous_delivery_lanes", {})
required = {
    "first_searchable",
    "full_import_ocr_backlog",
    "query_hot_path",
    "agent_query_replay",
    "repeat_amplification_control",
}
missing = required.difference(lanes)
if missing:
    fail(f"missing autonomous_delivery_lanes: {sorted(missing)}")
agent = lanes["agent_query_replay"]
if "D1M_real_distribution_quality" not in agent.get("cannot_claim", []):
    fail("agent_query_replay must not claim D1M real distribution quality")
```

- [ ] **Step 4: Implement `check-pr-budget.py`**

The script must validate `ACTIVE_GOAL.toml` budget and PR template anchors:

```python
goal = load_toml(ROOT / "ACTIVE_GOAL.toml")
budget = goal["autonomous_delivery"]["pr_budget"]
expected = {
    "max_commits": 5,
    "max_changed_files": 15,
    "max_net_lines": 800,
    "max_issue_count": 2,
    "require_single_primary_lane": True,
    "require_single_primary_hypothesis": True,
    "allow_scope_exception_auto_merge": False,
}
for key, expected_value in expected.items():
    if budget.get(key) != expected_value:
        fail(f"pr_budget.{key} must be {expected_value!r}")

template = (ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text()
for anchor in [
    "contract:scope",
    "contract:linked_issue",
    "contract:hypothesis_baseline",
    "contract:verification",
    "contract:performance_evidence",
    "contract:privacy_boundary",
    "contract:rollback_plan",
    "contract:merge_readiness",
]:
    if f"<!-- {anchor} -->" not in template:
        fail(f"PR template missing {anchor}")
```

- [ ] **Step 5: Implement `check-goal-complete.py`**

The script must reject `goal_complete` unless all required cells are present and main-reachable:

```python
import json

state = json.loads((ROOT / "perf" / "current-loop-state.json").read_text())
if state.get("workflow_state") != "goal_complete":
    print("check-goal-complete.py passed: workflow_state is not goal_complete")
    return 0

cells = {cell.get("cell"): cell for cell in state.get("evidence_cells", [])}
required = {
    "D10K_private_real",
    "D100K_synthetic_scale",
    "D1M_synthetic_scale",
    "first_searchable",
    "full_import_ocr_backlog",
    "query_hot_path",
    "agent_query_replay",
    "repeat_amplification_control",
    "soak_fault",
    "gui_manual",
}
missing = required.difference(cells)
if missing:
    fail(f"goal_complete missing evidence cells: {sorted(missing)}")
for name, cell in cells.items():
    if not cell.get("main_reachable_commit"):
        fail(f"{name} missing main_reachable_commit")
```

- [ ] **Step 6: Update aggregate entrypoint**

Modify `scripts/ci/check-performance-contracts.py` to run focused checks after existing fixture checks:

```python
import subprocess

FOCUSED_CHECKS = [
    "scripts/ci/check-autonomous-goal.py",
    "scripts/ci/check-loop-state.py",
    "scripts/ci/check-experiment-report.py",
    "scripts/ci/check-pr-budget.py",
    "scripts/ci/check-benchmark-lanes.py",
    "scripts/ci/check-private-evidence-redaction.py",
    "scripts/ci/check-gate-integrity.py",
    "scripts/ci/check-goal-complete.py",
]

def run_focused_checks() -> None:
    for rel in FOCUSED_CHECKS:
        subprocess.run([sys.executable, str(ROOT / rel)], check=True)
```

Call `run_focused_checks()` from `main()` after current public contract validation passes.

- [ ] **Step 7: Verify Python syntax and aggregate gate**

Run:

```bash
python3 -m py_compile scripts/ci/check-performance-contracts.py scripts/ci/check-autonomous-goal.py scripts/ci/check-loop-state.py scripts/ci/check-experiment-report.py scripts/ci/check-pr-budget.py scripts/ci/check-benchmark-lanes.py scripts/ci/check-private-evidence-redaction.py scripts/ci/check-gate-integrity.py scripts/ci/check-goal-complete.py
python3 scripts/ci/check-performance-contracts.py
```

Expected: syntax compile exits 0; aggregate gate exits 0.

- [ ] **Step 8: Commit**

Run:

```bash
git add scripts/ci/check-*.py
git commit -m "ci: split autonomous delivery contract guards"
```

### Task 7: Add Branch Protection Workflow Checks

**Files:**
- Modify: `.github/workflows/pr.yml`

- [ ] **Step 1: Add focused workflow steps**

Add steps under the existing PR workflow after setup:

```yaml
      - name: contract-check
        run: python3 scripts/ci/check-performance-contracts.py

      - name: privacy-guard
        run: ./scripts/ci/guard-public-repo.sh

      - name: pr-budget
        run: python3 scripts/ci/check-pr-budget.py

      - name: benchmark-lane-claims
        run: python3 scripts/ci/check-benchmark-lanes.py

      - name: gate-integrity
        run: python3 scripts/ci/check-gate-integrity.py

      - name: public-repo-guard
        run: ./scripts/ci/guard-public-repo.sh
```

If the existing workflow already runs the public guard once, keep one named `public-repo-guard` step and remove duplicate unnamed guard invocation.

- [ ] **Step 2: Verify YAML contains required check names**

Run:

```bash
rg -n "contract-check|privacy-guard|pr-budget|benchmark-lane-claims|gate-integrity|public-repo-guard" .github/workflows/pr.yml
```

Expected: all six names are present.

- [ ] **Step 3: Run local guards**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
./scripts/ci/guard-public-repo.sh
```

Expected: both commands exit 0.

- [ ] **Step 4: Commit**

Run:

```bash
git add .github/workflows/pr.yml
git commit -m "ci: require autonomous delivery contract checks"
```

### Task 8: Final Public Verification And Review Handoff

**Files:**
- Modify: `docs/superpowers/plans/2026-06-25-autonomous-delivery-issue-led-slice-train.md` only if task checklist status needs updating during execution.

- [ ] **Step 1: Run full public contract verification**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
python3 -m py_compile scripts/ci/check-*.py
./scripts/ci/guard-public-repo.sh
git diff --check -- ACTIVE_GOAL.toml perf .github scripts/ci 03_next_goal_高性能本地检索GUI闭环 docs/superpowers
```

Expected: all commands exit 0.

- [ ] **Step 2: Confirm no private artifacts entered git**

Run:

```bash
git diff --name-only origin/main...HEAD | sort
git ls-files | rg '(^|/)(local-data|data|resume-data|resumes|corpus|corpora|indexes|logs|diagnostics|bench-results|model-cache|\\.cache)(/|$)|\\.(sqlite|sqlite3|db|log|profraw)$|(^|/)(\\.env(\\..*)?|ipc\\.auth|[^/]*\\.(token|pem|key))$'
```

Expected: first command lists only contract/docs/template/CI files. Second command exits 1 with no output.

- [ ] **Step 3: Prepare PR review summary**

Write this summary into the PR body:

```markdown
## Scope

This PR lands the autonomous delivery contract layer only: target docs, machine-readable goal/evidence contract fields, GitHub issue/PR templates, and public CI guards.

## Out of Scope

No search optimization, daemon implementation, GUI implementation, private benchmark execution, raw query fixture, or private resume data is included.

## Verification

- `python3 scripts/ci/check-performance-contracts.py`
- `python3 -m py_compile scripts/ci/check-*.py`
- `./scripts/ci/guard-public-repo.sh`
- `git diff --check -- ACTIVE_GOAL.toml perf .github scripts/ci 03_next_goal_高性能本地检索GUI闭环 docs/superpowers`

## Privacy Boundary

Only redacted aggregate contracts, schemas, template anchors, and CI guard code are committed. Raw resumes, raw OCR text, raw query text, raw SeekTalent traces, local paths, provider payloads, diagnostics, tokens, and model caches remain local-only.
```

- [ ] **Step 4: Stop before production implementation**

Do not start performance optimization, GUI implementation, daemon implementation, private benchmark execution, or frozen query extraction from this plan. Those begin only after this contract layer is reviewed and merged.
