# PR #10 Documentation Contract Hardening Spec

Status: ready for fw-plan-review
Date: 2026-06-22
Linked plan: `../plans/2026-06-22-performance-goal-doc-contract.md`
Execution scope: documentation contract only

## Background

PR #10 adds the next-goal document package for high-performance local search, GUI, and manual/Codex closed-loop validation. External review correctly identified that the direction is useful but the current package is not yet a reliable long-running Codex execution contract.

The next phase will include broad breaking changes across search performance, daemon IPC, indexing, GUI, benchmarks, and manual validation. Before that work starts, the documentation must prevent goal drift, benchmark overfitting, and accidental relaxation of acceptance criteria.

## Verified Baseline

Current repository facts from the fw-office-hours and fw-ceo-review read-only audit:

- Branch: `codex/high-performance-local-search-gui-docs`.
- PR: `#10`, draft/open, base `main`, title `[codex] add high performance search GUI goal docs`.
- Diff against `origin/main`: 13 documentation files, 880 insertions.
- Worktree: clean at review time; no stash.
- Existing `docs/` contains only reports and runbooks; there is no `docs/superpowers/` directory before this fw-plan stage.
- `GOAL.md` still states the current stage does not require extreme performance, while lines later move full hot-index baseline, 500-query benchmark, P95/P99 reduction, and UI into follow-up goals. That conflicts with the active next-goal package.
- `tokei` baseline excluding `target`, `.git`, `local-data`, and `artifacts`: 89 Rust files, 100,361 Rust lines, 92,571 Rust code lines; 49 Markdown files, 26,493 Markdown lines; 223 total files, 152,198 total lines.

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
- `03_next_goal_高性能本地检索GUI闭环/`
- `docs/superpowers/specs/`
- `docs/superpowers/plans/`

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

## Non-Goals

- This change does not fix the Tantivy OR behavior.
- This change does not implement true incremental indexing.
- This change does not stream encrypted snapshot publishing.
- This change does not create a GUI.
- This change does not run private benchmark data.
- This change does not add new CI scripts or code-based policy checks.

## Required Documentation Contract

### 1. Goal Authority

The planned documentation change must make the relationship among root `GOAL.md`, the existing system design, and the next-goal directory explicit:

- `GOAL.md` remains the product-level goal and current/next-stage index.
- `03_next_goal_高性能本地检索GUI闭环/` becomes the active execution contract for the performance + GUI + closed-loop phase after current-stage closure.
- `docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md` and its linked plan are the planning artifacts for fixing this PR's documentation contract.

The plan must not silently create a new root machine contract file without plan-review approval. It must define the schema and acceptance expectations inside the goal documentation first.

### 2. Reviewer Issue Mapping

The target documentation package must include a reviewer issue ledger with:

- Issue id.
- Reviewer claim.
- Verified truth value.
- Evidence path.
- Required documentation owner.
- Acceptance condition.

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

- Simple whitespace text query means all non-stopword terms are required unless the user explicitly selects an OR mode.
- Quoted phrases remain phrase constraints.
- Field filters are hard filters and run before ranking.
- Boolean syntax must be explicit, documented, and tested against the simple text mode.
- Empty, huge, adversarial, or contradictory query input must return bounded, explainable responses.
- Benchmark tuning must not change this semantic contract.

The target docs must define metamorphic query checks such as:

- Reordered simple terms should preserve the result set within ranking tolerance.
- Adding a required term should not increase the unfiltered candidate set.
- Explicit OR should be the only way to widen simple term matching.
- Field filters should reduce or preserve candidate count, never widen it.

### 5. Daemon IPC and Diagnostics

The target docs must extend the daemon IPC and diagnostics contract before GUI implementation:

- Request envelope with request id, schema version, deadline, client class, idempotency key, cancel token, and optional batch id.
- Search batch contract for GUI and benchmark harness.
- Cancel contract that works for queued and active long-running requests.
- Overload response contract with retry timing and degraded-mode explanation.
- Backpressure and fairness rules separating interactive GUI work, Codex validation work, background import/OCR/vector work, and benchmark work.
- Diagnostics contract that remains redacted and aggregate-only.

### 6. Acceptance Matrix

The target docs must define a W0/W1 acceptance matrix:

- W0: public/synthetic, docs-only or smoke-capable checks that can run in CI or local without private data.
- W1: local private benchmark using local resumes and local query set, with redacted aggregate evidence only.
- Soak: resident daemon long-run, restart, fault injection, queue pressure, cancellation, and recovery.
- GUI/manual: import, status, query, detail, diagnostics, pause/resume/cancel, and failure visibility through versioned daemon contracts.

Acceptance must distinguish:

- Evidence required to merge the documentation PR.
- Evidence required to start implementation.
- Evidence required to mark each implementation slice complete.
- Evidence required to declare the whole performance + GUI goal complete.

### 7. Security and Privacy

The target docs must preserve and sharpen the repository privacy boundary:

- SeekTalent artifacts can only supply query shapes, terms, and query combinations.
- Raw query text must remain local and must not enter git.
- Candidate cards, resume text, names, contact info, paths, screenshots, browser traces, cookies, and tokens must not be copied into this repository.
- Benchmark evidence committed to git must be redacted aggregate evidence with hashes and counts only.
- Diagnostics evidence committed to git must assert that it contains no raw resume text, raw queries, candidate results, paths, or tokens.

## Acceptance Criteria for the Planned Docs Change

The future docs-hardening implementation is acceptable when:

1. `GOAL.md` no longer conflicts with the next active performance + GUI + closed-loop phase.
2. `MANIFEST.md` lists every new or changed target document.
3. The target goal directory has an explicit reviewer issue ledger.
4. The target goal directory has an explicit Loop Engineering state machine.
5. Query semantics, benchmark privacy, and anti-overfit rules are documented in the query benchmark document.
6. Daemon IPC and diagnostics contract includes deadline, cancel, batch, overload, fairness, and redaction fields.
7. Implementation slicing starts with P0 contract/semantics/acceptance and defers code changes until after fw-plan-review.
8. Verification confirms no planned docs-hardening diff touches implementation code paths.
9. `git diff --check` passes for the touched documentation files.
10. `./scripts/ci/guard-public-repo.sh` passes before any public push.

## Handoff

After this spec and linked plan are created, the next workflow stage is `fw-plan-review`. Implementation must not begin until plan review approves the documentation-hardening plan.
