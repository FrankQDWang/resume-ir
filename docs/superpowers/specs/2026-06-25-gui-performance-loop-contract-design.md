# GUI / Performance / Loop Contract Design

Date: 2026-06-25

Status: approved-for-spec-review

Scope: this spec defines how the next contract update should incorporate the GUI stack decision, `UI-reference/` visual baseline, four-layer performance optimization taxonomy, macOS/Windows profiling lanes, and autonomous loop drift controls. It does not authorize production GUI, daemon, benchmark runner, or private benchmark implementation.

## 1. Decisions

The target contract update must make these decisions machine-visible:

1. The default GUI stack is `Tauri + React + Vite + Tailwind + TypeScript`.
2. `UI-reference/` is a visual baseline, not a functional clone requirement.
3. The old egui/eframe versus Slint bakeoff is no longer a required pre-freeze step; it becomes a fallback only after a recorded blocker against the default stack.
4. Performance work is governed by a four-layer taxonomy with a mandatory observation precondition.
5. macOS M4 is the primary discovery lane; low-end Windows over private network access is the required weak-host validation lane for platform-sensitive gates.
6. The autonomous runner may decide whether a private corpus stays on macOS or is privately copied to the Windows weak-host for required gates, but raw private data and raw paths must never enter git, GitHub, public reports, or redacted evidence.
7. Loop Engineering must prevent lower-layer wins from closing higher-layer blockers.

## 2. GUI Contract

The production GUI should use Tauri as the desktop shell and native bridge. React renders the interface, Vite builds the static frontend, and Tailwind carries the visual system. The application must ship as static assets inside Tauri; it must not require a Next.js server at runtime. The existing Next.js reference is useful because it expresses the visual language, not because it defines the production architecture.

`UI-reference/` is the visual reference. The GUI may change screen inventory, information architecture details, and product functions where the daemon contract requires different behavior. It must preserve the visual language:

1. A quiet, local-first workstation UI, not a landing page.
2. Light surfaces, thin borders, compact density, and restrained color.
3. Left rail, top command bar, central workspace, detail side sheet or panel, and bottom or contextual status affordances.
4. Dense list/result treatment with stable row/card dimensions.
5. Lucide-style icon vocabulary, compact buttons, pills, tags, segmented controls, and redacted diagnostics affordances.
6. Primary accent near the existing reference primary color, with black/gray used for hierarchy and action weight.
7. Rounded corners around 8px unless a component has a clear local reason to differ.

The contract should use "pixel-level visual similarity" as the design target, not "identical functional clone." Visual drift must be caught by screenshots, token inventory, and manual/Codex review. Functional divergence is allowed only when it is tied to product requirements, daemon IPC state, diagnostics state, privacy state, or benchmark/manual validation needs.

The fallback rule should be explicit:

1. Tauri/React/Vite/Tailwind is the default lane.
2. A toolkit bakeoff may reopen only if a GitHub issue records a concrete blocker such as WebView2 packaging failure, unacceptable runtime footprint on weak Windows, inability to meet virtual-list interactivity, or inaccessible native integration.
3. Any fallback bakeoff must reuse the same visual reference inventory and representative pages; it cannot invent a different product look.

## 3. Performance Optimization Taxonomy

Every profile issue and performance PR must declare exactly one primary `optimization_layer`. It may declare optional `affected_layers`, but only the primary layer owns the acceptance criteria.

Required profile issue and PR fields:

```text
optimization_layer
affected_layers
baseline_artifact
profiler_summary
stage_histogram
bottleneck_statement
hypothesis
expected_delta
rollback_condition
negative_controls
acceptance_gate
workload_manifest
query_set_source
corpus_scale
hardware_class
warm_or_cold_definition
cache_state
platform_lane
```

### L0 Observation Precondition

L0 is not an optimization layer. It is the mandatory precondition for optimization. A slice may not enter implementation until it has a baseline, profiler summary, stage histogram, workload manifest, falsifiable hypothesis, expected delta, rollback condition, and negative controls.

Workload representativeness belongs in L0. The contract must record which corpus scale, query set source, hardware class, cache state, and warm/cold definition the profile represents. This prevents optimization based on the wrong workload.

### L1 Architecture-Level Optimization

Expected value: system stays recoverable and can expand by 10x scale.

L1 owns daemon lifecycle, IPC, storage/index topology, BM25/Tantivy schema and parameter choices, ANN index choice, first-searchable behavior, crash recovery, search while importing, OCR/semantic backgrounding, and product latency contracts.

L1 must report:

1. First-searchable latency.
2. Time to first result.
3. Time to full index ready.
4. Resume after crash.
5. Search while importing.
6. Incremental searchable lag.

Algorithm and index choice belongs in L1. The contract must not create a separate L5 for algorithm work.

### L2 Parallelism-Level Optimization

Expected value: corpus scale grows by 10x and throughput improves without sacrificing interactive latency.

L2 owns pipeline concurrency, queueing, backpressure, OCR/vector scheduling, content-read concurrency, parser concurrency, writer behavior, batch hydrate/snippet, fairness, cancellation, overload, and admission control.

L2 must report both throughput and latency:

1. Open-loop throughput.
2. Closed-loop user latency.
3. Queue wait histogram.
4. Scheduler fairness.
5. Cancel latency.
6. Peak RSS.
7. I/O saturation.
8. GUI main-thread blocked time when GUI is affected.

The weak-host Windows lane is a required validation lane for L2 acceptance when platform behavior, I/O, filesystem watching, GUI responsiveness, or resource pressure is part of the claim.

### L3 Compile-Level Optimization

Expected value: 0-15% runtime improvement, or measurable binary/startup/resource improvement, depending on the measured bottleneck.

L3 owns release profile tuning, LTO, codegen units, dependency feature pruning, build metadata, binary size, startup/cold-path behavior, symbol/debug split, and reproducible build settings.

L3 must use the same source code with different build configurations for A/B comparison. It must record binary size, build metadata, startup/cold-path impact, benchmark command, and profiler impact. L3 cannot close L1 or L2 blockers.

### L4 Microarchitecture-Level Optimization

Expected value: 0.5-3% improvement on a specific function or local hotspot.

L4 owns allocation reduction, clone removal, hot-loop simplification, local data-structure changes, and small symbol/function-level optimizations.

L4 must bind to a real symbol/function-level hotspot and explain what percentage of the parent stage it consumes. Criterion-style microbenchmarks are useful but insufficient alone; each L4 PR must explain how the microbenchmark maps to the real workload profile.

L4 defaults to no semantic change. It must not change external behavior, ranking semantics, error semantics, data contract, IPC shape, diagnostics shape, or persistence format unless the PR is reclassified to a higher layer.

### Lower-Layer Closure Rule

A lower-layer optimization cannot close a higher-layer blocker:

1. L4 cannot close an L1 blocker.
2. L3 cannot close an L2 starvation, fairness, or queue-pressure blocker.
3. L2 cannot close an L1 crash recovery, first-searchable, daemon lifecycle, IPC, or index topology blocker.

### Not Planned by Default

Direct hand-written SIMD, branch prediction tuning, cache-line alignment, and prefetching are not part of the normal plan. These are expected to be handled by lower-level libraries such as Tantivy, FAISS, ONNX Runtime, Rust standard libraries, or platform runtimes.

A Scope Exception for this class of work must satisfy all conditions:

1. The profile proves the hotspot is in project-owned code, not inside the library.
2. Existing library parameters, index type, and build features have already been tuned.
3. There is an A/B benchmark and correctness oracle.
4. There is a cross-platform fallback.
5. There is a maintenance-cost assessment.

## 4. Platform Profiling Lanes

The platform model has three lanes:

1. `macos_m4_discovery`
2. `windows_weak_host_validation`
3. `cross_os_ci_smoke`

`macos_m4_discovery` is the fast profile and hypothesis lane. It may use Samply, Instruments, structured `tracing` spans, histograms, release benchmarks, and synthetic scale pressure. It can rank hotspots and guide optimization. It cannot by itself close Windows, weak-host, or cross-platform acceptance gates.

`windows_weak_host_validation` is the required representative low-end Windows lane. It may be reached through private network SSH. It may use WPR/WPA/ETW or equivalent Windows performance tooling, PowerShell runners, USN Journal or filesystem watcher verification, WebView2/Tauri packaging smoke, and resource aggregates.

`cross_os_ci_smoke` is GitHub Actions or equivalent CI smoke. It proves basic Windows/macOS build and test viability. It does not replace private weak-host performance evidence.

Platform evidence must include public-safe fields only:

1. Hardware class.
2. OS build class.
3. Power mode class.
4. Runner version.
5. Benchmark runner version.
6. Redacted resource aggregate.
7. Redacted stage histogram.
8. Public-safe symbol summary, report hash, approved opaque manifest ref, or HMAC-SHA256 opaque manifest ref.

Raw private data, raw queries, raw traces, raw local paths, filenames, OCR text, diagnostics packages, tokens, and model caches must remain local-only.

The autonomous runner may decide whether to keep the private corpus on macOS or privately copy it to the Windows weak-host for required gates. Copying is allowed only for private execution, and only if the destination is a private local root with permissions controlled by the operator. Public evidence must still use symbolic source names such as `$RESUME_IR_PRIVATE_RESUME_ROOT`, `$RESUME_IR_QUERY_ARTIFACT_ROOT`, and `$RESUME_IR_LOCAL_EVIDENCE_DIR`.

Windows weak-host validation is required before:

1. L1 architecture acceptance when platform lifecycle, filesystem watching, recovery, or first-searchable behavior is part of the claim.
2. L2 parallelism acceptance.
3. GUI toolkit or packaging acceptance.
4. D100K weak-host acceptance.
5. `cross_os_passed`.
6. `goal_complete`.

If Windows or private network access is unavailable, the runner must enter reconciliation. It may not skip the gate and claim success. It must record distinct evidence paths before entering blocked state.

## 5. Autonomous Loop Integration

The target loop must keep policy truth ahead of conversation truth:

1. `ACTIVE_GOAL.toml`
2. `perf/acceptance-matrix.toml`
3. schemas and guard scripts
4. GitHub issue and PR ledger
5. redacted benchmark report hash or artifact id
6. `perf/current-loop-state.json`
7. conversation context

Conversation context may explain intent but cannot override policy truth or execution truth.

Performance slices should follow:

```text
goal_authorized
-> baseline_captured
-> profile_issue_opened
-> workload_manifest_recorded
-> optimization_layer_selected
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

GUI slices add:

```text
visual_contract_locked
-> reference_inventory_captured
-> design_tokens_locked
-> representative_page_built
-> visual_diff_reviewed
-> gui_manual_flow_verified
-> packaging_verified
```

Every PR must declare:

1. One primary issue.
2. One primary benchmark lane.
3. One primary hypothesis.
4. One `optimization_layer`, or `visual_reference_version` for GUI-only PRs.
5. One platform lane.
6. Allowed paths.
7. Verification commands.
8. Evidence cell.

Forbidden drift:

1. Lower-layer wins closing higher-layer blockers.
2. Mixing benchmark lanes inside one claim.
3. Changing gates or thresholds and then declaring success.
4. Auto-merging Scope Exception PRs.
5. Skipping Windows weak-host gates.
6. Letting GUI feature work drift away from the visual reference.
7. Treating macOS M4 numbers as Windows weak-host conclusions.
8. Writing private roots, Windows private paths, raw corpus material, raw query text, or raw traces into public evidence.

## 6. Blocked Retry Contract

Blocked means the same condition reproduced after real retries with distinct evidence paths. It does not mean the task is hard, slow, expensive, or ambiguous.

The runner may enter `blocked` only after:

1. The same blocker condition repeats.
2. Three effective retries have happened.
3. Each retry has a distinct `evidence_path`.
4. There is no new code, environment, configuration, or evidence path that could change the result.
5. External input or external state change is required.

Windows/private-network unavailability must start as reconciliation, not immediate blocked. Reconciliation should check SSH reachability, private root permissions, runner version, PowerShell/WPR capability, and any changed network or machine state. Only repeated failure with distinct evidence paths may become blocked.

## 7. Goal Completion Rule

`goal_complete` requires all of the following:

1. W0 docs gate accepted.
2. D10K private calibration accepted.
3. D100K weak-host gate accepted.
4. D1M synthetic or derived scale accepted.
5. Repeat amplification control accepted.
6. Query hot path benchmark accepted.
7. First-searchable benchmark accepted.
8. Full import and OCR backlog benchmark accepted.
9. Agent query replay benchmark accepted.
10. Soak/fault accepted.
11. GUI/manual loop accepted.
12. macOS discovery evidence complete where required.
13. Windows weak-host evidence complete where required.
14. Review ledger has no open blocker.
15. Privacy guard passes.
16. All issues close with evidence.
17. All PRs merge and main is synchronized.

## 8. Target Documents to Update Later

The implementation plan should update only contract and guard surfaces for this slice. Expected targets:

1. `03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md`
2. `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`
3. `03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md`
4. `ACTIVE_GOAL.toml`
5. `perf/acceptance-matrix.toml`
6. `perf/loop-state.schema.json`
7. `perf/experiment-report.schema.json`
8. `.github/ISSUE_TEMPLATE/profile_issue.md`
9. Existing contract guard scripts, only as needed to make the new fields machine-checkable.

This spec intentionally does not change production Rust code, GUI code, daemon code, benchmark runners, or private data execution.
