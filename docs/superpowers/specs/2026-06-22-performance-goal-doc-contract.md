# PR #10 Documentation Contract Hardening Spec

Status: request-changes repair implemented; pending fw-review
Date: 2026-06-22
Linked plan: `../plans/2026-06-22-performance-goal-doc-contract.md`
Execution scope: documentation contract only

## Background

PR #10 adds the next-goal document package for high-performance local search, GUI, and manual/Codex closed-loop validation. External review correctly identified that the direction is useful but the current package is not yet a reliable long-running Codex execution contract.

The next phase will include broad breaking changes across search performance, daemon IPC, indexing, GUI, benchmarks, and manual validation. Before that work starts, the documentation must prevent goal drift, benchmark overfitting, and accidental relaxation of acceptance criteria.

## Verified Baseline

Repository facts from the original fw-office-hours and fw-ceo-review read-only audit:

- Branch: `codex/high-performance-local-search-gui-docs`.
- PR: `#10`, draft/open, base `main`, title `[codex] add high performance search GUI goal docs`.
- Initial diff against `origin/main`: 13 documentation files, 880 insertions.
- Worktree: clean at review time; no stash.
- Existing `docs/` contains only reports and runbooks; there is no `docs/superpowers/` directory before this fw-plan stage.
- At the initial audit, `GOAL.md` stated the current stage did not require extreme performance, while lines later moved full hot-index baseline, 500-query benchmark, P95/P99 reduction, and UI into follow-up goals. That was the authority conflict this repair addresses.
- `tokei` baseline excluding `target`, `.git`, `local-data`, and `artifacts`: 89 Rust files, 100,361 Rust lines, 92,571 Rust code lines; 49 Markdown files, 26,493 Markdown lines; 223 total files, 152,198 total lines.

Request-changes repair facts:

- PR #10 remains draft/open while documentation blockers are repaired.
- The repair explicitly adds machine-readable public contract files: `ACTIVE_GOAL.toml`, `perf/acceptance-matrix.toml`, `perf/loop-state.schema.json`, and `perf/experiment-report.schema.json`.
- The repair remains docs/data only. It does not touch `crates/`, `scripts/`, `Cargo.toml`, `Cargo.lock`, tests, private resumes, raw query files, local artifacts, diagnostics packages, or model caches.
- Current `tokei` baseline excluding `target`, `.git`, `local-data`, `artifacts`, and `.worktrees`: 236 total files, 154,006 total lines; 89 Rust files, 100,361 Rust lines; 58 Markdown files, 27,738 Markdown lines.

Technical evidence behind the documentation risk:

- Full-text search currently uses Tantivy `QueryParser::for_index(...).parse_query_lenient(...)`; Tantivy 0.26 defaults whitespace terms to OR unless `set_conjunction_by_default()` is used.
- Incremental full-text snapshot publishing reads active stored documents, merges replacements, and republishes a full snapshot.
- Snapshot encryption currently archives snapshot files into memory before writing the encrypted snapshot.
- Import still crawls the directory and accumulates pending searchable documents before publishing visibility.
- Private query benchmark currently writes each query to a temp file and spawns a command per query.
- Daemon search IPC currently accepts `query`, `mode`, `top_k`, and `filters`; the target docs do not yet specify deadline, cancellation, batch, overload, backpressure, or fairness contracts.

## Problem

The current document package is directionally correct but under-specified in four areas that matter for long-running agentic work:

1. Goal authority is ambiguous.
2. Loop engineering has no formal state machine or machine-checkable drift controls.
3. Performance and query semantics are not frozen tightly enough before optimization begins.
4. Acceptance evidence is not separated into smoke, local private benchmark, soak, fault, and GUI/manual validation lanes.

If this remains unresolved, a future implementation run can pass local tests while still optimizing the wrong semantics, weakening privacy boundaries, chasing a moving target, or claiming performance success from smoke evidence.

## Scope

This spec is for a documentation-hardening change only.

In scope for the planned change:

- `GOAL.md`
- `MANIFEST.md`
- `ACTIVE_GOAL.toml`
- `03_next_goal_高性能本地检索GUI闭环/`
- `docs/superpowers/specs/`
- `docs/superpowers/plans/`
- `perf/`

Out of scope for the planned change:

- Any file under `crates/`
- Any file under `scripts/`
- `Cargo.toml`
- `Cargo.lock`
- Test source files
- Real resumes, local runtime data, raw query text, raw SeekTalent artifacts, tokens, diagnostics packages, model caches, screenshots, browser traces, or candidate result data
- Running the private 10k import benchmark or private query benchmark
- Implementing daemon, indexing, query parser, GUI, benchmark runner, or platform watcher behavior

## Goals

1. Make one active goal authority clear enough that a future Codex run cannot choose a weaker or older goal by accident.
2. Convert the reviewer report into a tracked documentation contract with one owner document for every blocker class.
3. Add a Loop Engineering state machine that defines allowed long-running task states, transition evidence, and drift prevention rules.
4. Freeze query business semantics at the documentation level before performance tuning begins.
5. Freeze daemon IPC and diagnostics boundaries at the documentation level before GUI work begins.
6. Define benchmark and evidence lanes that separate smoke, W0 local proof, W1 private proof, soak, fault, and GUI/manual validation.
7. Keep privacy rules explicit enough that no raw query, resume text, candidate result, path, token, or diagnostic package can enter git.
8. Add machine-readable public contracts for active goal state, acceptance redlines, loop-state reports, and redacted experiment reports.
9. Define profiling, platform journal, and all-computer discovery contracts before performance implementation.

## Non-Goals

- This change does not fix the Tantivy OR behavior.
- This change does not implement true incremental indexing.
- This change does not stream encrypted snapshot publishing.
- This change does not create a GUI.
- This change does not run private benchmark data.
- This change does not add new CI scripts or production code.
- This change does not claim W1, soak/fault, or GUI/manual evidence is complete.

## Required Documentation Contract

### 1. Goal Authority

The planned documentation change must make the relationship among root `GOAL.md`, the existing system design, and the next-goal directory explicit:

- `GOAL.md` remains the product-level goal and current/next-stage index.
- `03_next_goal_高性能本地检索GUI闭环/` becomes the active execution contract for the performance + GUI + closed-loop phase after current-stage closure.
- `docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md` and its linked plan are the planning artifacts for fixing this PR's documentation contract.

The request-changes repair explicitly creates root and `perf/` machine-readable contract files because review found that prose-only contracts were not tight enough. These files are public docs/data artifacts; they do not authorize production code changes or private benchmark execution.

### 2. Reviewer Issue Mapping

The target documentation package must include a reviewer issue ledger with:

- Issue id.
- Reviewer claim.
- Verified truth value.
- Evidence path.
- Required documentation owner.
- Acceptance condition.
- Status.
- Closure evidence.
- Closed-by slice or PR marker.

The ledger must include at least these issue classes:

- Root goal authority conflict.
- Query OR-vs-AND semantics.
- Incremental snapshot not being true incremental.
- Snapshot publish memory scaling.
- First searchable visibility delay.
- Private query benchmark process pollution.
- Daemon IPC missing deadline, cancel, batch, backpressure, fairness, overload.
- Adaptive governor not yet algorithmic.
- Business query semantics not frozen.
- Platform watcher and journal strategy incomplete.
- Stable identity drift risk.
- Tantivy stored body/snippet hot-path risk.
- Large OR doc-id filter risk.
- Encryption ADR missing.
- Resident daemon harness and profiling stack missing.
- GUI toolkit bakeoff missing.
- Acceptance matrix not machine-tight.
- Loop engineering state machine missing.
- Anti-overfit and machine-enforced policy incomplete.
- Implementation ordering needs P0 contract-first reset.
- Workspace `unsafe_code = "forbid"` versus possible platform FFI carve-out.

### 3. Loop Engineering State Machine

The target docs must define a state machine for long-running Codex work:

- `intake`
- `ceo_reviewed`
- `plan_ready`
- `plan_reviewed`
- `slice_active`
- `red_check_written`
- `implementation_active`
- `verification_active`
- `evidence_review`
- `blocked`
- `slice_complete`
- `goal_complete`

It must also define the performance experiment state sequence:

- `not_started`
- `contract_locked`
- `baseline_validated`
- `profile_captured`
- `hotspot_prioritized`
- `optimization_slice_active`
- `regression_checked`
- `w1_accepted`
- `soak_accepted`
- `gui_accepted`
- `blocked`
- `complete`

Every state must define:

- Entry condition.
- Allowed transitions.
- Required evidence.
- Disallowed shortcuts.
- How to detect goal drift.
- How to stop after repeated blocked evidence.

The state machine must explicitly prevent these failure modes:

- Reinterpreting the active goal to match an easier implementation.
- Treating smoke evidence as full benchmark evidence.
- Retrying the same blocked validation loop without new evidence.
- Changing query semantics to improve latency numbers.
- Letting GUI depend on unversioned internal daemon fields.
- Using raw private query text or resume content as committed evidence.

### 4. Query Semantics

The target docs must freeze business-visible query semantics before performance work:

- Simple whitespace text query means all normalized terms are required unless the user explicitly selects an OR mode.
- Stopword, synonym, stemming, typo, and semantic expansion are not part of the default simple text mode unless introduced as a new explicit semantic version.
- Quoted phrases remain phrase constraints.
- Field filters are hard filters and run before ranking.
- Boolean syntax must be explicit, documented, and tested against the simple text mode.
- Empty, huge, adversarial, or contradictory query input must return bounded, explainable responses.
- Benchmark tuning must not change this semantic contract.

The target docs must define metamorphic query checks such as:

- Reordered simple terms must preserve the candidate set exactly; ranking may differ only with documented tie behavior.
- Adding a required term must produce a subset or equal candidate set.
- Explicit OR should be the only way to widen simple term matching.
- Field filters and phrase constraints should reduce or preserve candidate count, never widen it.

### 5. Daemon IPC and Diagnostics

The target docs must extend the daemon IPC and diagnostics contract before GUI implementation:

- Request envelope with request id, schema version, deadline, client capability, idempotency key, cancel token, and optional batch id.
- Daemon-derived internal client class, so a client cannot self-report into a higher-priority fairness lane.
- Local transport and framing rules, including request size limits and no raw query in diagnostics/logs/traces.
- Search batch contract for GUI and benchmark harness.
- Cancel contract that works for queued and active long-running requests.
- Overload response contract with retry timing and degraded-mode explanation.
- Backpressure and weighted fairness rules separating interactive GUI work, Codex validation work, background import/OCR/vector work, and benchmark work.
- Diagnostics contract that remains redacted and aggregate-only.

### 6. Acceptance Matrix

The target docs must define a W0/W1 acceptance matrix:

- W0: public/synthetic, docs-only or smoke-capable checks that can run in CI or local without private data.
- W1: local private benchmark using local resumes and local query set, with redacted aggregate evidence only.
- Soak: resident daemon long-run, restart, fault injection, queue pressure, cancellation, and recovery.
- GUI/manual: import, status, query, detail, diagnostics, pause/resume/cancel, and failure visibility through versioned daemon contracts.
- Machine-readable thresholds in `perf/acceptance-matrix.toml`.

Acceptance must distinguish:

- Evidence required to merge the documentation PR.
- Evidence required to start implementation.
- Evidence required to mark each implementation slice complete.
- Evidence required to declare the whole performance + GUI goal complete.

W1 must include concrete redlines for minimum local document count, searchable count, query count, resident daemon batch execution, no process-spawn-per-query, P95/P99 per query bucket, stage P95, hot-path false flags, zero-change incremental, and privacy booleans.

### 7. Security and Privacy

The target docs must preserve and sharpen the repository privacy boundary:

- SeekTalent artifacts can only supply query shapes, terms, and query combinations.
- Raw query text must remain local and must not enter git.
- Candidate cards, resume text, names, contact info, paths, screenshots, browser traces, cookies, and tokens must not be copied into this repository.
- Benchmark evidence committed to git must be redacted aggregate evidence with hashes and counts only.
- Diagnostics evidence committed to git must assert that it contains no raw resume text, raw queries, candidate results, paths, or tokens.

### 8. Profiling and Platform Discovery

The target docs must define:

- Required Rust instrumentation spans and stage metrics for query parse, prefilter, BM25, ANN, fusion, bulk hydrate, and snippet.
- Histogram, profiler summary, and resource aggregate evidence requirements.
- macOS FSEvents and Windows USN journal contracts, including gap handling, dirty subtree, bounded reconciliation, and fallback manifest diff.
- Full-computer root-set semantics, symlink/reparse handling, cloud placeholder behavior, permission failures, and external volume offline behavior.

### 9. Machine-Readable Contracts

The docs-hardening repair must add:

- `ACTIVE_GOAL.toml`
- `perf/acceptance-matrix.toml`
- `perf/loop-state.schema.json`
- `perf/experiment-report.schema.json`

These files must parse as TOML/JSON and must encode the public privacy boundary.

## Acceptance Criteria for the Planned Docs Change

The future docs-hardening implementation is acceptable when:

1. `GOAL.md` no longer conflicts with the next active performance + GUI + closed-loop phase.
2. `MANIFEST.md` lists every new or changed target document.
3. The target goal directory has an explicit reviewer issue ledger.
4. The target goal directory has an explicit Loop Engineering state machine.
5. Query semantics, benchmark privacy, and anti-overfit rules are documented in the query benchmark document.
6. Daemon IPC and diagnostics contract includes deadline, cancel, batch, overload, fairness, and redaction fields.
7. Implementation slicing starts with P0 contract/semantics/acceptance and defers code changes until after fw-plan-review.
8. Profiling and platform journal contracts exist before performance implementation starts.
9. Machine-readable goal, acceptance, loop-state, and experiment-report contracts exist and parse.
10. Verification confirms no planned docs-hardening diff touches implementation code paths.
11. `git diff --check` passes for the touched documentation files.
12. `./scripts/ci/guard-public-repo.sh` passes before any public push.

## Handoff

After this request-changes repair is committed and pushed, the next workflow stage is `fw-review`. Production performance, daemon, indexing, query parser, GUI, and private benchmark implementation remain out of scope until a later implementation plan is approved.
