# Autonomous Delivery Issue-Led Slice Train Spec

Status: design approved in discussion; pending user review after commit
Date: 2026-06-25
Linked plan: pending after user review
Execution scope: autonomous delivery contract design for the next performance, GUI, and validation phase

## Background

The current high-performance local search and GUI goal package is drift-resistant, but it is not yet a true unattended runner contract. Current documents still contain human-gated transitions, and the active PR contract still forbids production code and private benchmark execution.

The next phase is designed to run from a compact Codex goal prompt until all required work in the goal directory is complete. The user selected C-level full delivery automation: Codex may modify production code, run private benchmarks, create and close GitHub issues, commit, push, open PRs, and merge ready PRs, as long as all permissions, evidence, privacy, branch protection, and benchmark contracts are pre-authorized before the run starts.

The prior long-running goal also showed a delivery failure mode: too much work accumulated in one PR with many commits. This spec prevents that by turning each performance observation into a GitHub issue, each optimization into a small verified slice, and each group of related slices into a reviewable PR.

## Scope

This spec designs the autonomous delivery contract only. It does not implement the runner, benchmark harness, GUI, daemon, search optimizations, GitHub templates, CI guards, or schema changes.

In scope for the eventual implementation plan:

- `ACTIVE_GOAL.toml`
- `perf/acceptance-matrix.toml`
- `perf/current-loop-state.json`
- `perf/loop-state.schema.json`
- `perf/experiment-report.schema.json`
- `03_next_goal_高性能本地检索GUI闭环/`
- `.github/ISSUE_TEMPLATE/`
- `.github/PULL_REQUEST_TEMPLATE.md` and optional pull request template subdirectory
- `.github/workflows/`
- `scripts/ci/`

Out of scope for this spec:

- Running private benchmarks now.
- Reading or committing raw private resumes.
- Creating frozen query fixtures now.
- Opening, updating, closing, or merging GitHub issues and PRs now.
- Changing production Rust, daemon, index, GUI, benchmark, or import behavior now.

## Design Goals

1. Replace mid-run human confirmation with a pre-authorized machine contract.
2. Prevent goal drift, state drift, evidence drift, permission drift, and PR scope drift during long autonomous runs.
3. Require baseline and hypothesis before performance implementation.
4. Use GitHub issues as the profiling ledger and GitHub PRs as bounded delivery bundles.
5. Keep benchmark evidence reproducible, comparable, static where needed, and clearly separated by lane.
6. Allow full private local validation while preventing raw private data, raw query text, trace contents, paths, tokens, logs, diagnostics, OCR text, or resumes from entering git or GitHub prose.
7. Make `goal_complete` machine-computed from accepted evidence, not declared by markdown prose.

## Autonomous Delivery Architecture

The runner starts from a single authorized goal and repeatedly performs one bounded action derived from repository and GitHub state. The continuously injected Codex goal prompt is a guardrail, not the source of execution truth.

The primary delivery state sequence is:

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
-> pr_review_ready
-> pr_merged
-> issue_closed_with_evidence
-> next_issue_or_goal_complete
```

No performance optimization can start before `baseline_captured` and `hypothesis_recorded`. If no baseline exists, the runner may only create instrumentation, benchmark harness, or baseline-capture work.

The runner loop is:

```text
read_goal_contract
-> read_loop_state
-> read_issue_ledger
-> read_pr_state
-> reconcile_state
-> validate_authority
-> validate_integrity
-> select_next_action
-> execute_one_slice
-> verify
-> update_state_and_ledger
-> continue_or_complete
```

Policy truth priority:

```text
ACTIVE_GOAL.toml
> perf/acceptance-matrix.toml
> JSON schemas
> 03_next_goal_高性能本地检索GUI闭环/18_Autonomous_Delivery与Issue_Led_Slice_Train.md
> 03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md
> other active goal documents
> historical documents
```

Execution truth priority:

```text
GitHub PR/issue ledger
> git branch/base sha
> benchmark artifact hashes
> perf/current-loop-state.json
> current conversation context
```

`perf/current-loop-state.json` is a cache. If it conflicts with GitHub, git, or artifact hashes, the runner must reconcile state before implementation.

## Issue-Led Profile Ledger

Every performance claim starts as a GitHub issue. The issue is the durable record for baseline, profile evidence, hypothesis, linked PRs, and closing evidence.

Issue templates must be:

- `.github/ISSUE_TEMPLATE/profile_issue.md`
- `.github/ISSUE_TEMPLATE/gui_manual_loop.md`
- `.github/ISSUE_TEMPLATE/benchmark_infra.md`
- `.github/ISSUE_TEMPLATE/contract_change.md`
- `.github/ISSUE_TEMPLATE/privacy_boundary.md`

`profile_issue.md` covers the full lifecycle: baseline, bottleneck, hypothesis, implementation evidence, and closure. Splitting baseline, bottleneck, and hypothesis into separate issue types would make template choice ambiguous for an autonomous runner.

Every profile issue must include machine-readable anchors or fields for:

```text
Profile Lane
Dataset
Corpus Profile Hash
Query Set Hash
Baseline Command
Baseline Evidence
Profiler Evidence
Hypothesis
Target Metric
Success Threshold
Failure or Regression Guard
Privacy Boundary
Linked PRs
Closing Evidence
```

Rules:

- A baseline issue may exist without a hypothesis.
- An optimization issue must have a baseline and hypothesis before implementation.
- A PR must link at least one primary issue.
- A PR may include supporting issues only when they share the same primary lane and hypothesis.
- Issue closure must include an explicit comment with before/after metrics, percentage change, query set hash, corpus profile hash, command id or script name, and privacy-redaction confirmation.
- A negative experiment is valid evidence. A failed hypothesis may close or update the issue with the data that disproved it.

## PR, Commit, and Merge Contract

Each small slice defaults to one commit. Commit messages must be structured:

```text
<area>: <change summary>

Issue: #123
Hypothesis: H1 hydrate N+1 dominates D10K first-searchable detail latency
Verification: <commands>
Evidence: perf/current-loop-state.json cell=<id>
```

Commits without issue, hypothesis, and evidence are only allowed for small mechanical work such as formatting, renaming, or isolated CI repair.

Default PR budget:

```toml
[autonomous_delivery.pr_budget]
max_commits = 5
max_changed_files = 15
max_net_lines = 800
max_issue_count = 2
require_single_primary_lane = true
require_single_primary_hypothesis = true
allow_scope_exception_auto_merge = false
```

Bundle PR rules:

- The PR has one primary issue.
- Supporting issues must share the same profile lane and primary hypothesis.
- The PR has one clear objective.
- The PR can be verified and rolled back independently.
- The PR does not mix unrelated domains such as GUI layout, OCR scheduling, query parser behavior, and index storage unless explicitly authorized as a Scope Exception.
- The PR does not merge multiple experimental hypotheses.

Scope Exception PRs:

- Must include a `Scope Exception` section.
- Must explain why the change cannot be split.
- Must cover all affected modules with verification commands.
- Must list risks, rollback path, and affected benchmark lanes.
- Must not auto-merge unless `ACTIVE_GOAL.toml` explicitly allows it.

PR lifecycle:

```text
branch_active
-> implementation_active
-> verification_active
-> pr_opened
-> base_synced
-> pr_review_ready
-> ci_green
-> local_gate_green
-> privacy_gate_green
-> merge_method_selected
-> merge_to_main
-> sync_main
-> next_branch
```

Default merge method is squash merge into `main`, while the PR body preserves the slice commit and evidence summary. This keeps `main` history readable without losing PR-level audit detail.

Automatic merge conditions:

- PR template is complete.
- CI is green.
- Required local gates are green.
- Privacy guard is green.
- `./scripts/ci/guard-public-repo.sh` is green.
- Baseline, hypothesis, and evidence references are complete.
- No raw private data is staged or committed.
- No acceptance threshold was lowered.
- No benchmark lanes were mixed.
- No D10K or D100K evidence claims `goal_complete`.
- No unresolved Request Changes review exists.
- No admin bypass is used.
- No direct push to `main` is used.
- No gate, guard, threshold, or workflow was modified to bypass checks.
- Scope Exception PRs do not auto-merge by default.

## Benchmark Lanes

Benchmark evidence is split into five lanes. Lanes cannot be used to claim each other's results.

### First Searchable

Purpose: prove how quickly the user can search directly parseable documents.

Required characteristics:

```text
lane = first_searchable
corpus = D10K_private_real
ocr_policy = disabled_or_background
included_docs = text_pdf + word + directly_parseable_docs
excluded_docs = image_only_pdf_for_hot_path
hot_path_ocr = false
```

It must record TTF100, TTF1000, searchable count, searchable ratio, P50/P95/P99 query latency, image-only PDF discovery count, image-only PDF queued count, and wait reasons. OCR completion time is not a pass condition for this lane.

### Full Import OCR Backlog

Purpose: prove full import, OCR queue behavior, external OCR dependency isolation, recovery, and background scheduling.

Required metrics include OCR queue depth, pages processed, pages per second, timeout count, retry count, failed-retryable count, RSS, CPU, and wall time.

This lane cannot claim query P95 success. External OCR engine latency must be reported separately from resume-ir scheduling and queue behavior.

### Query Hot Path

Purpose: prove hot-index query performance under a resident daemon.

Required characteristics:

```text
resident_daemon = true
spawn_per_query = false
hot_path_ocr = false
hot_path_parse = false
hot_path_heavy_model = false
```

It must record P50/P95/P99, stage latency, zero-result rate, timeout rate, and partial rate. D10K proves real private calibration. D100K and D1M use synthetic or derived corpus for scale pressure and cannot be represented as real distribution quality.

### Agent Query Replay

Purpose: prove resume-ir behavior under static, real SeekTalent Agent query workload.

This lane uses a fixed frozen query set. It is not infinite dynamic sampling and it does not construct queries from job descriptions or prompts.

Allowed source:

```text
source_root = $RESUME_IR_QUERY_ARTIFACT_ROOT
source_glob = **/runtime/trace.log
event_filter = tool_called
tool_filter = source_search
query_source = source_search keyword query segment only
query_extraction_version = trace_source_search_v1
```

$RESUME_IR_QUERY_ARTIFACT_ROOT is a private local environment variable that resolves to the SeekTalent run artifact root. Public contracts and evidence must keep the symbolic variable name and must not write the resolved local path.

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
raw log line outside the source_search keyword query segment
debug blob
screenshot OCR
```

The extractor collects real `source_search` queries, filters a fixed set that is useful on the D10K private corpus, validates candidates against the current local searchable corpus, drops zero-hit queries, and freezes the result by `query_set_sha256`. The frozen benchmark gate must use `--max-zero-result-queries 0`.

Required metrics:

```text
query_set_sha256
query_set_size
query_type_buckets
P50/P95/P99
stage_latency
zero_result_rate
timeout_rate
partial_rate
error_rate
result_count_distribution
duplicate_result_rate
top_k_stability
intersection_clause_count
query_parse_error_rate
```

Space-separated intersection queries must additionally record:

```text
intersection_clause_count
per_clause_candidate_count
intersection_result_count
fallback_used
parse_error
```

Allowed claims:

```text
real_agent_workload_latency
real_agent_query_parser_compatibility
intersection_query_behavior
zero_result_regression
duplicate_result_regression
top_k_stability_under_agent_queries
```

Forbidden claims:

```text
human_search_quality
full_resume_relevance_quality
D1M_real_distribution_quality
OCR_completion_performance
```

If query extraction or redaction rules change, a new `query_set_sha256` is required and old results cannot be compared directly as before/after evidence.

### Repeat Amplification Control

Purpose: detect dedupe, cache, index, and duplicate-result pathologies.

Required characteristics:

```text
lane = repeat_amplification_control
source = D10K_private_real_derived
claim_allowed = pathology_only
```

This lane tests dedupe degradation, cache pollution, index merge or segment slowdown, top-k duplicate flooding, repeated import sensitivity, and query result bias under repeated data. It cannot claim real distribution quality.

## Scale Evidence Strategy

The final scale strategy is:

- D10K uses the real private corpus for quality, distribution, OCR ratio, first-searchable, and query calibration.
- D100K and D1M use synthetic or derived corpora for scale pressure.
- Repeat-amplification control uses derived repetition to test pathology only.

`goal_complete` cannot be claimed from D10K alone, D100K alone, or any unmerged branch evidence.

## Runner Drift Prevention

Before each slice, the runner must check:

```text
goal_id unchanged
ACTIVE_GOAL.toml authority valid
ACTIVE_GOAL.toml hash unchanged or explicitly migrated
ACTIVE_GOAL.toml allowed_paths match slice
primary issue present
primary hypothesis present
baseline hash present
query_set_sha256 unchanged
corpus profile compatible
benchmark runner version compatible
benchmark lane not mixed
acceptance thresholds unchanged
no raw private data staged
branch based on latest main
PR budget not exceeded
gate or guard changes are allowed and isolated
no unmerged evidence-producing branch
```

If gate, guard, workflow, benchmark, threshold, or contract files change, the linked issue must explicitly authorize the change and the PR must be isolated from performance optimization. Gate-changing diffs cannot be bundled with optimization diffs.

Files that trigger gate-change handling include:

```text
.github/workflows/**
scripts/ci/**
scripts/bench/**
perf/contracts/**
ACTIVE_GOAL.toml
benchmark threshold files
privacy guard scripts
```

## Blocked and Retry Policy

Failure is not automatically blocked. The runner first classifies the failure:

```text
test_failure
benchmark_regression
profile_inconclusive
privacy_gate_failure
ci_failure
base_drift
external_dependency_missing
credential_missing
scope_conflict
state_conflict
gate_integrity_failure
benchmark_integrity_failure
```

`base_drift` is a reconciliation action, not a normal retry. The runner syncs or rebases against latest `main`, reruns affected gates, and only then starts retry counting if the same failure remains.

`benchmark_regression` defaults to `experiment_negative`, not blocked.

`privacy_gate_failure` cannot be solved by weakening the privacy gate.

`gate_integrity_failure` cannot be fixed in the same PR as a performance optimization.

Same-condition retry budget:

```toml
[autonomous_delivery.blocked_policy]
max_same_condition_effective_retries = 3
require_new_evidence_path_per_retry = true
```

Each effective retry must have a new evidence path, such as a new profiler, a smaller slice, rollback then retest, base sync then rerun, a different corpus lane, more instrumentation, benchmark harness repair, isolated gate-fix PR, or stricter redaction followed by privacy scan.

Machine state must record:

```json
{
  "blocker_key": "query_hot_path::D10K::privacy_gate_failure::raw_query_staged",
  "same_condition_count": 2,
  "attempts": [
    {
      "attempt_id": "a1",
      "evidence_path": "privacy_scan_v1",
      "action": "unstage raw query fixture and add redacted fixture",
      "result": "failed"
    },
    {
      "attempt_id": "a2",
      "evidence_path": "privacy_scan_v2_with_path_detector",
      "action": "add path redaction and rerun guard",
      "result": "failed"
    }
  ]
}
```

Blocked reports must include:

```text
blocked_reason
same_condition_count
blocker_key
last_three_evidence_refs
what_was_tried
why_more_autonomous_work_is_unsafe
minimum_external_input_needed
suggested_contract_change_if_any
```

Suggested contract changes are recommendations only; the runner cannot apply them without an authorized contract-change path.

## Permissions, Security, and Private Data Boundary

C-level automation is pre-authorized full delivery, not unlimited authority.

`ACTIVE_GOAL.toml` must include:

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
```

Private local input boundaries:

```text
$RESUME_IR_PRIVATE_RESUME_ROOT
```

$RESUME_IR_PRIVATE_RESUME_ROOT is a private local environment variable that resolves to the private resume corpus root. Public contracts and evidence must keep the symbolic variable name and must not write the resolved local path.

This source is allowed only for D10K private real corpus import, OCR backlog, first-searchable, and quality calibration. Raw files, paths, extracted text, OCR output, and candidate information cannot be committed or written into GitHub issue or PR text.

```text
$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log
```

This source is allowed only for extracting static real `source_search` query sets. Raw trace files and non-query trace content cannot be committed or summarized as evidence.

Public commit allowed:

```text
synthetic fixtures
schemas and contracts
counts
hashes
percentiles
stage latency
redacted corpus profile
query_set_sha256
issue and PR references
```

Private local only:

```text
raw resumes
raw OCR text
raw query set
local paths
full trace.log
provider payload
runtime logs
benchmark raw samples
diagnostic packages
tokens
model caches
```

Every public evidence export must be generated by a redaction/export step. Manual copy of private raw output into public evidence is disallowed.

## Machine Contract Landing

The eventual implementation must add `18_Autonomous_Delivery与Issue_Led_Slice_Train.md` as the autonomous delivery entrypoint. It must state:

```text
This document is the autonomous delivery entrypoint.
If it conflicts with older goal documents, ACTIVE_GOAL.toml and this document win over older prose.
```

Existing target documents to update:

- `05_Query_Benchmark与真实Query种子.md`
- `09_安全隐私与本地证据边界.md`
- `10_实施切片与验收门槛.md`
- `13_Loop_Engineering状态机.md`
- `14_W0_W1验收矩阵与证据协议.md`
- `15_性能观测与Profiling工具链.md`
- `17_机器可读Goal与Experiment协议.md`

Machine-readable files to update:

- `ACTIVE_GOAL.toml`
- `perf/acceptance-matrix.toml`
- `perf/loop-state.schema.json`
- `perf/experiment-report.schema.json`
- `perf/current-loop-state.json`

Required TOML sections:

```toml
[autonomous_delivery.permissions]
[autonomous_delivery.pr_budget]
[autonomous_delivery.merge_policy]
[autonomous_delivery.private_sources]
[autonomous_delivery.github_ledger]
[autonomous_delivery.contract_integrity]
[autonomous_delivery.benchmark_lanes]
[autonomous_delivery.blocked_policy]
```

Every evidence cell must carry:

```text
goal_contract_hash
acceptance_matrix_hash
runner_version
benchmark_runner_version
query_set_sha256
corpus_profile_hash
main_reachable_commit
```

## CI and Guard Layer

Keep `scripts/ci/check-performance-contracts.py` as the aggregate entrypoint, but split specific checks into focused scripts:

```text
scripts/ci/check-autonomous-goal.py
scripts/ci/check-loop-state.py
scripts/ci/check-experiment-report.py
scripts/ci/check-pr-budget.py
scripts/ci/check-benchmark-lanes.py
scripts/ci/check-private-evidence-redaction.py
scripts/ci/check-gate-integrity.py
scripts/ci/check-goal-complete.py
```

`check-goal-complete.py` computes completion. Markdown cannot declare completion.

It must check at least:

- D10K real evidence cell accepted.
- D100K synthetic evidence cell accepted.
- D1M synthetic evidence cell accepted.
- Static agent query replay evidence accepted.
- First-searchable evidence accepted.
- Full OCR backlog evidence accepted.
- Repeat-amplification evidence accepted.
- GUI/manual loop evidence accepted.
- All accepted evidence is from `main` HEAD or a merged PR head reachable from `main`.
- No open blocker issue remains.
- No active unmerged PR remains.
- Privacy guard is green.
- Branch protection and CI were not bypassed.

The contract checks must become required status checks in branch protection. Minimum required checks:

```text
contract-check
privacy-guard
pr-budget
benchmark-lane-claims
gate-integrity
public-repo-guard
```

## Template Anchors

PR and issue templates must include machine anchors, not just markdown headings:

```text
<!-- contract:scope -->
## Scope

<!-- contract:linked_issue -->
## Linked Issue

<!-- contract:hypothesis_baseline -->
## Hypothesis / Baseline

<!-- contract:verification -->
## Verification

<!-- contract:performance_evidence -->
## Performance Evidence

<!-- contract:privacy_boundary -->
## Privacy Boundary

<!-- contract:rollback_plan -->
## Rollback Plan

<!-- contract:merge_readiness -->
## Merge Readiness
```

CI can verify that required anchors exist, are non-empty, include linked issue ids, and include evidence references.

PR templates must also include:

```text
## Contract / Gate Changes
- [ ] No gate/guard/threshold changes
- [ ] Gate changes are isolated in this PR
- [ ] Gate changes are explicitly authorized by linked issue
```

## Schema and CI Rejection Cases

Schema and CI must reject:

- Optimization state without baseline.
- Implementation state without hypothesis.
- Missing baseline, query set, or corpus profile hash.
- Experiment report without before/after comparable command.
- Empty privacy boundary.
- Empty redaction statement.
- Benchmark lane claim outside allowed claims.
- D10K or D100K claiming `goal_complete`.
- Agent query replay without frozen `query_set_sha256`.
- Evidence that is not main-reachable.
- PR budget exceeded without Scope Exception.
- Scope Exception auto-merge when not explicitly allowed.
- Gate-changing diff bundled with performance diff.
- Loop state conflicting with GitHub PR/issue or git state without reconciliation.
- Acceptance threshold relaxation.
- Guard or workflow change that bypasses checks.

JSON Schema must validate structure, types, enums, and local constraints. Cross-file checks, GitHub state, branch reachability, and branch protection checks belong in CI scripts.

## Goal Completion Contract

`goal_complete` is valid only when all of these are true:

- D10K private real calibration is accepted.
- D100K synthetic scale is accepted.
- D1M synthetic scale is accepted.
- Repeat-amplification control is accepted.
- Static agent query replay benchmark is accepted.
- First-searchable benchmark is accepted.
- Full OCR backlog evidence is accepted.
- GUI/manual loop evidence is accepted.
- Issue ledger has no open blocker.
- All PRs that produced accepted evidence are merged.
- Accepted evidence is public-safe and redacted.
- Accepted evidence is from `main` HEAD or merged PR heads reachable from `main`.
- Contract, privacy, PR budget, benchmark-lane, gate-integrity, and public-repo guards pass.
- Auto-merge did not bypass branch protection.

## Success Criteria for This Design

This design is successful when a future implementation plan can translate it into:

1. A new target entrypoint document.
2. Updated goal and benchmark documents.
3. Updated machine-readable goal, loop, and experiment contracts.
4. GitHub issue and PR templates with machine anchors.
5. Focused CI guard scripts.
6. Branch-protection-required contract checks.
7. A runner loop that can proceed without mid-run human confirmation while still stopping on unsafe, contradictory, or non-comparable states.
